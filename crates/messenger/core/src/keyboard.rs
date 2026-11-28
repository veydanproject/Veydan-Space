// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Buttons under a message of a bot (internal/platform-spec.md §9.7,
//! the contract of the bridge (sdk/docs/bridge.md, «Buttons under a message»)). A button is a label and an opaque
//! string the app hands back to the bot byte for byte (`d`); nothing
//! else. Pressing one does nothing in the app but send a `press` to the
//! author. Buttons with a link (`url`) are not in this version (the
//! owner's word, 2026-11-28): a keyboard with one is malformed.
//!
//! `{"t":"text","text":"…","kb":{"rows":[[{"t":"Allow","d":"a:1"}],
//! [{"t":"Deny","d":"d:1","style":"danger"}]]}}` — `kb` rides on a `text`
//! and on an `edit` (which replaces the keyboard, and takes it away when
//! it has none). `{"t":"press","of":"<message id>","d":"a:1"}` is the
//! press, a note to the author.
//!
//! Two readings of one shape. The sender's (`Keyboard::for_send`) refuses
//! whatever is over a limit or malformed, with a code the host shows. The
//! receiver's (`Keyboard::received`) drops the whole keyboard when it is
//! malformed or over a limit, cuts a long label, and strips from every
//! label the characters that could make it read as something else
//! (`clean_label`); the text of the message is shown either way.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The field of a `text` or an `edit` that carries the keyboard.
pub const FIELD_KB: &str = "kb";
/// `{"t":"press","of":"<message id>","d":"<data of the button>"}`.
pub const T_PRESS: &str = "press";

/// Rows of one keyboard.
pub const MAX_KB_ROWS: usize = 8;
/// Buttons of one row.
pub const MAX_KB_ROW_BUTTONS: usize = 4;
/// Characters of a label, after `clean_label`; a longer one is refused by
/// the sender and cut with «…» by the receiver.
pub const MAX_KB_LABEL_CHARS: usize = 64;
/// Bytes of the data of a button (`d`), and of a `press`.
pub const MAX_KB_DATA_BYTES: usize = 128;
/// Bytes of the whole `kb` as JSON.
pub const MAX_KB_BYTES: usize = 4096;
/// How long the app holds a second press of the same message while no
/// edit came: the press may not have reached the bot (§2 D6).
pub const PRESS_REPEAT_SECS: i64 = 30;
/// Presses of the same (message, data, presser) within this are told once.
pub const PRESS_DEDUP_SECS: i64 = 60;

/// How a button looks: `quiet` unless said otherwise; the one the bot
/// waits for is `primary`, a refusal `danger`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonStyle {
    Primary,
    #[default]
    Quiet,
    Danger,
}

impl ButtonStyle {
    pub fn is_quiet(&self) -> bool {
        *self == ButtonStyle::Quiet
    }

    fn of(name: &str) -> Option<Self> {
        match name {
            "primary" => Some(Self::Primary),
            "quiet" => Some(Self::Quiet),
            "danger" => Some(Self::Danger),
            _ => None,
        }
    }
}

/// One button: its label `t` and the data `d` the bot gets back when it
/// is pressed. `style` is left out when quiet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    pub t: String,
    pub d: String,
    #[serde(default, skip_serializing_if = "ButtonStyle::is_quiet")]
    pub style: ButtonStyle,
}

impl Button {
    pub fn data(t: &str, d: &str) -> Self {
        Self {
            t: t.into(),
            d: d.into(),
            style: ButtonStyle::Quiet,
        }
    }

    pub fn styled(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }
}

/// Rows of buttons under a message, as on the wire (`kb`) and as the host
/// is shown them (`MessageView.kb`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyboard {
    pub rows: Vec<Vec<Button>>,
}

/// Why the sender's keyboard is refused: a code the host translates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KbError {
    /// Not `{"rows":[[{…},…],…]}`, a label or a field of the wrong type.
    Shape,
    /// No rows, or more than `MAX_KB_ROWS`.
    Rows,
    /// A row without buttons.
    EmptyRow,
    /// More than `MAX_KB_ROW_BUTTONS` in a row.
    RowButtons,
    /// A label empty once cleaned.
    LabelEmpty,
    /// A label over `MAX_KB_LABEL_CHARS` once cleaned.
    LabelLong,
    /// No `d`.
    Action,
    /// `d` empty or over `MAX_KB_DATA_BYTES`.
    Data,
    /// A button with a link (`url`): not in this version, whatever the
    /// link (`javascript:`, `file:`, `https:` alike).
    Url,
    /// `style` none of primary, quiet, danger.
    Style,
    /// The whole `kb` over `MAX_KB_BYTES`.
    Size,
}

