// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What a message is, for whatever tells of it (see `messenger_core::Body`).
//! One reading of an envelope for the push on a phone, the card in the
//! app and a computer's notification, direct message or group alike.

use crate::view::{md, preview};
use messenger_core::envelope::{T_CONTACT, T_MEDIA, T_TEXT};
use messenger_core::{Body, Envelope, LinkKind};
use messenger_links::{ContactLink, LinkType, ServerLink, Uri};
use messenger_store::messages::{self as repo, MessageRow};

/// The kinds of media the app sends; any other is told as a file.
const MEDIA_KINDS: [&str; 6] = ["image", "video", "audio", "file", "voice", "circle"];
const MAX_BATCH: usize = 64;

/// A message of the chat as a body. None: nothing to show (an empty text,
/// a signal, an edit), or a type that is not a message of a chat.
///
/// A type from a newer app is told by its text when it has one.
pub fn body_of(envelope: &Envelope) -> Option<Body> {
    let body = body_of_type(envelope)?;
    Some(if envelope.fwd().is_some() { forwarded(body) } else { body })
}

/// What a forward carries, said to be forwarded.
fn forwarded(body: Body) -> Body {
    Body::Forwarded { body: Box::new(body) }
}

fn body_of_type(envelope: &Envelope) -> Option<Body> {
    match envelope.t.as_str() {
        T_TEXT => text(&words(envelope, envelope.as_text())),
        T_MEDIA => Some(media(envelope)),
        T_CONTACT => Some(contact(envelope)),
        _ => text(&words(envelope, envelope.str_field("text"))),
    }
}

/// The words of a text or a caption as they read: a Markdown one without
/// its marks.
fn words(envelope: &Envelope, text: Option<&str>) -> String {
    let text = text.unwrap_or_default();
    let plain = if envelope.format().is_markdown() { md::plain(text) } else { String::new() };
    if plain.is_empty() { text.to_string() } else { plain }
}

/// A stored message as a body, as `body_of` tells its envelope: what the
/// list of chats says of its newest message (a photo is a photo, not the
/// name of its file). A Markdown text or caption by its words. None for
/// what is no message of a chat or has nothing to say (a card of the bot:
/// the list words it itself).
pub fn body_of_row(row: &MessageRow) -> Option<Body> {
    let body = body_of_row_type(row)?;
    Some(if crate::view::forwarded(&row.content_type, &row.envelope_json).is_some() { forwarded(body) } else { body })
}

fn body_of_row_type(row: &MessageRow) -> Option<Body> {
    let words = row.plain.as_deref().or(row.text.as_deref());
    match row.content_type.as_str() {
        repo::CT_TEXT => text(words.unwrap_or_default()),
        repo::CT_MEDIA => {
            let m: serde_json::Value = row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            let field = |k: &str| m.get(k).and_then(|v| v.as_str());
            let kind = field("kind").filter(|k| MEDIA_KINDS.contains(k)).unwrap_or("file");
            Some(Body::Media {
                kind: kind.to_string(),
                name: preview(field("name").unwrap_or_default()),
                caption: words.map(|w| preview(&redact_secrets(w))).filter(|c| !c.is_empty()),
                duration_ms: m.get("duration_ms").and_then(|d| d.as_u64()).filter(|d| *d > 0),
                batch: field("batch").filter(|b| !b.is_empty() && b.len() <= MAX_BATCH).map(String::from),
            })
        }
        repo::CT_CONTACT => {
            let card: serde_json::Value = row.media_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            Some(Body::Link { link: LinkKind::Contact, title: preview(&messenger_contacts::card::name_of(&card)) })
        }
        _ => None,
    }
}

fn text(text: &str) -> Option<Body> {
    if let Some(link) = link(text.trim()) {
        return Some(link);
    }
    let text = preview(&redact_secrets(text));
    (!text.is_empty()).then_some(Body::Text { text })
}

/// The parameters of a link of the app that let somebody in and are no
/// word to show: the app key of a server card (`k`) and the card of a
/// server riding on a link of a person or a group (`srv`, which carries
/// the key in it).
const SECRET_PARAMS: [&str; 2] = ["k", "srv"];

