//! Pictures in the thread pane: asked for once each, decoded off the draw path, sized in cells
//! from the terminal's font, and drawn over rows the pane reserves for them.
use image::DynamicImage;
use ratatui::layout::Size;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use std::collections::HashMap;
use std::fmt;

/// Widest a thumbnail gets, in cells; its height follows the picture, up to [`MAX_ROWS`].
pub const MAX_COLS: u16 = 60;
pub const MAX_ROWS: u16 = 14;
const MAX_PIXELS: u32 = 4096;

pub enum Thumb {
    Loading,
    /// The picture ready to draw, with its size in pixels to reserve rows for it.
    Ready(Box<StatefulProtocol>, (u32, u32)),
    Failed,
}

/// Whether the terminal can draw pictures, and each picture's state, by the link its note gave.
pub struct Thumbs {
    picker: Option<Picker>,
    slots: HashMap<String, Thumb>,
}

impl fmt::Debug for Thumbs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Thumbs({} enabled, {} pictures)", self.enabled(), self.slots.len())
    }
}

impl Default for Thumbs {
    fn default() -> Self {
        Self::off()
    }
}

impl Thumbs {
    pub fn off() -> Self {
        Self { picker: None, slots: HashMap::new() }
    }

    pub fn with(picker: Picker) -> Self {
        Self { picker: Some(picker), slots: HashMap::new() }
    }

    pub fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    /// The cells a ready picture takes within `max_cols`, never scaled past its own pixels.
    pub fn cells(&self, url: &str, max_cols: u16) -> Option<Size> {
        let picker = self.picker.as_ref()?;
        let Some(Thumb::Ready(_, (w, h))) = self.slots.get(url) else { return None };
        let font = picker.font_size();
        let (fw, fh) = (f64::from(font.width.max(1)), f64::from(font.height.max(1)));
        let (w, h) = (f64::from((*w).max(1)), f64::from((*h).max(1)));
        let max_cols = f64::from(max_cols.clamp(1, MAX_COLS));
        let scale = (max_cols * fw / w).min(f64::from(MAX_ROWS) * fh / h).min(1.0);
        let cols = (w * scale / fw).ceil().max(1.0) as u16;
        let rows = (h * scale / fh).ceil().max(1.0) as u16;
        Some(Size::new(cols, rows))
    }

    /// Links not asked for yet, marked loading so each is fetched once.
    pub fn wanted(&mut self, urls: impl IntoIterator<Item = String>) -> Vec<String> {
        if !self.enabled() {
            return vec![];
        }
        urls.into_iter()
            .filter(|url| {
                let new = !self.slots.contains_key(url);
                if new {
                    self.slots.insert(url.clone(), Thumb::Loading);
                }
                new
            })
            .collect()
    }

    pub fn arrived(&mut self, url: &str, image: Option<DynamicImage>) {
        let thumb = match (image, &self.picker) {
            (Some(image), Some(picker)) => {
                let size = (image.width(), image.height());
                Thumb::Ready(Box::new(picker.new_resize_protocol(image)), size)
            }
            _ => Thumb::Failed,
        };
        self.slots.insert(url.to_owned(), thumb);
    }

    pub fn get(&self, url: &str) -> Option<&Thumb> {
        self.slots.get(url)
    }

    pub fn get_mut(&mut self, url: &str) -> Option<&mut Thumb> {
        self.slots.get_mut(url)
    }
}

/// Asks the terminal what it can draw. Pixel protocols only: a half-block mosaic reads worse
/// than the `[image: …]` line, so those terminals get text.
pub fn ask_terminal() -> Option<Picker> {
    Picker::from_query_stdio().ok().filter(|picker| picker.protocol_type() != ProtocolType::Halfblocks)
}

/// Decodes with hard limits, so a hostile upload cannot balloon into gigabytes of pixels.
pub fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PIXELS);
    limits.max_image_height = Some(MAX_PIXELS);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    reader.decode().ok()
}

#[cfg(test)]
pub mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        DynamicImage::new_rgb8(w, h).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A half-block picker with a 10x20 font: it draws into a plain buffer, so tests can see it.
    pub fn test_thumbs() -> Thumbs {
        Thumbs::with(Picker::halfblocks())
    }

    #[test]
    fn each_picture_is_asked_once_and_settles_ready_or_failed() {
        let mut thumbs = test_thumbs();
        let urls = || vec!["https://x/a.png".to_owned(), "https://x/b.png".to_owned()];
        assert_eq!(thumbs.wanted(urls()), urls());
        assert!(thumbs.wanted(urls()).is_empty());
        thumbs.arrived("https://x/a.png", decode(&png(8, 8)));
        thumbs.arrived("https://x/b.png", None);
        assert!(matches!(thumbs.get("https://x/a.png"), Some(Thumb::Ready(_, (8, 8)))));
        assert!(matches!(thumbs.get("https://x/b.png"), Some(Thumb::Failed)));
        assert!(Thumbs::off().wanted(urls()).is_empty(), "a terminal without pictures asks for none");
    }

    #[test]
    fn cells_follow_the_aspect_and_never_upscale() {
        let mut thumbs = test_thumbs();
        let arrive = |thumbs: &mut Thumbs, url: &str, w, h| thumbs.arrived(url, Some(DynamicImage::new_rgb8(w, h)));
        arrive(&mut thumbs, "wide", 720, 480);
        arrive(&mut thumbs, "tiny", 16, 16);
        assert_eq!(thumbs.cells("wide", 80), Some(Size::new(42, 14)));
        assert_eq!(thumbs.cells("wide", 20), Some(Size::new(20, 7)));
        assert_eq!(thumbs.cells("tiny", 80), Some(Size::new(2, 1)));
        assert_eq!(thumbs.cells("missing", 80), None);
    }

    #[test]
    fn decode_refuses_garbage_and_oversized_pictures() {
        assert!(decode(b"not a picture").is_none());
        assert!(decode(&png(4, 4)).is_some());
        assert!(decode(&png(MAX_PIXELS + 1, 1)).is_none());
    }
}
