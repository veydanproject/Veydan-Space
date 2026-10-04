// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

use serde::{Deserialize, Serialize};

/// Binary payload captured in the page (image, screenshot), base64 encoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureAsset {
    /// Original URL for article images (used to rewrite links), empty for screenshots
    #[serde(default)]
    pub src: String,
    pub name: String,
    pub data_b64: String,
}

/// Payload sent by the extension; relayed verbatim by the host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRequest {
    /// `selection_new` | `selection_append` | `bookmark` | `article` | `screenshot`
    /// | `list_notes` | `search_notes` | `open_note`
    pub kind: String,
    pub profile_id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub selection_text: String,
    #[serde(default)]
    pub selection_html: String,
    /// Article body already converted to Markdown by the extension
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub assets: Vec<CaptureAsset>,
    /// Target note for `open_note` and `selection_append` (last profile note when absent)
    #[serde(default)]
    pub note_id: Option<String>,
    /// Search text for `search_notes`
    #[serde(default)]
    pub query: String,
}

/// Note summary for the extension popup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteRef {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    /// `url` | `domain` | `profile` | `all`
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<NoteRef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CaptureResponse {
    pub fn ok(note_id: String, title: String) -> Self {
        Self {
            ok: true,
            note_id: Some(note_id),
            title: Some(title),
            notes: None,
            error: None,
        }
    }
    pub fn notes(notes: Vec<NoteRef>) -> Self {
        Self {
            ok: true,
            note_id: None,
            title: None,
            notes: Some(notes),
            error: None,
        }
    }
    pub fn done() -> Self {
        Self {
            ok: true,
            note_id: None,
            title: None,
            notes: None,
            error: None,
        }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            note_id: None,
            title: None,
            notes: None,
            error: Some(msg.into()),
        }
    }
}
