// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! A contact card: a person's key and public profile sent as a message,
//! `{"v":1,"t":"contact","card":{…}}`, with a tiny picture inside so the
//! receiver shows it without fetching anything. A phone travels only in
//! my own card, and only when I choose so; a card forwarded or made of
//! somebody else's profile has none.
//!
//! A received card is untrusted. `validate` reads every field on its own
//! and keeps it only in the one form this side writes: a key of 64 hex,
//! up to three `wss://` relays, names of at most `MAX_NAME_CHARS` without
//! controls, a bio in the one form of messenger-richtext, an `https`
//! website, links as `social` checks them, a phone as `phone` writes it
//! and a picture decoded and encoded again as a fresh JPEG. A field that
//! fails is dropped; only a card without a valid key, or larger than
//! `MAX_CARD_BYTES`, is refused.

use crate::phone::normalize_phone;
use crate::profile::{short_npub, ProfileView};
use crate::social::{self, SocialLink, SocialView};
use base64::Engine;
use messenger_core::{MessengerError, RelayUrl, Result};
use messenger_richtext::{self as richtext, Span};
use nostr::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// The whole card as JSON.
pub const MAX_CARD_BYTES: usize = 24 * 1024;
/// `name` and `display_name`, in Unicode scalar values.
pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_RELAYS: usize = 3;
/// A relay's address.
const MAX_RELAY_BYTES: usize = 256;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The wire and storage form. Absent fields are left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactCard {
    /// Lowercase hex.
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The bio, with its marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub socials: Vec<SocialLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// Base64 of a JPEG of at most `messenger_avatar::CARD_MAX_BYTES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// When the card was made, unix seconds.
    #[serde(default)]
    pub at: i64,
}

/// A received card as the UI shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CardView {
    pub pubkey: String,
    pub npub: String,
    /// display_name → name → short npub.
    pub label: String,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub bio: Vec<Span>,
    pub website: Option<String>,
    pub socials: Vec<SocialView>,
    pub phone: Option<String>,
    /// A `data:` URL of the picture.
    pub avatar: Option<String>,
    /// The card is of me.
    pub is_me: bool,
    /// The person is in my contacts.
    pub is_contact: bool,
    pub blocked: bool,
}

impl ContactCard {
    /// A card of `profile`. `relays` are where the person reads; `phone` is
    /// only ever mine (refused as `phone_invalid` when it is no number);
    /// `avatar` is a picture in any format, made small here, and left out
    /// when it cannot be read. When the card would be larger than
    /// `MAX_CARD_BYTES` the picture is left out; when it still is, it is
    /// refused as `card_too_large`.
    pub fn from_profile(
        profile: &ProfileView,
        relays: &[String],
        phone: Option<&str>,
        avatar: Option<&[u8]>,
        at: i64,
    ) -> Result<ContactCard> {
        let pubkey = valid_pubkey(&profile.pubkey).ok_or_else(|| invalid("card_invalid"))?;
        let phone = match phone.map(str::trim).filter(|p| !p.is_empty()) {
            Some(p) => Some(normalize_phone(p).map_err(invalid)?),
            None => None,
        };
        let mut card = ContactCard {
            pubkey,
            relays: clean_relays(relays.iter().map(String::as_str)),
            name: profile.name.as_deref().and_then(clean_name),
            display_name: profile.display_name.as_deref().and_then(clean_name),
            // Markup, a plain `about` escaped (`ProfileView::bio_source`).
            about: profile.bio_source.as_deref().and_then(clean_about),
            website: profile.website.as_deref().and_then(clean_website),
            socials: social::clean_list(&profile.links),
            phone,
            avatar: avatar.and_then(|bytes| messenger_avatar::card_thumb(bytes).ok()).map(|jpeg| B64.encode(jpeg)),
            at: at.max(0),
        };
        if card.json_len() > MAX_CARD_BYTES {
            card.avatar = None;
        }
        if card.json_len() > MAX_CARD_BYTES {
            return Err(invalid("card_too_large"));
        }
        Ok(card)
    }

    /// The same card without the phone: what may be forwarded, and what is
    /// kept of a card about somebody other than its sender.
    pub fn without_phone(mut self) -> Self {
        self.phone = None;
        self
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("a card serializes")
    }

    /// display_name → name → short npub.
    pub fn label(&self) -> String {
        self.display_name
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| short_npub(&npub_of(&self.pubkey)))
    }

    fn json_len(&self) -> usize {
        serde_json::to_string(self).map_or(usize::MAX, |s| s.len())
    }
}

