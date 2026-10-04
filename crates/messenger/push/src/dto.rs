// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! What is said to a push server and what it answers.
//!
//! The protocol belongs to the server (`services/push/spec/protocol.md`). These
//! types are the client's copy of it, and `tests/golden` is the client's
//! copy of the server's examples: the tests read them, so a drift between
//! the two shows up as a failed test and not on a user's phone.
//!
//! What the client reads is read loosely: a field or a value a newer server
//! adds must not break an older app.

use serde::{Deserialize, Serialize};

/// Which service carries pushes to the device, and the device's address there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum Channel {
    Fcm { token: String },
}

impl Channel {
    pub fn provider(&self) -> &'static str {
        match self {
            Self::Fcm { .. } => "fcm",
        }
    }

    pub fn token(&self) -> &str {
        match self {
            Self::Fcm { token } => token,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    pub dm: bool,
    pub groups: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { dm: true, groups: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayWatch {
    pub url: String,
    pub dm: bool,
    pub groups: bool,
}

/// A group to watch, and what tells its events from those of strangers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupWatch {
    /// 64 hex characters.
    pub id: String,
    /// Keys of the group's marks, one or two, 64 hex characters each: made
    /// from the group's keys, they open nothing. The server pushes an
    /// event of the group only when one of them made a mark on it.
    pub keys: Vec<String>,
}

/// Everything about the device, sent whole every time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePut {
    pub app_id: String,
    pub channel: Channel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    pub prefs: Prefs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_key: Option<String>,
    pub relays: Vec<RelayWatch>,
    /// The groups to watch.
    pub groups: Vec<GroupWatch>,
}

/// What the server does with a relay the device named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayStatus {
    Ok,
    Pending,
    NotAllowed,
    Invalid,
    Restricted,
    Unreachable,
    /// A word of a newer server.
    #[serde(other)]
    Unknown,
}

impl RelayStatus {
    /// Will pushes come for what arrives on this relay.
    pub fn watched(self) -> bool {
        matches!(self, Self::Ok | Self::Pending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayAnswer {
    pub url: String,
    pub status: RelayStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAnswer {
    pub device_id: String,
    pub expires_at: u64,
    #[serde(default)]
    pub relays: Vec<RelayAnswer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestAnswer {
    /// `delivered`, `dead_token`, `rejected`, `retry`.
    pub outcome: String,
    pub trace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
    #[serde(default)]
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub server_time: Option<u64>,
}

/// What a server says about itself before anything is sent to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    pub version: String,
    #[serde(default)]
    pub apps: Vec<AppInfo>,
    #[serde(default)]
    pub relays: Option<RelayPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInfo {
    pub id: String,
    #[serde(default)]
    pub providers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayPolicy {
    pub policy: String,
    #[serde(default)]
    pub allowed: Vec<String>,
}

impl Info {
    /// Does the server push to this app through this service.
    pub fn serves(&self, app_id: &str, provider: &str) -> bool {
        self.apps
            .iter()
            .any(|a| a.id == app_id && a.providers.iter().any(|p| p == provider))
    }
}
