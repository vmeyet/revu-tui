//! GitLab reads a draft from its title: a `Draft:`, `[Draft]` or `(Draft)` prefix makes one, taking it off makes it ready.
use crate::forge::{Readiness, Title};

const DRAFT_MARKERS: [&str; 3] = ["draft:", "[draft]", "(draft)"];

/// `title` without the draft marker GitLab puts in front of it.
pub(crate) fn bare_title(title: &str) -> &str {
    let title = title.trim_start();
    let marked = DRAFT_MARKERS.iter().find(|marker| title.get(..marker.len()).is_some_and(|head| head.eq_ignore_ascii_case(marker)));
    marked.map_or(title, |marker| title[marker.len()..].trim_start())
}

/// The title GitLab is sent so the MR keeps `readiness`: a new title never turns a draft ready, nor the other way.
pub(super) fn keeping(title: &Title, readiness: Readiness) -> String {
    let bare = bare_title(title.as_str());
    match readiness {
        Readiness::Draft => format!("Draft: {bare}"),
        Readiness::Ready => bare.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_draft_marker_comes_off_whatever_its_case() {
        for marked in ["Draft: Count the widgets", "[draft] Count the widgets", "(DRAFT)Count the widgets", "Count the widgets"] {
            assert_eq!(bare_title(marked), "Count the widgets", "{marked}");
        }
        assert_eq!(bare_title("Drafting the widgets"), "Drafting the widgets");
    }

    #[test]
    fn a_new_title_keeps_the_readiness_it_is_sent_with() {
        let title = |text| Title::parse(text).unwrap();
        assert_eq!(keeping(&title("Count the widgets"), Readiness::Draft), "Draft: Count the widgets");
        assert_eq!(keeping(&title("[Draft] Count the widgets"), Readiness::Draft), "Draft: Count the widgets");
        assert_eq!(keeping(&title("Draft: Count the widgets"), Readiness::Ready), "Count the widgets");
    }
}
