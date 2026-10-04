// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a message is, for whatever tells of it (see `messenger_core::Body`).
//! One reading of an envelope for the push on a phone, the card in the
//! app and a computer's notification, direct message or group alike.

use crate::view::preview;
use messenger_core::envelope::{T_MEDIA, T_TEXT};
use messenger_core::{Body, Envelope, LinkKind};
use messenger_links::{ContactLink, LinkType, Uri};

/// The kinds of media the app sends; any other is told as a file.
const MEDIA_KINDS: [&str; 6] = ["image", "video", "audio", "file", "voice", "circle"];
const MAX_BATCH: usize = 64;

/// A message of the chat as a body. None: nothing to show (an empty text,
/// a signal, an edit), or a type that is not a message of a chat.
///
/// A type from a newer app is told by its text when it has one.
pub fn body_of(envelope: &Envelope) -> Option<Body> {
    match envelope.t.as_str() {
        T_TEXT => text(envelope.as_text().unwrap_or_default()),
        T_MEDIA => Some(media(envelope)),
        _ => text(envelope.str_field("text").unwrap_or_default()),
    }
}

fn text(text: &str) -> Option<Body> {
    if let Some(link) = link(text.trim()) {
        return Some(link);
    }
    let text = preview(text);
    (!text.is_empty()).then_some(Body::Text { text })
}

/// A text that is one link and nothing else, as what the link names. A
/// link of the app is long and says nothing to a reader; an address on
/// the web is shown without its scheme.
fn link(text: &str) -> Option<Body> {
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    if let Ok(uri) = Uri::parse(text) {
        return match uri.link_type() {
            LinkType::Group => {
                let name = uri.text("n").ok().flatten().unwrap_or_default();
                Some(Body::Link { link: LinkKind::Group, title: preview(&clean(&name)) })
            }
            LinkType::Contact => {
                let name = ContactLink::from_uri(&uri).map(|c| c.name).unwrap_or_default();
                Some(Body::Link { link: LinkKind::Contact, title: preview(&name) })
            }
            // A bridge has no name to show; the text of the link stands.
            LinkType::Bridge | LinkType::Unknown(_) => None,
        };
    }
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest).trim_end_matches('/');
    (!rest.is_empty()).then(|| Body::Link { link: LinkKind::Web, title: preview(rest) })
}

/// What anybody may put into a link is no word to show as it is.
fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).collect()
}

/// A file somebody sent is told by its kind and name, even when its
/// descriptor is one the app would refuse to download.
fn media(envelope: &Envelope) -> Body {
    let kind = envelope.str_field("kind").filter(|k| MEDIA_KINDS.contains(k)).unwrap_or("file");
    let name = preview(envelope.str_field("name").unwrap_or_default());
    let caption = envelope.str_field("caption").map(preview).filter(|c| !c.is_empty());
    let duration_ms = envelope.fields.get("duration_ms").and_then(|d| d.as_u64()).filter(|d| *d > 0);
    let batch = envelope
        .str_field("batch")
        .filter(|b| !b.is_empty() && b.len() <= MAX_BATCH)
        .map(String::from);
    Body::Media { kind: kind.to_string(), name, caption, duration_ms, batch }
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_links::UriBuilder;

    fn file(fields: serde_json::Value) -> Envelope {
        let mut e = Envelope::new(T_MEDIA);
        e.fields = fields.as_object().unwrap().clone();
        e
    }

    #[test]
    fn a_text_is_its_one_line() {
        assert_eq!(body_of(&Envelope::text(" привет\n  мир ")), Some(Body::Text { text: "привет мир".into() }));
        assert_eq!(body_of(&Envelope::text("   ")), None, "nothing to show");
        let long = "а".repeat(500);
        let Some(Body::Text { text }) = body_of(&Envelope::text(&long)) else { panic!() };
        assert_eq!(text.chars().count(), 120);
    }

    #[test]
    fn a_link_alone_is_told_by_what_it_names() {
        let web = |t: &str| body_of(&Envelope::text(t));
        assert_eq!(
            web("https://www.youtube.com/watch?v=abc"),
            Some(Body::Link { link: LinkKind::Web, title: "youtube.com/watch?v=abc".into() })
        );
        assert_eq!(web(" http://example.org/ "), Some(Body::Link { link: LinkKind::Web, title: "example.org".into() }));
        // A link among words is part of a text.
        assert_eq!(web("смотри https://example.org"), Some(Body::Text { text: "смотри https://example.org".into() }));
        assert_eq!(web("https://"), Some(Body::Text { text: "https://".into() }));

        let group = UriBuilder::new(&LinkType::Group, &"ab".repeat(32)).param("t", "public").param("n", "Клуб & друзья").build();
        assert_eq!(body_of(&Envelope::text(&group)), Some(Body::Link { link: LinkKind::Group, title: "Клуб & друзья".into() }));
        let nameless = UriBuilder::new(&LinkType::Group, &"ab".repeat(32)).param("t", "private").build();
        assert_eq!(body_of(&Envelope::text(&nameless)), Some(Body::Link { link: LinkKind::Group, title: String::new() }));

        let npub = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";
        let contact = UriBuilder::new(&LinkType::Contact, npub).param("n", "Анна").build();
        assert_eq!(body_of(&Envelope::text(&contact)), Some(Body::Link { link: LinkKind::Contact, title: "Анна".into() }));

        // A link of a kind this app does not know is shown as it came.
        assert_eq!(body_of(&Envelope::text("veydan://poll/abc")), Some(Body::Text { text: "veydan://poll/abc".into() }));
    }

    #[test]
    fn a_file_is_told_by_its_kind_and_what_it_carries() {
        let photo = file(serde_json::json!({ "kind": "image", "name": "IMG_1.jpg", "caption": null, "batch": "b1234567" }));
        assert_eq!(
            body_of(&photo),
            Some(Body::Media { kind: "image".into(), name: "IMG_1.jpg".into(), caption: None, duration_ms: None, batch: Some("b1234567".into()) })
        );
        let voice = file(serde_json::json!({ "kind": "voice", "name": "voice.weba", "duration_ms": 12_400 }));
        assert_eq!(
            body_of(&voice),
            Some(Body::Media { kind: "voice".into(), name: "voice.weba".into(), caption: None, duration_ms: Some(12_400), batch: None })
        );
        let doc = file(serde_json::json!({ "kind": "file", "name": "отчёт.pdf", "caption": "  к пятнице " }));
        assert!(matches!(body_of(&doc), Some(Body::Media { caption: Some(c), .. }) if c == "к пятнице"));
        let odd = file(serde_json::json!({ "kind": "hologram", "name": "x" }));
        assert!(matches!(body_of(&odd), Some(Body::Media { kind, .. }) if kind == "file"));
    }
}