impl KbError {
    pub fn code(self) -> &'static str {
        match self {
            KbError::Shape => "kb_shape",
            KbError::Rows => "kb_rows",
            KbError::EmptyRow => "kb_empty_row",
            KbError::RowButtons => "kb_row_buttons",
            KbError::LabelEmpty => "kb_label_empty",
            KbError::LabelLong => "kb_label_long",
            KbError::Action => "kb_action",
            KbError::Data => "kb_data",
            KbError::Url => "kb_url",
            KbError::Style => "kb_style",
            KbError::Size => "kb_size",
        }
    }
}

impl From<KbError> for crate::MessengerError {
    fn from(e: KbError) -> Self {
        crate::MessengerError::Invalid(e.code().into())
    }
}

/// Whether `c` is stripped from a label: the characters that change the
/// direction of the writing (U+061C ALM, U+200E LRM, U+200F RLM,
/// U+202A–U+202E LRE RLE PDF LRO RLO, U+2066–U+2069 LRI RLI FSI PDI), so
/// that «Отклонить» cannot be made to read as «Разрешить», and the byte
/// order mark U+FEFF. Control characters (C0, DEL, C1) and the line and
/// paragraph separators U+2028, U+2029 become a blank instead.
pub fn stripped(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// A label as it is shown: one line, without the characters of `stripped`,
/// controls and separators turned to blanks, blanks collapsed and trimmed.
/// Not cut: the sender refuses a long one, the receiver cuts it.
pub fn clean_label(label: &str) -> String {
    let flat: String = label
        .chars()
        .filter(|c| !stripped(*c))
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A label cut to `MAX_KB_LABEL_CHARS` with «…» at the end.
fn cut_label(label: String) -> String {
    if label.chars().count() <= MAX_KB_LABEL_CHARS {
        return label;
    }
    let mut cut: String = label.chars().take(MAX_KB_LABEL_CHARS - 1).collect();
    cut.push('…');
    cut
}

impl Keyboard {
    /// The keyboard a sender puts on a message, read strictly: every limit
    /// and every malformed piece is refused. Labels come back cleaned.
    pub fn for_send(v: &Value) -> Result<Self, KbError> {
        read(v, true)
    }

    /// A keyboard a sender built in code, held to the same rules.
    pub fn checked(&self) -> Result<Self, KbError> {
        Self::for_send(&self.to_value())
    }

    /// The keyboard of a message as it came: `None` when there is none or
    /// it is malformed or over a limit (the whole keyboard goes; the text
    /// stays). Labels cleaned and cut.
    pub fn received(v: &Value) -> Option<Self> {
        read(v, false).ok()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a keyboard serializes")
    }

    /// Whether a button of it carries exactly this data.
    pub fn has_data(&self, d: &str) -> bool {
        self.rows.iter().flatten().any(|b| b.d == d)
    }
}

fn read(v: &Value, strict: bool) -> Result<Keyboard, KbError> {
    if serde_json::to_string(v)
        .map(|s| s.len())
        .unwrap_or(usize::MAX)
        > MAX_KB_BYTES
    {
        return Err(KbError::Size);
    }
    let rows = v
        .as_object()
        .and_then(|o| o.get("rows"))
        .and_then(Value::as_array)
        .ok_or(KbError::Shape)?;
    if rows.is_empty() || rows.len() > MAX_KB_ROWS {
        return Err(KbError::Rows);
    }
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let buttons = row.as_array().ok_or(KbError::Shape)?;
        if buttons.is_empty() {
            return Err(KbError::EmptyRow);
        }
        if buttons.len() > MAX_KB_ROW_BUTTONS {
            return Err(KbError::RowButtons);
        }
        let mut line = Vec::with_capacity(buttons.len());
        for b in buttons {
            line.push(button(b, strict)?);
        }
        out.push(line);
    }
    Ok(Keyboard { rows: out })
}

fn button(v: &Value, strict: bool) -> Result<Button, KbError> {
    let o = v.as_object().ok_or(KbError::Shape)?;
    let t = clean_label(o.get("t").and_then(Value::as_str).ok_or(KbError::Shape)?);
    if t.is_empty() {
        return Err(KbError::LabelEmpty);
    }
    if strict && t.chars().count() > MAX_KB_LABEL_CHARS {
        return Err(KbError::LabelLong);
    }
    // A link is no button of this version, whatever it holds (null too).
    if o.contains_key("url") {
        return Err(KbError::Url);
    }
    let d = match o.get("d") {
        None | Some(Value::Null) => return Err(KbError::Action),
        Some(Value::String(d)) => d.clone(),
        Some(_) => return Err(KbError::Shape),
    };
    if d.is_empty() || d.len() > MAX_KB_DATA_BYTES {
        return Err(KbError::Data);
    }
    let style = match o.get("style") {
        None | Some(Value::Null) => ButtonStyle::Quiet,
        Some(Value::String(s)) => match ButtonStyle::of(s) {
            Some(style) => style,
            None if strict => return Err(KbError::Style),
            // A style of a newer build: shown as the plain one.
            None => ButtonStyle::Quiet,
        },
        Some(_) if strict => return Err(KbError::Style),
        Some(_) => ButtonStyle::Quiet,
    };
    Ok(Button {
        t: cut_label(t),
        d,
        style,
    })
}

/// What a `press` says, read strictly: the id of the message (64 hex,
/// lower case) and the data of the button. `None` for anything else.
pub fn press_of(e: &crate::Envelope) -> Option<(String, String)> {
    if e.t != T_PRESS {
        return None;
    }
    let of = crate::EventId::parse(e.str_field("of")?)?;
    let d = e.str_field("d")?;
    (!d.is_empty() && d.len() <= MAX_KB_DATA_BYTES)
        .then(|| (of.as_hex().to_string(), d.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Envelope;
    use serde_json::json;

    #[test]
    fn kb_golden_vector_on_a_text_and_an_edit() {
        let k = Keyboard {
            rows: vec![
                vec![Button::data("Allow", "a:1").styled(ButtonStyle::Primary)],
                vec![
                    Button::data("Deny", "d:1").styled(ButtonStyle::Danger),
                    Button::data("Later", "l:1"),
                ],
            ],
        };
        // The keys of every object come sorted, as everywhere in an envelope.
        let wire = r#"{"v":1,"t":"text","kb":{"rows":[[{"d":"a:1","style":"primary","t":"Allow"}],[{"d":"d:1","style":"danger","t":"Deny"},{"d":"l:1","t":"Later"}]]},"text":"Open it?"}"#;
        assert_eq!(Envelope::text("Open it?").with_kb(Some(&k)).encode(), wire);
        let back = Envelope::parse(wire).unwrap();
        assert_eq!(back.kb(), Some(k.clone()));
        assert_eq!(
            back.as_text(),
            Some("Open it?"),
            "a reader before the field reads the text"
        );
        let edit = Envelope::edit("ab", "Done").with_kb(Some(&k));
        assert_eq!(
            Envelope::parse(&edit.encode()).unwrap().kb(),
            Some(k.clone())
        );
        assert_eq!(
            Envelope::edit("ab", "Done").kb(),
            None,
            "an edit without kb has no keyboard"
        );
        assert_eq!(
            edit.with_kb(None).encode(),
            r#"{"v":1,"t":"edit","target":"ab","text":"Done"}"#
        );
        assert_eq!(
            Envelope::new("media").with("kb", k.to_value()).kb(),
            None,
            "a text and an edit only"
        );
    }

    #[test]
    fn press_golden_vector() {
        let of = "ab".repeat(32);
        let wire = format!(r#"{{"v":1,"t":"press","d":"a:78fe6469","of":"{of}"}}"#);
        assert_eq!(Envelope::press(&of, "a:78fe6469").encode(), wire);
        assert_eq!(
            press_of(&Envelope::parse(&wire).unwrap()),
            Some((of.clone(), "a:78fe6469".into()))
        );
        assert_eq!(
            press_of(&Envelope::press(&of.to_uppercase(), "x")),
            Some((of.clone(), "x".into())),
            "the id read in lower case"
        );
        for bad in [
            Envelope::press("abc", "x"),
            Envelope::press(&of, ""),
            Envelope::press(&of, &"x".repeat(129)),
            Envelope::text("x"),
        ] {
            assert_eq!(press_of(&bad), None, "{}", bad.encode());
        }
        assert_eq!(
            press_of(&Envelope::new(T_PRESS).with("d", "x")),
            None,
            "no of"
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let v = json!({"rows":[[{"t":"Ok","d":"1","color":"red","x":{"y":1}}]],"layout":"grid"});
        assert_eq!(
            Keyboard::received(&v),
            Some(Keyboard {
                rows: vec![vec![Button::data("Ok", "1")]]
            })
        );
        assert!(Keyboard::for_send(&v).is_ok());
    }

    #[test]
    fn nine_rows_are_refused() {
        let row = json!([{"t":"x","d":"1"}]);
        let eight = json!({ "rows": vec![row.clone(); 8] });
        let nine = json!({ "rows": vec![row; 9] });
        assert!(Keyboard::for_send(&eight).is_ok());
        assert_eq!(Keyboard::for_send(&nine), Err(KbError::Rows));
        assert_eq!(Keyboard::received(&nine), None);
        assert_eq!(
            Keyboard::for_send(&json!({"rows":[]})),
            Err(KbError::Rows),
            "no rows"
        );
    }

    #[test]
    fn five_buttons_in_a_row_are_refused() {
        let b = json!({"t":"x","d":"1"});
        assert!(Keyboard::for_send(&json!({"rows":[vec![b.clone(); 4]]})).is_ok());
        let five = json!({"rows":[vec![b; 5]]});
        assert_eq!(Keyboard::for_send(&five), Err(KbError::RowButtons));
        assert_eq!(Keyboard::received(&five), None);
    }

    #[test]
    fn an_empty_row_drops_the_keyboard() {
        let v = json!({"rows":[[{"t":"x","d":"1"}],[]]});
        assert_eq!(Keyboard::for_send(&v), Err(KbError::EmptyRow));
        assert_eq!(Keyboard::received(&v), None);
    }

    #[test]
    fn a_65_char_label_is_truncated_at_receive_and_refused_at_send() {
        let long = "я".repeat(65);
        let v = json!({"rows":[[{"t": long, "d":"1"}]]});
        assert_eq!(Keyboard::for_send(&v), Err(KbError::LabelLong));
        let got = Keyboard::received(&v).unwrap().rows[0][0].t.clone();
        assert_eq!(got.chars().count(), MAX_KB_LABEL_CHARS);
        assert!(got.ends_with('…'));
        assert_eq!(got, format!("{}…", "я".repeat(63)));
        let exact = json!({"rows":[[{"t": "я".repeat(64), "d":"1"}]]});
        assert_eq!(
            Keyboard::for_send(&exact).unwrap().rows[0][0].t,
            "я".repeat(64)
        );
    }

    #[test]
    fn rtlo_and_bidi_in_a_label_are_stripped() {
        // «Отклонить» with RLO … PDF around a reversed «Разрешить».
        let label = "\u{202E}ьтишерзаР\u{202C} \u{2066}x\u{2069}\u{200F}\u{200E}\u{061C}\u{FEFF}\u{202A}\u{202B}\u{202D}\u{2067}\u{2068}";
        let v = json!({"rows":[[{"t": label, "d":"1"}]]});
        let got = Keyboard::received(&v).unwrap().rows[0][0].t.clone();
        assert_eq!(got, "ьтишерзаР x");
        assert!(!got.chars().any(stripped));
        assert_eq!(
            Keyboard::for_send(&v).unwrap().rows[0][0].t,
            "ьтишерзаР x",
            "the sender's is cleaned too"
        );
        assert_eq!(
            clean_label("a\nb\u{7}\u{85}c\u{2028}d\t "),
            "a b c d",
            "controls and separators are blanks"
        );
        let only_marks = json!({"rows":[[{"t": "\u{202E}\u{200F} \n", "d":"1"}]]});
        assert_eq!(Keyboard::for_send(&only_marks), Err(KbError::LabelEmpty));
        assert_eq!(
            Keyboard::received(&only_marks),
            None,
            "a label of nothing breaks the keyboard"
        );
    }

    #[test]
    fn a_button_without_d_is_refused() {
        let neither = json!({"rows":[[{"t":"x"}]]});
        assert_eq!(Keyboard::for_send(&neither), Err(KbError::Action));
        assert_eq!(Keyboard::received(&neither), None);
        let null = json!({"rows":[[{"t":"x","d":null}]]});
        assert_eq!(Keyboard::received(&null), None);
    }

    /// No link buttons in version 1 (the owner, 2026-11-28): a `url` on a
    /// button, `javascript:` or `https:`, alone or beside `d`, breaks the
    /// keyboard at the receiver and is refused at the sender.
    #[test]
    fn url_buttons_are_refused_javascript_file_data_and_https_alike() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,<b>x</b>",
            "veydan://server/abc",
            "https://wttr.in/",
        ] {
            for v in [
                json!({"rows":[[{"t":"x","url": url}]]}),
                json!({"rows":[[{"t":"ok","d":"1"},{"t":"x","d":"2","url": url}]]}),
            ] {
                assert_eq!(Keyboard::for_send(&v), Err(KbError::Url), "{v}");
                assert_eq!(Keyboard::received(&v), None, "{v}: the whole keyboard goes");
            }
        }
        assert_eq!(
            Keyboard::received(&json!({"rows":[[{"t":"x","d":"1","url":null}]]})),
            None,
            "a url of null is a url"
        );
    }

    #[test]
    fn data_over_128_bytes_is_refused() {
        assert!(Keyboard::for_send(&json!({"rows":[[{"t":"x","d": "a".repeat(128)}]]})).is_ok());
        let v = json!({"rows":[[{"t":"x","d": "a".repeat(129)}]]});
        assert_eq!(Keyboard::for_send(&v), Err(KbError::Data));
        assert_eq!(Keyboard::received(&v), None);
        assert_eq!(
            Keyboard::for_send(&json!({"rows":[[{"t":"x","d":""}]]})),
            Err(KbError::Data)
        );
    }

    #[test]
    fn a_keyboard_over_4_kib_is_refused() {
        // 8 rows of 4 buttons with 128 bytes of data each: well over 4 KiB.
        let b = json!({"t":"x","d": "a".repeat(128)});
        let v = json!({ "rows": vec![vec![b; 4]; 8] });
        assert_eq!(Keyboard::for_send(&v), Err(KbError::Size));
        assert_eq!(Keyboard::received(&v), None);
    }

    #[test]
    fn a_broken_kb_is_no_keyboard_but_the_envelope_reads() {
        for broken in [
            r#""rows""#,
            "[]",
            r#"{"rows":"x"}"#,
            r#"{"rows":[["x"]]}"#,
            r#"{"rows":[[{"d":"1"}]]}"#,
            r#"{"rows":[[{"t":7,"d":"1"}]]}"#,
            r#"{"rows":[[{"t":"x","d":7}]]}"#,
        ] {
            let e = Envelope::parse(&format!(
                r#"{{"v":1,"t":"text","text":"hi","kb":{broken}}}"#
            ))
            .unwrap();
            assert_eq!(e.kb(), None, "{broken}");
            assert_eq!(e.as_text(), Some("hi"), "{broken}: the text stays");
        }
    }

    #[test]
    fn styles() {
        let v = json!({"rows":[[{"t":"a","d":"1","style":"primary"},{"t":"b","d":"2","style":"quiet"},{"t":"c","d":"3","style":"danger"}]]});
        let k = Keyboard::for_send(&v).unwrap();
        assert_eq!(
            k.rows[0].iter().map(|b| b.style).collect::<Vec<_>>(),
            vec![
                ButtonStyle::Primary,
                ButtonStyle::Quiet,
                ButtonStyle::Danger
            ]
        );
        assert!(
            !k.to_value().to_string().contains("quiet"),
            "quiet is never written"
        );
        let odd = json!({"rows":[[{"t":"a","d":"1","style":"success"}]]});
        assert_eq!(Keyboard::for_send(&odd), Err(KbError::Style));
        assert_eq!(
            Keyboard::received(&odd).unwrap().rows[0][0].style,
            ButtonStyle::Quiet,
            "a style of a newer build is the plain one"
        );
        assert!(Keyboard::has_data(&k, "2") && !k.has_data("4"));
    }

    #[test]
    fn refusals_reach_the_host_as_codes() {
        assert!(
            matches!(crate::MessengerError::from(KbError::Url), crate::MessengerError::Invalid(c) if c == "kb_url")
        );
    }
}