/// A text as a line of the list of chats or a notification may show it:
/// in every `veydan://` link in it, the value of `k=` and of `srv=` is
/// `…`. The chat shows the message as it came; a line seen on a locked
/// screen or over a shoulder does not show the key of a server.
pub fn redact_secrets(text: &str) -> String {
    // ASCII lowercase keeps every byte where it was.
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(messenger_links::uri::SCHEME) {
        let start = at + found;
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | ')' | ']'))
            .map_or(text.len(), |n| start + n);
        out.push_str(&text[at..start]);
        out.push_str(&redact_link(&text[start..end]));
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

fn redact_link(link: &str) -> String {
    let Some((head, rest)) = link.split_once('?') else { return link.to_string() };
    let (query, fragment) = match rest.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (rest, None),
    };
    let pairs: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) if !v.is_empty() && SECRET_PARAMS.iter().any(|s| s.eq_ignore_ascii_case(k)) => format!("{k}=…"),
            _ => pair.to_string(),
        })
        .collect();
    let mut out = format!("{head}?{}", pairs.join("&"));
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
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
            // The card of a server is told by its name and the host of its
            // relay, never as written: the text carries its app key.
            LinkType::Server => {
                let card = ServerLink::from_uri(&uri).ok()?;
                let host = card.relay.as_str().split_once("://").map_or(card.relay.as_str(), |(_, h)| h).to_string();
                let name = clean(&card.name);
                let text = if name.is_empty() { host } else { format!("{name} ({host})") };
                Some(Body::Text { text: preview(&text) })
            }
            // A bridge, a call node, a request of recovery or a sign-in has no name to
            // show; the text of the link stands.
            LinkType::Bridge | LinkType::CallNode | LinkType::Recover | LinkType::Web | LinkType::Channel | LinkType::Unknown(_) => None,
        };
    }
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest).trim_end_matches('/');
    (!rest.is_empty()).then(|| Body::Link { link: LinkKind::Web, title: preview(rest) })
}

/// Whether a text has anything in it a reader could follow: an address
/// with a scheme, a `www.` host, a key or a profile of the network, or the
/// `nostr:` prefix of either, in any case. The gate lets a stranger send
/// none of these before approval, so that a request is never a lure. A
/// bare domain without a scheme is left alone: the app shows the text of
/// a request plain, so nothing of it is clickable anyway.
pub(crate) fn has_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["://", "www.", "npub1", "nprofile1", "nostr:"].iter().any(|mark| lower.contains(mark))
}

/// What anybody may put into a link is no word to show as it is.
fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).collect()
}

/// A contact card is told as a link to a person: by the name it shows.
fn contact(envelope: &Envelope) -> Body {
    let name = envelope.fields.get("card").map(messenger_contacts::card::name_of).unwrap_or_default();
    Body::Link { link: LinkKind::Contact, title: preview(&name) }
}

