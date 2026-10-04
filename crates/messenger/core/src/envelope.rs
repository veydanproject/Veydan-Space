// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Application envelope carried inside encrypted content (DM rumors, group
//! messages). One shape for everything: `{"v":1,"t":"<type>", …fields}`.
//! Unknown `t` values are preserved so newer clients can add types without
//! breaking older readers. See docs/messenger-wire.md §2.

use crate::error::{MessengerError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ENVELOPE_VERSION: u32 = 1;

/// Envelope types this version knows. Unknown ones are carried as-is.
pub const T_TEXT: &str = "text";
/// `{"t":"edit","target":"<rumor id>","text":"…"}` — replace a message's text.
pub const T_EDIT: &str = "edit";
/// `{"t":"delete","target":"<rumor id>"}` — retract a message.
pub const T_DELETE: &str = "delete";
/// `{"t":"control","action":"…"}` — DM relationship signal (stage 5b).
pub const T_CONTROL: &str = "control";
/// `{"t":"media", …}` — encrypted blob reference (stage 6).
pub const T_MEDIA: &str = "media";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub t: String,
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl Envelope {
    pub fn new(t: &str) -> Self {
        Self { v: ENVELOPE_VERSION, t: t.to_string(), fields: Map::new() }
    }

    pub fn text(text: &str) -> Self {
        let mut e = Self::new(T_TEXT);
        e.fields.insert("text".into(), Value::String(text.to_string()));
        e
    }

    pub fn edit(target: &str, text: &str) -> Self {
        Self::new(T_EDIT).with("target", target).with("text", text)
    }

    pub fn delete(target: &str) -> Self {
        Self::new(T_DELETE).with("target", target)
    }

    pub fn control(action: &str) -> Self {
        Self::new(T_CONTROL).with("action", action)
    }

    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).and_then(Value::as_str)
    }

    /// Compact JSON. Field order is `v`, `t`, then the rest as inserted;
    /// readers must not depend on order.
    pub fn encode(&self) -> String {
        serde_json::to_string(self).expect("envelope is always serializable")
    }

    /// Strict parse: must be a JSON object with numeric `v` and string `t`.
    /// Versions above `ENVELOPE_VERSION` are rejected; older ones accepted.
    pub fn parse(content: &str) -> Result<Self> {
        let e: Envelope = serde_json::from_str(content)
            .map_err(|e| MessengerError::Invalid(format!("envelope: {e}")))?;
        if e.v == 0 || e.v > ENVELOPE_VERSION {
            return Err(MessengerError::Invalid(format!("envelope v{} unsupported", e.v)));
        }
        if e.t.is_empty() {
            return Err(MessengerError::Invalid("envelope has an empty type".into()));
        }
        Ok(e)
    }

    /// Plain-text body for `t = text`.
    pub fn as_text(&self) -> Option<&str> {
        if self.t == T_TEXT {
            self.str_field("text")
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_golden_vector() {
        let e = Envelope::text("hello");
        assert_eq!(e.encode(), r#"{"v":1,"t":"text","text":"hello"}"#);
        let back = Envelope::parse(r#"{"v":1,"t":"text","text":"hello"}"#).unwrap();
        assert_eq!(back, e);
        assert_eq!(back.as_text(), Some("hello"));
    }

    #[test]
    fn edit_delete_control_golden_vectors() {
        assert_eq!(Envelope::edit("ab", "new").encode(), r#"{"v":1,"t":"edit","target":"ab","text":"new"}"#);
        assert_eq!(Envelope::delete("ab").encode(), r#"{"v":1,"t":"delete","target":"ab"}"#);
        assert_eq!(Envelope::control("dm_accept").encode(), r#"{"v":1,"t":"control","action":"dm_accept"}"#);
        let e = Envelope::parse(r#"{"v":1,"t":"edit","target":"ab","text":"new"}"#).unwrap();
        assert_eq!(e.str_field("target"), Some("ab"));
        assert!(e.as_text().is_none(), "edits are not plain text");
    }

    #[test]
    fn unknown_type_is_preserved_and_extra_fields_survive() {
        let e = Envelope::parse(r#"{"v":1,"t":"sticker","pack":"p","id":3,"extra":{"a":1}}"#).unwrap();
        assert_eq!(e.t, "sticker");
        assert_eq!(e.str_field("pack"), Some("p"));
        assert!(e.as_text().is_none());
        assert_eq!(Envelope::parse(&e.encode()).unwrap(), e);
    }

    #[test]
    fn rejects_garbage_and_future_versions() {
        assert!(Envelope::parse("hello plain text").is_err());
        assert!(Envelope::parse(r#"{"t":"text"}"#).is_err(), "missing v");
        assert!(Envelope::parse(r#"{"v":2,"t":"text"}"#).is_err(), "future version");
        assert!(Envelope::parse(r#"{"v":0,"t":"text"}"#).is_err());
        assert!(Envelope::parse(r#"{"v":1,"t":""}"#).is_err());
        assert!(Envelope::parse(r#"[1,2]"#).is_err());
    }
}
