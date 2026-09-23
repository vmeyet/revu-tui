//! Pictures in a note: markdown `![alt](url)` and the `<img src>` GitHub writes, found line by line
//! so the pane can put each one under the text it came from.

/// One picture a note points at, `url` exactly as written (relative GitLab uploads included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub alt: String,
    pub url: String,
}

/// The line with its pictures taken out, and the pictures in order.
pub fn split(line: &str) -> (String, Vec<Image>) {
    let mut text = String::new();
    let mut images = vec![];
    let mut rest = line;
    while let Some((before, image, after)) = next_image(rest) {
        text.push_str(before);
        images.push(image);
        rest = after;
    }
    text.push_str(rest);
    (text, images)
}

/// Every picture of a body, skipping fenced code where `![x](y)` is only text.
pub fn images_in(body: &str) -> Vec<Image> {
    let mut fenced = false;
    let mut out = vec![];
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            out.extend(split(line).1);
        }
    }
    out
}

/// The first picture in `text`: what comes before it, the picture, what comes after.
fn next_image(text: &str) -> Option<(&str, Image, &str)> {
    let markdown = text.find("![").and_then(|at| markdown_at(text, at));
    let html = find_ascii_ci(text, "<img").and_then(|at| html_at(text, at));
    let (start, image, end) = match (markdown, html) {
        (Some(m), Some(h)) => {
            if m.0 <= h.0 {
                m
            } else {
                h
            }
        }
        (Some(found), None) | (None, Some(found)) => found,
        (None, None) => return None,
    };
    Some((&text[..start], image, &text[end..]))
}

/// `![alt](url "title")` starting at `at`; the title is dropped.
fn markdown_at(text: &str, at: usize) -> Option<(usize, Image, usize)> {
    let rest = &text[at + 2..];
    let close = rest.find("](")?;
    let alt = &rest[..close];
    let target = &rest[close + 2..];
    let end = target.find(')')?;
    let url = target[..end].split_whitespace().next()?.trim_matches(['<', '>']);
    let consumed = at + 2 + close + 2 + end + 1;
    Some((at, Image { alt: alt.to_owned(), url: url.to_owned() }, consumed))
}

/// `<img … src="url" alt="alt" …>` starting at `at`, attributes in any order and quote style.
fn html_at(text: &str, at: usize) -> Option<(usize, Image, usize)> {
    let end = at + text[at..].find('>')? + 1;
    let tag = &text[at..end];
    let url = attribute(tag, "src")?;
    Some((at, Image { alt: attribute(tag, "alt").unwrap_or_default(), url }, end))
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        let before_ok = lower[..at].ends_with(char::is_whitespace);
        let after = lower[at + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let value = tag[tag.len() - after.len() + 1..].trim_start();
            let quote = value.chars().next()?;
            return if quote == '"' || quote == '\'' {
                value[1..].find(quote).map(|close| value[1..=close].to_owned())
            } else {
                Some(value.split(|c: char| c.is_whitespace() || c == '>').next()?.trim_end_matches('/').to_owned())
            };
        }
        from = at + name.len();
    }
    None
}

fn find_ascii_ci(text: &str, needle: &str) -> Option<usize> {
    text.to_ascii_lowercase().find(needle)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn image(alt: &str, url: &str) -> Image {
        Image { alt: alt.into(), url: url.into() }
    }

    #[test]
    fn markdown_pictures_leave_their_text_behind() {
        let (text, images) = split("see ![the chart](/uploads/abc/chart.png \"title\") above");
        assert_eq!(text, "see  above");
        assert_eq!(images, [image("the chart", "/uploads/abc/chart.png")]);
        assert_eq!(split("![](https://x/a.png)![b](https://x/b.png)").1, [image("", "https://x/a.png"), image("b", "https://x/b.png")]);
        assert_eq!(split("a [link](https://x) is not a picture").1, []);
        assert_eq!(split("![broken](no close").1, []);
    }

    #[test]
    fn html_pictures_read_src_and_alt_in_any_order() {
        let line = r#"<img width="300" alt="Screenshot 2026" src="https://github.com/user-attachments/assets/1f2e" />"#;
        let (text, images) = split(line);
        assert_eq!(text, "");
        assert_eq!(images, [image("Screenshot 2026", "https://github.com/user-attachments/assets/1f2e")]);
        assert_eq!(split("<IMG SRC='https://x/a.png'>").1, [image("", "https://x/a.png")]);
        assert_eq!(split("<img data-src=\"no\" src=https://x/b.png>").1, [image("", "https://x/b.png")]);
        assert_eq!(split("<img alt=\"none\">").1, []);
    }

    #[test]
    fn pictures_in_code_fences_are_only_text() {
        let body = "look:\n![a](https://x/a.png)\n```md\n![b](https://x/b.png)\n```\n<img src=\"https://x/c.png\">";
        assert_eq!(images_in(body), [image("a", "https://x/a.png"), image("", "https://x/c.png")]);
    }
}