impl CardView {
    pub fn new(card: &ContactCard, is_me: bool, is_contact: bool, blocked: bool) -> Self {
        let avatar = card
            .avatar
            .as_deref()
            .and_then(|b| B64.decode(b).ok())
            .map(|jpeg| messenger_avatar::data_url(&jpeg));
        CardView {
            pubkey: card.pubkey.clone(),
            npub: npub_of(&card.pubkey),
            label: card.label(),
            name: card.name.clone(),
            display_name: card.display_name.clone(),
            bio: card.about.as_deref().map(richtext::parse).unwrap_or_default(),
            website: card.website.clone(),
            socials: card.socials.iter().filter_map(social::view).collect(),
            phone: card.phone.clone(),
            avatar,
            is_me,
            is_contact,
            blocked,
        }
    }
}

/// A received card in the one form (see the module). Errors (as
/// `Invalid`): `card_too_large`, `card_invalid` (not an object, or no
/// valid key).
pub fn validate(raw: &Value) -> Result<ContactCard> {
    if serde_json::to_string(raw).map_or(true, |s| s.len() > MAX_CARD_BYTES) {
        return Err(invalid("card_too_large"));
    }
    let obj = raw.as_object().ok_or_else(|| invalid("card_invalid"))?;
    let text = |key: &str| obj.get(key).and_then(Value::as_str);
    let pubkey = text("pubkey").and_then(valid_pubkey).ok_or_else(|| invalid("card_invalid"))?;
    let relays = obj
        .get("relays")
        .and_then(Value::as_array)
        .map(|list| clean_relays(list.iter().filter_map(Value::as_str)))
        .unwrap_or_default();
    Ok(ContactCard {
        pubkey,
        relays,
        name: text("name").and_then(clean_name),
        display_name: text("display_name").and_then(clean_name),
        about: text("about").and_then(clean_about),
        website: text("website").and_then(clean_website),
        socials: obj.get("socials").map(social::list_from_json).unwrap_or_default(),
        phone: text("phone").and_then(|p| normalize_phone(p).ok()),
        avatar: text("avatar").and_then(clean_avatar),
        at: obj.get("at").and_then(Value::as_i64).filter(|at| *at >= 0).unwrap_or(0),
    })
}

/// The name a received card shows, display_name → name, checked as
/// `validate` checks it but without reading the rest (no picture is
/// decoded): for a notification. Empty when it has none.
pub fn name_of(raw: &Value) -> String {
    let text = |key: &str| raw.get(key).and_then(Value::as_str).and_then(clean_name);
    text("display_name").or_else(|| text("name")).unwrap_or_default()
}

fn invalid(code: &str) -> MessengerError {
    MessengerError::Invalid(code.into())
}

/// 64 hex of a valid key, lowercase.
fn valid_pubkey(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let s = s.to_ascii_lowercase();
    PublicKey::from_hex(&s).ok().map(|_| s)
}

fn npub_of(hex: &str) -> String {
    PublicKey::from_hex(hex).ok().and_then(|p| p.to_bech32().ok()).unwrap_or_else(|| hex.to_string())
}

/// Up to `MAX_RELAYS` distinct `wss://host[:port]` addresses.
fn clean_relays<'a>(list: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in list.take(MAX_RELAYS * 8) {
        if out.len() >= MAX_RELAYS {
            break;
        }
        if s.len() > MAX_RELAY_BYTES || !s.trim().starts_with("wss://") {
            continue;
        }
        let Some(url) = RelayUrl::parse(s) else { continue };
        let host = &url.as_str()["wss://".len()..];
        if !host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']')) {
            continue;
        }
        let url = url.as_str().to_ascii_lowercase();
        if !out.contains(&url) {
            out.push(url);
        }
    }
    out
}