/// A file somebody sent is told by its kind and name, even when its
/// descriptor is one the app would refuse to download.
fn media(envelope: &Envelope) -> Body {
    let kind = envelope.str_field("kind").filter(|k| MEDIA_KINDS.contains(k)).unwrap_or("file");
    let name = preview(envelope.str_field("name").unwrap_or_default());
    let caption = envelope.str_field("caption").map(|c| preview(&redact_secrets(&words(envelope, Some(c))))).filter(|c| !c.is_empty());
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

    fn row(content_type: &str, text: Option<&str>, plain: Option<&str>, media: Option<serde_json::Value>) -> MessageRow {
        MessageRow {
            id: "m".into(),
            chat_id: "dm:x".into(),
            wire_id: None,
            direction: "in".into(),
            status: "received".into(),
            content_type: content_type.into(),
            text: text.map(String::from),
            envelope_json: "{}".into(),
            sender_pubkey: "ab".into(),
            reply_to_id: None,
            target_id: None,
            created_at: 1,
            received_at: 1,
            edited_at: None,
            deleted_at: None,
            is_hidden: false,
            outbox_local_id: None,
            failure_reason: None,
            media_json: media.map(|m| m.to_string()),
            mod_state: String::new(),
            review: "ok".into(),
            fmt: String::new(),
            plain: plain.map(String::from),
        }
    }

    #[test]
    fn a_stored_message_is_told_as_its_envelope_is() {
        let voice = row("media", None, None, Some(serde_json::json!({ "kind": "voice", "name": "voice.weba", "duration_ms": 12_400 })));
        assert_eq!(body_of_row(&voice), Some(Body::Media { kind: "voice".into(), name: "voice.weba".into(), caption: None, duration_ms: Some(12_400), batch: None }));
        let odd = row("media", Some("hi"), None, Some(serde_json::json!({ "kind": "exe", "name": "a.exe", "batch": "b1" })));
        assert_eq!(body_of_row(&odd), Some(Body::Media { kind: "file".into(), name: "a.exe".into(), caption: Some("hi".into()), duration_ms: None, batch: Some("b1".into()) }));
        assert_eq!(body_of_row(&row("text", Some("**hi**"), Some("hi"), None)), Some(Body::Text { text: "hi".into() }), "Markdown by its words");
        let card = row("contact", None, None, Some(serde_json::json!({ "display_name": "Анна", "pubkey": "ab" })));
        assert_eq!(body_of_row(&card), Some(Body::Link { link: LinkKind::Contact, title: "Анна".into() }));
        assert_eq!(body_of_row(&row("bot_card", None, None, None)), None);
    }

    #[test]
    fn a_forward_is_told_as_what_it_carries() {
        let anna = messenger_core::Forwarded::named(Some("Анна"));
        let text = Envelope::text("привет").with_fwd(Some(&anna));
        let fwd = |b: Body| Some(Body::Forwarded { body: Box::new(b) });
        assert_eq!(body_of(&text), fwd(Body::Text { text: "привет".into() }));
        let photo = file(serde_json::json!({ "kind": "image", "name": "a.png", "fwd": {} }));
        assert_eq!(body_of(&photo), fwd(Body::Media { kind: "image".into(), name: "a.png".into(), caption: None, duration_ms: None, batch: None }));
        assert_eq!(body_of(&Envelope::text("   ").with_fwd(Some(&anna))), None, "nothing to show stays nothing");
        assert_eq!(body_of(&Envelope::text("привет")), Some(Body::Text { text: "привет".into() }), "no fwd, no mark");

        let mut stored = row("text", Some("привет"), None, None);
        stored.envelope_json = text.encode();
        assert_eq!(body_of_row(&stored), fwd(Body::Text { text: "привет".into() }));
        let mut voice = row("media", None, None, Some(serde_json::json!({ "kind": "voice", "name": "v.weba", "duration_ms": 1000 })));
        voice.envelope_json = r#"{"v":1,"t":"media","kind":"voice","fwd":{"name":"Анна"}}"#.into();
        assert_eq!(body_of_row(&voice), fwd(Body::Media { kind: "voice".into(), name: "v.weba".into(), caption: None, duration_ms: Some(1000), batch: None }));
    }

    #[test]
    fn markdown_is_told_without_its_marks() {
        let md = |t: &str| Envelope::text(t).with_format(messenger_core::TextFormat::Markdown);
        assert_eq!(body_of(&md("# Plan\n\n**bold** and `code`")), Some(Body::Text { text: "Plan bold and code".into() }));
        assert_eq!(body_of(&Envelope::text("**bold**")), Some(Body::Text { text: "**bold**".into() }), "plain text keeps its stars");
        let mut caption = file(serde_json::json!({ "kind": "image", "name": "a.jpg", "caption": "*look*" }));
        caption = caption.with_format(messenger_core::TextFormat::Markdown);
        let Some(Body::Media { caption, .. }) = body_of(&caption) else { panic!() };
        assert_eq!(caption.as_deref(), Some("look"));
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

        // The card of a server: its name and host, never its app key.
        let server = messenger_links::ServerLink {
            key: Some("app-key".into()),
            name: "S1".into(),
            ..messenger_links::ServerLink::new(messenger_core::RelayUrl::parse("wss://relay.s1.example").unwrap())
        };
        assert_eq!(body_of(&Envelope::text(&server.encode())), Some(Body::Text { text: "S1 (relay.s1.example)".into() }));

        // A link of a kind this app does not know is shown as it came.
        assert_eq!(body_of(&Envelope::text("veydan://poll/abc")), Some(Body::Text { text: "veydan://poll/abc".into() }));
    }

    /// A key of a server in a text among words, a server card the parser
    /// refuses, a `srv=` on a person's or a group's link: the line of the
    /// list and of a notification shows `…` in their place.
    #[test]
    fn the_key_of_a_server_is_not_in_a_line() {
        let server = messenger_links::ServerLink {
            key: Some("s3cr3t-app-key".into()),
            name: "S1".into(),
            ..messenger_links::ServerLink::new(messenger_core::RelayUrl::parse("wss://relay.s1.example").unwrap())
        };
        let card = server.encode();
        let npub = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";
        let person = UriBuilder::new(&LinkType::Contact, npub).param("n", "Анна").param("srv", &server.srv_param()).build();
        let srv = server.srv_param();
        let lines = [
            format!("наш сервер: {card} — добавь"),
            format!("{card} ещё раз"),
            // A card of a relay in clear, which the parser refuses, alone.
            card.replace("wss%3A%2F%2F", "ws%3A%2F%2F"),
            format!("пиши мне там: {person}"),
            format!("(veydan://group/{}?r=wss%3A%2F%2Fr&SRV={srv}#x)", "ab".repeat(32)),
        ];
        for text in &lines {
            let Some(Body::Text { text: line }) = body_of(&Envelope::text(text)) else { panic!("{text}") };
            // Cut at 120 characters or not, nothing of the key is left.
            assert!(!line.contains("s3cr3t") && !line.contains(&srv[..12]), "{line}");
        }
        assert!(redact_secrets(&lines[3]).ends_with("&srv=…"), "{}", redact_secrets(&lines[3]));
        assert!(redact_secrets(&lines[4]).ends_with("&SRV=…#x)"), "{}", redact_secrets(&lines[4]));
        assert_eq!(redact_secrets(&format!("x {card} y")), format!("x {} y", card.replace("k=s3cr3t-app-key", "k=…")));
        assert_eq!(redact_secrets("veydan://server/ab?u=x&k=&p=off"), "veydan://server/ab?u=x&k=&p=off", "an empty key is none");
        assert_eq!(redact_secrets("k=1 https://x.org/?k=2"), "k=1 https://x.org/?k=2", "only links of the app");
        // A caption, and a stored message as the list reads it.
        let photo = file(serde_json::json!({ "kind": "image", "name": "a.jpg", "caption": format!("ключ: {card}") }));
        let Some(Body::Media { caption: Some(caption), .. }) = body_of(&photo) else { panic!() };
        assert!(!caption.contains("s3cr3t"), "{caption}");
        let stored = row("text", Some(&lines[0]), None, None);
        let Some(Body::Text { text: line }) = body_of_row(&stored) else { panic!() };
        assert!(!line.contains("s3cr3t"), "{line}");
    }

    #[test]
    fn a_link_in_a_text_is_found_in_any_case() {
        for text in [
            "see https://example.org",
            "HTTP://EXAMPLE.ORG/x",
            "veydan://contact/npub1abc",
            "go to WWW.example.org now",
            "my key NPUB1qqqq",
            "nprofile1qqqs...",
            "Nostr:npub1abc",
            "nostr:nevent1abc",
        ] {
            assert!(has_link(text), "{text}");
        }
        for text in ["hello there", "example.org", "mail me at a@b.c", "npub is a word", "www", "http:/x", ""] {
            assert!(!has_link(text), "{text}");
        }
    }

    #[test]
    fn a_contact_card_is_told_by_its_name() {
        let card = serde_json::json!({ "pubkey": "ab".repeat(32), "name": "anna", "display_name": "  Анна\u{202E}\nСмирнова " });
        assert_eq!(
            body_of(&Envelope::contact(card)),
            Some(Body::Link { link: LinkKind::Contact, title: "Анна Смирнова".into() })
        );
        let nameless = Envelope::contact(serde_json::json!({ "pubkey": "ab".repeat(32) }));
        assert_eq!(body_of(&nameless), Some(Body::Link { link: LinkKind::Contact, title: String::new() }));
        let broken = Envelope::new(T_CONTACT).with("card", "junk");
        assert_eq!(body_of(&broken), Some(Body::Link { link: LinkKind::Contact, title: String::new() }));
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
