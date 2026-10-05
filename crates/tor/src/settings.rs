// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The settings of tor on this device: bridges, the proxy tor itself goes
//! out through, the ports a firewall lets through, the countries no circuit
//! may cross, when an instance starts and stops, and the user's own lines.
//!
//! They are one JSON value under [`SETTINGS_KEY`] in `app_settings`. The key
//! is local: the module registers nothing for sync, because bridges, a proxy
//! and a firewall belong to the network this device is on.
//!
//! A value written by another version still loads: missing fields take
//! their defaults, unknown fields are ignored, and a field this version
//! cannot read falls back to its default alone ([`TorSettings::from_stored`]).
//! What the user writes is held to [`TorSettings::validate`] first.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The key of the settings in `app_settings`.
pub const SETTINGS_KEY: &str = "tor_settings";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorSettings {
    pub bridges: Bridges,
    /// The proxy tor connects through, if the network allows nothing else.
    pub upstream: Option<Upstream>,
    /// The only ports tor may connect to (`ReachableAddresses`); empty: any.
    pub reachable_ports: Vec<u16>,
    /// No circuit goes through a relay of these countries (`ExcludeNodes`).
    pub exclude_countries: Vec<String>,
    /// `StrictNodes 1`: the exclusion holds even where it breaks things.
    pub strict_exclude: bool,
    /// The instance with no exit countries starts with the app and does not
    /// stop when idle.
    pub start_with_app: bool,
    /// How long an instance nobody uses keeps running; 0: it stops at once.
    pub idle_minutes: u32,
    /// A fixed SOCKS port on 127.0.0.1 of the instance with no exit
    /// countries, for programs outside the app.
    pub external_socks_port: Option<u16>,
    /// The user's own torrc lines, written last.
    pub extra_torrc: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bridges {
    pub mode: BridgeMode,
    /// The kind of the bridges the bundle carries (`pt_config.json`):
    /// `obfs4`, `snowflake`, `meek` in 15.0.
    pub builtin: String,
    /// The user's own bridge lines, `obfs4 1.2.3.4:443 FINGERPRINT cert=…`;
    /// a leading `Bridge ` is allowed.
    pub lines: Vec<String>,
}

impl Default for Bridges {
    fn default() -> Self {
        Self {
            mode: BridgeMode::None,
            builtin: "obfs4".to_string(),
            lines: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeMode {
    #[default]
    None,
    Builtin,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upstream {
    pub kind: UpstreamKind,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamKind {
    Socks5,
    Https,
}

/// The settings of a device that has none stored; also what a field missing
/// from a stored value takes.
impl Default for TorSettings {
    fn default() -> Self {
        Self {
            bridges: Bridges::default(),
            upstream: None,
            reachable_ports: Vec::new(),
            exclude_countries: Vec::new(),
            strict_exclude: false,
            start_with_app: false,
            idle_minutes: 10,
            external_socks_port: None,
            extra_torrc: Vec::new(),
        }
    }
}

impl TorSettings {
    /// The settings of a device that has none stored.
    pub fn defaults() -> Self {
        Self::default()
    }

    /// The stored value, as far as this version can read it; the defaults
    /// when nothing is stored or it is no JSON object at all.
    pub fn from_stored(json: Option<&str>) -> Self {
        let Some(stored) = json.and_then(|j| serde_json::from_str::<Value>(j).ok()) else {
            return Self::defaults();
        };
        let mut merged = serde_json::to_value(Self::defaults()).unwrap_or(Value::Null);
        merge_lenient(&mut merged, &stored, &mut Vec::new());
        serde_json::from_value(merged).unwrap_or_else(|_| Self::defaults())
    }

    /// Holds what the user wrote to the rules torrc needs, and returns it
    /// tidied: country codes in lower case, lines trimmed, empty lines and
    /// repeated ports and countries dropped.
    pub fn validate(mut self) -> Result<Self, String> {
        for line in self.bridges.lines.iter().chain(&self.extra_torrc) {
            one_line(line)?;
        }
        self.bridges.lines = tidy_lines(&self.bridges.lines);
        self.extra_torrc = tidy_lines(&self.extra_torrc);
        let builtin = self.bridges.builtin.trim().to_string();
        if !builtin
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(format!("tor_invalid_settings: bad bridge kind {builtin:?}"));
        }
        self.bridges.builtin = builtin;
        match self.bridges.mode {
            BridgeMode::Builtin if self.bridges.builtin.is_empty() => {
                return Err("tor_invalid_settings: no kind of built-in bridges chosen".into())
            }
            BridgeMode::Custom if self.bridges.lines.is_empty() => {
                return Err("tor_invalid_settings: no bridge lines".into())
            }
            _ => {}
        }

        if let Some(up) = &mut self.upstream {
            up.host = up.host.trim().to_string();
            if up.host.is_empty() || up.host.chars().any(|c| c.is_whitespace() || c.is_control())
            {
                return Err(format!("tor_invalid_settings: bad proxy host {:?}", up.host));
            }
            if up.port == 0 {
                return Err("tor_invalid_settings: the proxy port is 0".into());
            }
            for value in [&up.username, &up.password] {
                one_line(value)?;
                if value.len() > 255 {
                    return Err("tor_invalid_settings: proxy credentials over 255 bytes".into());
                }
            }
            match up.kind {
                // tor wants both or neither.
                UpstreamKind::Socks5 if up.username.is_empty() != up.password.is_empty() => {
                    return Err(
                        "tor_invalid_settings: a SOCKS5 proxy needs both the username and the password, or neither"
                            .into(),
                    )
                }
                // `HTTPSProxyAuthenticator user:pass`.
                UpstreamKind::Https if up.username.contains(':') => {
                    return Err("tor_invalid_settings: the proxy username holds ':'".into())
                }
                _ => {}
            }
        }

        if self.reachable_ports.contains(&0) || self.external_socks_port == Some(0) {
            return Err("tor_invalid_settings: port 0".into());
        }
        self.reachable_ports.sort_unstable();
        self.reachable_ports.dedup();

        let mut countries = Vec::new();
        for code in &self.exclude_countries {
            countries.push(country(code)?);
        }
        countries.sort();
        countries.dedup();
        self.exclude_countries = countries;
        Ok(self)
    }
}

/// A country code as torrc writes it: two ASCII letters, lower case.
pub fn country(code: &str) -> Result<String, String> {
    let code = code.trim();
    if code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()) {
        Ok(code.to_ascii_lowercase())
    } else {
        Err(format!("tor_invalid_country: {code:?} is not a two-letter country code"))
    }
}

/// A value that goes onto one line of torrc.
fn one_line(value: &str) -> Result<(), String> {
    if value.contains(['\n', '\r', '\0']) {
        Err("tor_invalid_settings: a line break or NUL in a value".into())
    } else {
        Ok(())
    }
}

fn tidy_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Takes into `base` (the defaults, as JSON) every field of `stored` that
/// still leaves a value [`TorSettings`] reads; an object is taken field by
/// field, so one unreadable field costs only itself.
fn merge_lenient(base: &mut Value, stored: &Value, path: &mut Vec<String>) {
    let Value::Object(fields) = stored else {
        return;
    };
    for (name, value) in fields {
        path.push(name.clone());
        let Some(slot) = pointer_mut(base, path) else {
            path.pop();
            continue; // unknown here
        };
        let before = std::mem::replace(slot, value.clone());
        if serde_json::from_value::<TorSettings>(base.clone()).is_err() {
            // Not readable as a whole: go into it if both are objects.
            let slot = pointer_mut(base, path).expect("the slot is still there");
            *slot = before;
            if value.is_object() && slot.is_object() {
                merge_lenient(base, value, path);
            }
        }
        path.pop();
    }
}

fn pointer_mut<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut at = value;
    for part in path {
        at = at.as_object_mut()?.get_mut(part)?;
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults() {
        let s = TorSettings::defaults();
        assert_eq!(s.idle_minutes, 10);
        assert!(!s.start_with_app);
        assert_eq!(s.bridges.mode, BridgeMode::None);
        assert_eq!(s.bridges.builtin, "obfs4");
        assert_eq!(s.external_socks_port, None);
        assert_eq!(TorSettings::from_stored(None), s);
        assert_eq!(TorSettings::from_stored(Some("not json")), s);
        assert_eq!(TorSettings::from_stored(Some("[1,2]")), s);
        assert_eq!(TorSettings::from_stored(Some("{}")), s);
    }

    #[test]
    fn the_json_the_ui_reads() {
        let json = serde_json::to_value(TorSettings::defaults()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "bridges": { "mode": "none", "builtin": "obfs4", "lines": [] },
                "upstream": null,
                "reachable_ports": [],
                "exclude_countries": [],
                "strict_exclude": false,
                "start_with_app": false,
                "idle_minutes": 10,
                "external_socks_port": null,
                "extra_torrc": []
            })
        );
    }

    #[test]
    fn a_value_of_another_version_loads_field_by_field() {
        // From the future: a new field, a bridge mode and a proxy kind this
        // version does not know, a field of another type.
        let stored = r#"{
            "bridges": { "mode": "magic", "builtin": "webtunnel", "lines": ["obfs4 1.2.3.4:443 AB cert=x iat-mode=0"] },
            "upstream": { "kind": "socks4", "host": "h", "port": 1 },
            "reachable_ports": [443],
            "exclude_countries": "ru",
            "start_with_app": true,
            "idle_minutes": 3,
            "new_field": { "x": 1 }
        }"#;
        let s = TorSettings::from_stored(Some(stored));
        assert_eq!(s.bridges.mode, BridgeMode::None);
        assert_eq!(s.bridges.builtin, "webtunnel");
        assert_eq!(s.bridges.lines.len(), 1);
        assert_eq!(s.upstream, None);
        assert_eq!(s.reachable_ports, vec![443]);
        assert!(s.exclude_countries.is_empty());
        assert!(s.start_with_app);
        assert_eq!(s.idle_minutes, 3);

        // From the past: fields missing.
        let s = TorSettings::from_stored(Some(r#"{"strict_exclude": true}"#));
        assert!(s.strict_exclude);
        assert_eq!(s.idle_minutes, 10);

        // The value written by this version reads back as it was.
        let mut full = TorSettings::defaults();
        full.upstream = Some(Upstream {
            kind: UpstreamKind::Https,
            host: "proxy.lan".into(),
            port: 3128,
            username: "u".into(),
            password: "p".into(),
        });
        full.external_socks_port = Some(9150);
        let json = serde_json::to_string(&full).unwrap();
        assert_eq!(TorSettings::from_stored(Some(&json)), full);
    }

    #[test]
    fn what_the_user_writes_is_checked_and_tidied() {
        let mut s = TorSettings::defaults();
        s.exclude_countries = vec!["RU".into(), " by ".into(), "ru".into()];
        s.reachable_ports = vec![443, 80, 443];
        s.extra_torrc = vec!["  ".into(), " NumEntryGuards 2 ".into()];
        let s = s.validate().unwrap();
        assert_eq!(s.exclude_countries, vec!["by", "ru"]);
        assert_eq!(s.reachable_ports, vec![80, 443]);
        assert_eq!(s.extra_torrc, vec!["NumEntryGuards 2"]);

        let bad = |f: &dyn Fn(&mut TorSettings)| {
            let mut s = TorSettings::defaults();
            f(&mut s);
            s.validate().unwrap_err()
        };
        assert!(bad(&|s| s.exclude_countries = vec!["rus".into()]).contains("tor_invalid_country"));
        assert!(bad(&|s| s.exclude_countries = vec!["r1".into()]).contains("tor_invalid_country"));
        assert!(bad(&|s| s.reachable_ports = vec![0]).contains("port 0"));
        assert!(bad(&|s| s.external_socks_port = Some(0)).contains("port 0"));
        assert!(bad(&|s| s.extra_torrc = vec!["A 1\nControlPort 9051".into()]).contains("line break"));
        assert!(bad(&|s| s.extra_torrc = vec!["A\0".into()]).contains("NUL"));
        assert!(bad(&|s| {
            s.bridges.mode = BridgeMode::Custom;
            s.bridges.lines = vec!["obfs4 x\r".into()];
        })
        .contains("line break"));
        assert!(bad(&|s| s.bridges.mode = BridgeMode::Custom).contains("no bridge lines"));
        assert!(bad(&|s| {
            s.bridges.mode = BridgeMode::Builtin;
            s.bridges.builtin = String::new();
        })
        .contains("no kind"));
        assert!(bad(&|s| s.bridges.builtin = "ob fs4".into()).contains("bridge kind"));
        let up = |kind, host: &str, port, user: &str, pass: &str| {
            Some(Upstream {
                kind,
                host: host.into(),
                port,
                username: user.into(),
                password: pass.into(),
            })
        };
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Socks5, "a b", 1, "", "")).contains("host"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Socks5, " ", 1, "", "")).contains("host"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Socks5, "h", 0, "", "")).contains("port is 0"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Socks5, "h", 1, "u", "")).contains("both"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Https, "h", 1, "a:b", "p")).contains("':'"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Https, "h", 1, "u", "p\n")).contains("line break"));
        assert!(bad(&|s| s.upstream = up(UpstreamKind::Socks5, "h", 1, &"u".repeat(256), "p")).contains("255"));

        let mut s = TorSettings::defaults();
        s.upstream = up(UpstreamKind::Https, " [::1] ", 8080, "", "");
        assert_eq!(s.validate().unwrap().upstream.unwrap().host, "[::1]");
    }
}