/// One line in the one form, without controls, at most `MAX_NAME_CHARS`.
fn clean_name(s: &str) -> Option<String> {
    let one_line = richtext::normalize(s).replace('\n', " ");
    let name: String = one_line.trim().chars().take(MAX_NAME_CHARS).collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

fn clean_about(s: &str) -> Option<String> {
    Some(richtext::normalize(s)).filter(|m| !m.is_empty())
}

/// An `https` address with a host and no user name.
fn clean_website(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() > richtext::MAX_LINK_BYTES || !s.starts_with("https://") || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let url = Url::parse(s).ok()?;
    (url.scheme() == "https" && url.host_str().is_some_and(|h| !h.is_empty()) && url.username().is_empty() && url.password().is_none())
        .then(|| s.to_string())
}

/// The picture decoded and encoded again; anything else is dropped.
fn clean_avatar(b64: &str) -> Option<String> {
    let bytes = B64.decode(b64.trim()).ok()?;
    messenger_avatar::card_thumb(&bytes).ok().map(|jpeg| B64.encode(jpeg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use messenger_avatar::CARD_MAX_BYTES;
    use serde_json::json;

    fn hex_key() -> String {
        Keys::generate().public_key().to_hex()
    }

    /// A 24-bit BMP, `w` by `h`, of a gradient: the one format easy to
    /// write by hand.
    fn bmp(w: u32, h: u32) -> Vec<u8> {
        let row = (w * 3).div_ceil(4) * 4;
        let size = 54 + row * h;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(row * h).to_le_bytes());
        out.extend_from_slice(&[0; 16]);
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) & 0xff) as u8]);
            }
            out.extend(std::iter::repeat_n(0, (row - w * 3) as usize));
        }
        out
    }

    fn profile(pubkey: &str) -> ProfileView {
        ProfileView {
            pubkey: pubkey.into(),
            npub: npub_of(pubkey),
            name: Some("anna".into()),
            display_name: Some("Анна".into()),
            about: Some("Hi red".into()),
            picture: None,
            banner: None,
            website: Some("https://anna.example/".into()),
            nip05: None,
            lud16: None,
            nip05_verified: false,
            event_created_at: 1,
            fetched_at: 1,
            bio: Vec::new(),
            bio_source: Some("**Hi** {red}red{/}".into()),
            socials: Vec::new(),
            links: vec![SocialLink { p: "github".into(), h: "octocat".into() }],
        }
    }

    #[test]
    fn a_card_of_a_profile_survives_the_wire() {
        let pk = hex_key();
        let relays = vec!["wss://relay.example/".to_string(), "ws://plain.example".into(), "https://x.example".into()];
        let card = ContactCard::from_profile(&profile(&pk), &relays, Some("+7 (999) 123-45-67"), Some(&bmp(300, 200)), 1_759_700_000).unwrap();
        assert_eq!(card.relays, vec!["wss://relay.example"]);
        assert_eq!(card.phone.as_deref(), Some("+79991234567"));
        assert_eq!(card.about.as_deref(), Some("**Hi** {red}red{/}"));
        assert!(B64.decode(card.avatar.as_deref().unwrap()).unwrap().len() <= CARD_MAX_BYTES);

        let back = validate(&card.to_json()).unwrap();
        assert_eq!(ContactCard { avatar: None, ..back.clone() }, ContactCard { avatar: None, ..card.clone() });
        assert!(back.avatar.is_some(), "the picture is read and made again");

        let view = CardView::new(&back, false, true, false);
        assert_eq!(view.label, "Анна");
        assert!(view.npub.starts_with("npub1"));
        assert!(view.avatar.as_deref().unwrap().starts_with("data:image/jpeg;base64,"));
        assert_eq!(view.socials[0].url, "https://github.com/octocat");
        assert_eq!(view.bio[0], Span::Text { text: "Hi".into(), style: richtext::Style { bold: true, ..Default::default() } });
        assert!((view.is_contact, view.is_me, view.blocked) == (true, false, false));

        assert_eq!(card.clone().without_phone().phone, None);
        assert!(ContactCard::from_profile(&profile(&pk), &[], Some("call me"), None, 0).is_err(), "phone_invalid");
    }

    #[tokio::test]
    async fn a_plain_about_keeps_its_text() {
        // Written by another client: no marks of ours.
        let about = "2*3*4 is *not* bold, see https://a.example/x*y_{z}";
        let profiles = crate::ProfileService::new(messenger_store::Store::open_in_memory().await.unwrap());
        let pk = messenger_core::PubKey::parse(&hex_key()).unwrap();
        profiles.apply_event(&pk, messenger_core::Timestamp(1), &json!({ "about": about }).to_string()).await.unwrap();
        let p = profiles.get(&pk).await.unwrap().unwrap();
        let card = ContactCard::from_profile(&p, &[], None, None, 0).unwrap();
        let view = CardView::new(&validate(&card.to_json()).unwrap(), false, false, false);
        assert_eq!(view.bio, richtext::plain(about), "shows exactly as the plain about");
    }

    #[test]
    fn hostile_cards() {
        let pk = hex_key();
        // No key, a bad key, not an object: refused.
        for raw in [json!({}), json!({"pubkey": "zz"}), json!({"pubkey": "ab".repeat(33)}), json!([pk]), json!("x"), json!(null)] {
            assert!(validate(&raw).is_err(), "{raw}");
        }
        // Too large: refused before anything is read.
        assert!(validate(&json!({"pubkey": pk, "about": "x".repeat(MAX_CARD_BYTES)})).is_err());

        let bomb = {
            let mut b = bmp(4, 4);
            b[18..22].copy_from_slice(&100_000i32.to_le_bytes());
            b[22..26].copy_from_slice(&100_000i32.to_le_bytes());
            B64.encode(b)
        };
        let raw = json!({
            "pubkey": pk.to_uppercase(),
            "relays": ["wss://a.example", "wss://a.example/", "wss://user@evil.example", "wss://x.example/path", "javascript:alert(1)", 5,
                       "wss://b.example", "wss://c.example:7777", "wss://d.example"],
            "name": "\u{202E}evil\u{0007}name\nline two",
            "display_name": "x".repeat(500),
            "about": "\u{202E}**bold**\u{200F}",
            "website": "https://user:pw@evil.example/",
            "socials": [{"p": "github", "h": "evil.com/x"}, {"p": "github", "h": "ok-user"}, "junk"],
            "phone": "+0 123",
            "avatar": bomb,
            "at": -5,
            "extra": {"ignored": true}
        });
        let c = validate(&raw).unwrap();
        assert_eq!(c.pubkey, pk, "lowercase");
        assert_eq!(c.relays, vec!["wss://a.example", "wss://b.example", "wss://c.example:7777"]);
        assert_eq!(c.name.as_deref(), Some("evilname line two"));
        assert_eq!(c.display_name.as_ref().unwrap().chars().count(), MAX_NAME_CHARS);
        assert_eq!(c.about.as_deref(), Some("**bold**"));
        assert_eq!(c.website, None);
        assert_eq!(c.socials, vec![SocialLink { p: "github".into(), h: "ok-user".into() }]);
        assert_eq!((c.phone, c.avatar, c.at), (None, None, 0));

        // Fields of the wrong type are as good as absent.
        let c = validate(&json!({"pubkey": pk, "name": 1, "about": [], "website": {}, "socials": "x", "phone": 7, "avatar": "!!!", "relays": "wss://a.example", "at": "now"})).unwrap();
        assert_eq!(c, ContactCard { pubkey: pk.clone(), ..Default::default() });

        // Garbage that is base64 but no picture; an SVG.
        let svg = B64.encode(br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#);
        for avatar in [B64.encode(b"not a picture at all"), svg] {
            assert_eq!(validate(&json!({"pubkey": pk, "avatar": avatar})).unwrap().avatar, None);
        }
    }

    #[test]
    fn the_name_of_a_card_for_a_notification() {
        assert_eq!(name_of(&json!({"display_name": " \u{202E}Анна\n ", "name": "anna"})), "Анна");
        assert_eq!(name_of(&json!({"display_name": "", "name": "anna"})), "anna");
        assert_eq!(name_of(&json!({"display_name": 5, "name": ["x"]})), "");
        assert_eq!(name_of(&json!("x")), "");
        assert_eq!(name_of(&json!({"name": "x".repeat(500)})).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn a_website_must_be_https() {
        for bad in ["http://a.example", "javascript:alert(1)", "https://", "https://a b.example", "ftp://a.example", "https://u@a.example"] {
            assert_eq!(clean_website(bad), None, "{bad}");
        }
        assert_eq!(clean_website(" https://a.example/x?y=1 ").as_deref(), Some("https://a.example/x?y=1"));
    }

    #[test]
    fn a_card_too_large_loses_its_picture_first() {
        let pk = hex_key();
        // Four-byte characters: the longest bio in bytes.
        let p = ProfileView { bio_source: Some("😀".repeat(richtext::MAX_CHARS)), ..profile(&pk) };
        // Noise: a picture that stays near the largest a card takes.
        let mut noisy = bmp(160, 160);
        let mut seed = 0x2545_f491_u32;
        for b in &mut noisy[54..] {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *b = seed as u8;
        }
        let small = ContactCard::from_profile(&profile(&pk), &[], None, Some(&noisy), 0).unwrap();
        assert!(small.avatar.is_some(), "a usual card keeps its picture");
        let full = ContactCard::from_profile(&p, &[], None, Some(&noisy), 0).unwrap();
        assert!(full.avatar.is_some(), "the longest bio still leaves room for it");

        // The longest bio and every link: the picture goes, the rest stays whole.
        let lots: Vec<SocialLink> = (0..social::MAX_SOCIALS)
            .map(|i| SocialLink { p: "other".into(), h: format!("https://a{i}.example/{}", "x".repeat(480)) })
            .collect();
        let p = ProfileView { links: lots.clone(), ..p };
        let card = ContactCard::from_profile(&p, &[], None, Some(&noisy), 0).unwrap();
        assert!(card.avatar.is_none(), "the picture goes first");
        assert_eq!((&card.about, &card.socials), (&p.bio_source, &lots), "the rest stays whole");
        assert!(serde_json::to_string(&card).unwrap().len() <= MAX_CARD_BYTES);
        assert_eq!(validate(&card.to_json()).unwrap(), card, "and is a valid card");
    }
}
