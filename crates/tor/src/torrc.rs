// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The torrc of one instance, written from the settings, its exit countries
//! and its paths by a pure function, and the `pt_config.json` of the bundle
//! it takes the pluggable transports and the built-in bridges from.
//!
//! Paths are written quoted: torrc reads a quoted value as a C string, so a
//! backslash of a Windows path and a space survive. The one exception is the
//! executable of a pluggable transport: tor splits `ClientTransportPlugin`
//! on spaces after it reads it, so that path must hold none. tor runs with
//! `<data>/tor/` as its working directory and the transports are named
//! relative to it (`bundle/tor/pluggable_transports/lyrebird`), as Tor
//! Browser names its own.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::manager::ExitSet;
use crate::settings::{BridgeMode, TorSettings, UpstreamKind};

/// The file of the bundle that lists the transports and the built-in bridges.
pub const PT_CONFIG: &str = "pt_config.json";

/// `pt_config.json` of the Tor Expert Bundle.
///
/// ```json
/// { "recommendedDefault": "obfs4",
///   "pluggableTransports": { "lyrebird": "ClientTransportPlugin meek_lite,obfs2,obfs3,obfs4,scramblesuit,webtunnel exec ${pt_path}lyrebird", "snowflake": "…", "conjure": "…" },
///   "bridges": { "meek": ["meek_lite 192.0.2.20:80 …"], "obfs4": ["obfs4 37.218.245.14:38224 …", …], "snowflake": ["snowflake 192.0.2.3:80 …", …] } }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PtConfig {
    pub recommended_default: String,
    /// The `ClientTransportPlugin` lines, `${pt_path}` standing for the
    /// directory of the transports.
    pub pluggable_transports: BTreeMap<String, String>,
    /// The built-in bridge lines by kind.
    pub bridges: BTreeMap<String, Vec<String>>,
}

impl PtConfig {
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("bad {PT_CONFIG}: {e}"))
    }

    /// `pt_config.json` in the directory of the transports.
    pub fn read(pt_dir: &Path) -> Result<Self, String> {
        let path = pt_dir.join(PT_CONFIG);
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&json)
    }

    /// The kinds of built-in bridges, sorted.
    pub fn builtin_kinds(&self) -> Vec<String> {
        self.bridges
            .iter()
            .filter(|(_, lines)| !lines.is_empty())
            .map(|(kind, _)| kind.clone())
            .collect()
    }
}

/// Where the files of one instance are, as torrc names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrcPaths {
    pub data_dir: PathBuf,
    pub control_port_file: PathBuf,
    pub cookie_file: PathBuf,
    pub geoip: PathBuf,
    pub geoip6: PathBuf,
    /// What `${pt_path}` of `pt_config.json` becomes: the directory of the
    /// transports with a trailing separator, relative to the working
    /// directory of tor ([`pt_path`]).
    pub pt_path: String,
}

/// The torrc of the instance for `exit`; the one with no exit countries
/// also opens the fixed external SOCKS port of the settings. `socks_port`:
/// the port consumers use, `None` for one tor picks (`auto`); an instance
/// started again keeps the port its leases name. `pt`: the transports of the
/// bundle, needed only with bridges. The settings are expected
/// [validated](TorSettings::validate); a value that would break a line is
/// refused here once more.
pub fn torrc(
    settings: &TorSettings,
    exit: &ExitSet,
    socks_port: Option<u16>,
    paths: &TorrcPaths,
    owner_pid: u32,
    pt: Option<&PtConfig>,
) -> Result<String, String> {
    let mut out = Vec::<String>::new();
    let mut put = |line: String| out.push(line);

    put(format!(
        "# Written by the app for the tor instance \"{}\" at each start; edits are lost.",
        exit.key()
    ));
    put(format!("DataDirectory {}", quote_path(&paths.data_dir)?));
    // Every consumer gets this port; their streams are kept apart by their
    // SOCKS credentials (IsolateSOCKSAuth is on by default).
    put(match socks_port {
        Some(port) => format!("SocksPort 127.0.0.1:{port}"),
        None => "SocksPort auto".into(),
    });
    if exit.is_any() {
        if let Some(port) = settings.external_socks_port {
            put(format!("SocksPort 127.0.0.1:{port}"));
        }
    }
    put("ControlPort auto".into());
    put(format!(
        "ControlPortWriteToFile {}",
        quote_path(&paths.control_port_file)?
    ));
    put("CookieAuthentication 1".into());
    put(format!("CookieAuthFile {}", quote_path(&paths.cookie_file)?));
    // tor exits when the app is gone, even if it was killed.
    put(format!("__OwningControllerProcess {owner_pid}"));
    put(format!("GeoIPFile {}", quote_path(&paths.geoip)?));
    put(format!("GeoIPv6File {}", quote_path(&paths.geoip6)?));
    put("ClientOnly 1".into());
    put("AvoidDiskWrites 1".into());
    put("Log notice stdout".into());

    if !exit.is_any() {
        put(format!("ExitNodes {}", countries(exit.codes())));
    }
    if !settings.exclude_countries.is_empty() {
        put(format!(
            "ExcludeNodes {}",
            countries(&settings.exclude_countries)
        ));
    }
    // StrictNodes concerns ExcludeNodes alone: ExitNodes are always kept to
    // for exit circuits (tor(1): "StrictNodes does not apply to
    // ExcludeExitNodes, ExitNodes, MiddleNodes, or MapAddress").
    if settings.strict_exclude && !settings.exclude_countries.is_empty() {
        put("StrictNodes 1".into());
    }

    let bridges: Vec<String> = match settings.bridges.mode {
        BridgeMode::None => Vec::new(),
        BridgeMode::Builtin => {
            let kind = &settings.bridges.builtin;
            let pt = pt.ok_or_else(|| format!("tor_no_transports: no {PT_CONFIG} in the bundle"))?;
            match pt.bridges.get(kind) {
                Some(lines) if !lines.is_empty() => lines.clone(),
                _ => {
                    return Err(format!(
                        "tor_no_builtin_bridges: the bundle has no built-in bridges of kind {kind:?}"
                    ))
                }
            }
        }
        BridgeMode::Custom => settings
            .bridges
            .lines
            .iter()
            .map(|line| {
                let line = line.trim();
                line.strip_prefix("Bridge ")
                    .or_else(|| line.strip_prefix("bridge "))
                    .unwrap_or(line)
                    .trim()
                    .to_string()
            })
            .filter(|line| !line.is_empty())
            .collect(),
    };
    if !bridges.is_empty() {
        put("UseBridges 1".into());
        // The transports are started by tor only when a bridge needs them,
        // so all of them can be named. A line that is no bridge of a
        // transport (`1.2.3.4:443 FP`) needs none.
        if let Some(pt) = pt {
            for line in pt.pluggable_transports.values() {
                put(one_line(&line.replace("${pt_path}", &paths.pt_path))?);
            }
        } else if bridges.iter().any(|b| !starts_with_address(b)) {
            return Err(format!("tor_no_transports: no {PT_CONFIG} in the bundle"));
        }
        for line in bridges {
            put(format!("Bridge {}", one_line(&line)?));
        }
    }

    if let Some(up) = &settings.upstream {
        let address = host_port(&up.host, up.port);
        one_line(&address)?;
        match up.kind {
            UpstreamKind::Socks5 => {
                put(format!("Socks5Proxy {address}"));
                if !up.username.is_empty() {
                    put(format!("Socks5ProxyUsername {}", quote(&up.username)?));
                    put(format!("Socks5ProxyPassword {}", quote(&up.password)?));
                }
            }
            UpstreamKind::Https => {
                put(format!("HTTPSProxy {address}"));
                if !up.username.is_empty() || !up.password.is_empty() {
                    put(format!(
                        "HTTPSProxyAuthenticator {}",
                        quote(&format!("{}:{}", up.username, up.password))?
                    ));
                }
            }
        }
    }

    if !settings.reachable_ports.is_empty() {
        let list: Vec<String> = settings
            .reachable_ports
            .iter()
            .map(|p| format!("*:{p}"))
            .collect();
        put(format!("ReachableAddresses {}", list.join(",")));
    }

    for line in &settings.extra_torrc {
        put(one_line(line.trim())?);
    }

    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// `{de},{nl}`.
fn countries(codes: &[String]) -> String {
    codes
        .iter()
        .map(|c| format!("{{{c}}}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// A bridge line without a transport starts with its address.
fn starts_with_address(line: &str) -> bool {
    line.starts_with(|c: char| c.is_ascii_digit() || c == '[')
}

/// `host:port`; an IPv6 address in brackets.
fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn one_line(value: &str) -> Result<String, String> {
    if value.contains(['\n', '\r', '\0']) {
        Err("tor_invalid_settings: a line break or NUL in a torrc value".into())
    } else {
        Ok(value.to_string())
    }
}

/// A value as a quoted torrc string: tor unescapes `\\` and `\"` in it.
pub fn quote(value: &str) -> Result<String, String> {
    one_line(value)?;
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

pub fn quote_path(path: &Path) -> Result<String, String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("the path {} is not valid Unicode", path.display()))?;
    quote(text)
}

/// `path` relative to `base` when it lies inside it.
fn relative_to(path: &Path, base: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(base).ok()?;
    let clean = rest
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    clean.then(|| rest.to_path_buf())
}

/// How torrc names `path` when tor runs in `cwd`. Absolute as a rule. With
/// `relative_when_not_ascii`, a path with other than ASCII in it that lies
/// inside `cwd` is written relative to it: tor on Windows opens files
/// through the ANSI code page, which the UTF-8 bytes of a torrc would not
/// match, while it resolves a relative path itself (with a warning in its
/// log).
pub fn torrc_path(path: &Path, cwd: &Path, relative_when_not_ascii: bool) -> PathBuf {
    let ascii = path.to_str().is_some_and(|p| p.is_ascii());
    if relative_when_not_ascii && !ascii {
        if let Some(rel) = relative_to(path, cwd) {
            return rel;
        }
    }
    path.to_path_buf()
}

/// What `${pt_path}` becomes: the directory of the transports relative to
/// the working directory of tor if it lies inside it, else absolute; with a
/// trailing separator. It goes unquoted into a line tor splits on spaces, so
/// a space, a quote or a `#` in it is refused.
pub fn pt_path(pt_dir: &Path, cwd: &Path) -> Result<String, String> {
    let path = relative_to(pt_dir, cwd).unwrap_or_else(|| pt_dir.to_path_buf());
    let mut text = path
        .to_str()
        .ok_or_else(|| format!("the path {} is not valid Unicode", path.display()))?
        .to_string();
    if text.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '#') {
        return Err(format!(
            "tor_bad_transport_path: the path of the pluggable transports holds a space or a quote: {text}"
        ));
    }
    if !text.is_empty() && !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Upstream, UpstreamKind};

    /// The `pt_config.json` of the 15.0.24 bundle for Linux, with one bridge
    /// line of each kind.
    const PT_JSON: &str = r#"{
  "recommendedDefault" : "obfs4",
  "pluggableTransports" : {
    "lyrebird" : "ClientTransportPlugin meek_lite,obfs2,obfs3,obfs4,scramblesuit,webtunnel exec ${pt_path}lyrebird",
    "snowflake": "ClientTransportPlugin snowflake exec ${pt_path}lyrebird",
    "conjure" : "ClientTransportPlugin conjure exec ${pt_path}conjure-client -registerURL https://registration.refraction.network/api"
  },
  "bridges" : {
    "meek" : [
      "meek_lite 192.0.2.20:80 url=https://1603026938.rsc.cdn77.org front=www.phpmyadmin.net utls=HelloRandomizedALPN"
    ],
    "obfs4" : [
      "obfs4 37.218.245.14:38224 D9A82D2F9C2F65A18407B1D2B764F130847F8B5D cert=bjRaMrr1BRiAW8IE9U5z27fQaYgOhX1UCmOpg2pFpoMvo6ZgQMzLsaTzzQNTlm7hNcb+Sg iat-mode=0"
    ],
    "snowflake" : [
      "snowflake 192.0.2.3:80 2B280B23E1107BB62ABFC40DDCC8824814F80A72 fingerprint=2B280B23E1107BB62ABFC40DDCC8824814F80A72 url=https://1098762253.rsc.cdn77.org/"
    ]
  }
}"#;

    fn paths() -> TorrcPaths {
        TorrcPaths {
            data_dir: "/d/tor/instances/any/data".into(),
            control_port_file: "/d/tor/instances/any/control.port".into(),
            cookie_file: "/d/tor/instances/any/control.cookie".into(),
            geoip: "/d/tor/bundle/data/geoip".into(),
            geoip6: "/d/tor/bundle/data/geoip6".into(),
            pt_path: "bundle/tor/pluggable_transports/".into(),
        }
    }

    const HEAD: &str = r#"DataDirectory "/d/tor/instances/any/data"
SocksPort auto
ControlPort auto
ControlPortWriteToFile "/d/tor/instances/any/control.port"
CookieAuthentication 1
CookieAuthFile "/d/tor/instances/any/control.cookie"
__OwningControllerProcess 4242
GeoIPFile "/d/tor/bundle/data/geoip"
GeoIPv6File "/d/tor/bundle/data/geoip6"
ClientOnly 1
AvoidDiskWrites 1
Log notice stdout
"#;

    fn body(text: &str) -> &str {
        // Past the comment line.
        text.split_once('\n').unwrap().1
    }

    #[test]
    fn the_plain_instance() {
        let pt = PtConfig::parse(PT_JSON).unwrap();
        let text = torrc(&TorSettings::defaults(), &ExitSet::any(), None, &paths(), 4242, Some(&pt)).unwrap();
        assert!(text.starts_with("# Written by the app for the tor instance \"any\""));
        assert_eq!(body(&text), HEAD);
    }

    #[test]
    fn exit_countries_exclusions_and_the_external_port() {
        let mut s = TorSettings::defaults();
        s.external_socks_port = Some(9150);
        s.exclude_countries = vec!["by".into(), "ru".into()];
        s.strict_exclude = true;
        s.reachable_ports = vec![80, 443];
        s.extra_torrc = vec!["NumEntryGuards 2".into()];

        let any = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap();
        assert_eq!(
            body(&any),
            HEAD.replace("SocksPort auto\n", "SocksPort auto\nSocksPort 127.0.0.1:9150\n")
                + "ExcludeNodes {by},{ru}\nStrictNodes 1\nReachableAddresses *:80,*:443\nNumEntryGuards 2\n"
        );

        // A port kept from the last start.
        let again = torrc(&s, &ExitSet::any(), Some(41234), &paths(), 4242, None).unwrap();
        assert_eq!(
            body(&again),
            body(&any).replace("SocksPort auto\n", "SocksPort 127.0.0.1:41234\n")
        );

        // The external port is the default instance's alone.
        let de_nl = ExitSet::parse("NL, de").unwrap();
        let text = torrc(&s, &de_nl, None, &paths(), 4242, None).unwrap();
        assert!(text.starts_with("# Written by the app for the tor instance \"de-nl\""));
        assert_eq!(
            body(&text),
            HEAD.to_string()
                + "ExitNodes {de},{nl}\nExcludeNodes {by},{ru}\nStrictNodes 1\nReachableAddresses *:80,*:443\nNumEntryGuards 2\n"
        );

        // Exit countries alone do not make the exclusions strict.
        s.strict_exclude = false;
        let text = torrc(&s, &de_nl, None, &paths(), 4242, None).unwrap();
        assert!(!text.contains("StrictNodes"));
    }

    #[test]
    fn built_in_and_custom_bridges() {
        let pt = PtConfig::parse(PT_JSON).unwrap();
        let mut s = TorSettings::defaults();
        s.bridges.mode = BridgeMode::Builtin;
        s.bridges.builtin = "snowflake".into();
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, Some(&pt)).unwrap();
        let plugins = "UseBridges 1\n\
ClientTransportPlugin conjure exec bundle/tor/pluggable_transports/conjure-client -registerURL https://registration.refraction.network/api\n\
ClientTransportPlugin meek_lite,obfs2,obfs3,obfs4,scramblesuit,webtunnel exec bundle/tor/pluggable_transports/lyrebird\n\
ClientTransportPlugin snowflake exec bundle/tor/pluggable_transports/lyrebird\n";
        assert_eq!(
            body(&text),
            format!(
                "{HEAD}{plugins}Bridge snowflake 192.0.2.3:80 2B280B23E1107BB62ABFC40DDCC8824814F80A72 fingerprint=2B280B23E1107BB62ABFC40DDCC8824814F80A72 url=https://1098762253.rsc.cdn77.org/\n"
            )
        );

        s.bridges.builtin = "webtunnel".into();
        let err = torrc(&s, &ExitSet::any(), None, &paths(), 4242, Some(&pt)).unwrap_err();
        assert!(err.starts_with("tor_no_builtin_bridges"), "{err}");
        let err = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap_err();
        assert!(err.starts_with("tor_no_transports"), "{err}");

        s.bridges.mode = BridgeMode::Custom;
        s.bridges.lines = vec![
            "Bridge obfs4 1.2.3.4:443 AB cert=c iat-mode=0".into(),
            " 5.6.7.8:9001 CD ".into(),
        ];
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, Some(&pt)).unwrap();
        assert_eq!(
            body(&text),
            format!("{HEAD}{plugins}Bridge obfs4 1.2.3.4:443 AB cert=c iat-mode=0\nBridge 5.6.7.8:9001 CD\n")
        );
        // Plain bridges need no transports.
        s.bridges.lines = vec!["5.6.7.8:9001 CD".into()];
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap();
        assert_eq!(body(&text), format!("{HEAD}UseBridges 1\nBridge 5.6.7.8:9001 CD\n"));
    }

    #[test]
    fn the_upstream_proxy() {
        let mut s = TorSettings::defaults();
        s.upstream = Some(Upstream {
            kind: UpstreamKind::Socks5,
            host: "10.0.0.1".into(),
            port: 1080,
            username: "me".into(),
            password: r#"p"a\ss #1"#.into(),
        });
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap();
        assert_eq!(
            body(&text),
            format!(
                "{HEAD}Socks5Proxy 10.0.0.1:1080\nSocks5ProxyUsername \"me\"\nSocks5ProxyPassword \"p\\\"a\\\\ss #1\"\n"
            )
        );

        s.upstream = Some(Upstream {
            kind: UpstreamKind::Https,
            host: "::1".into(),
            port: 3128,
            username: "u".into(),
            password: "p:w".into(),
        });
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap();
        assert_eq!(
            body(&text),
            format!("{HEAD}HTTPSProxy [::1]:3128\nHTTPSProxyAuthenticator \"u:p:w\"\n")
        );

        s.upstream.as_mut().unwrap().username.clear();
        s.upstream.as_mut().unwrap().password.clear();
        let text = torrc(&s, &ExitSet::any(), None, &paths(), 4242, None).unwrap();
        assert_eq!(body(&text), format!("{HEAD}HTTPSProxy [::1]:3128\n"));
    }

    #[test]
    fn nothing_breaks_a_line() {
        let mut s = TorSettings::defaults();
        s.extra_torrc = vec!["A 1\nControlPort 9051".into()];
        assert!(torrc(&s, &ExitSet::any(), None, &paths(), 1, None).is_err());
        let mut p = paths();
        p.data_dir = "/d/a\nb".into();
        assert!(torrc(&TorSettings::defaults(), &ExitSet::any(), None, &p, 1, None).is_err());
    }

    #[test]
    fn paths_are_quoted_for_torrc() {
        assert_eq!(quote_path(Path::new("/home/a b/x")).unwrap(), r#""/home/a b/x""#);
        assert_eq!(
            quote(r"C:\Users\Jane Doe\AppData\Roaming\net.veydan\tor").unwrap(),
            r#""C:\\Users\\Jane Doe\\AppData\\Roaming\\net.veydan\\tor""#
        );
        assert_eq!(quote(r#"a"b"#).unwrap(), r#""a\"b""#);
        assert_eq!(quote("Дмитрий").unwrap(), "\"Дмитрий\"");
        assert!(quote("a\rb").is_err());
    }

    #[test]
    fn paths_relative_to_the_working_directory() {
        let cwd = Path::new("/home/Дмитрий/data/tor");
        let inside = cwd.join("instances").join("de").join("data");
        assert_eq!(
            torrc_path(&inside, cwd, true),
            Path::new("instances").join("de").join("data")
        );
        assert_eq!(torrc_path(&inside, cwd, false), inside);
        let ascii = Path::new("/home/dima/data/tor/instances/any/data");
        assert_eq!(torrc_path(ascii, Path::new("/home/dima/data/tor"), true), ascii);
        // Outside the working directory it stays absolute.
        let outside = Path::new("/opt/Дмитрий/geoip");
        assert_eq!(torrc_path(outside, cwd, true), outside);

        let sep = std::path::MAIN_SEPARATOR;
        let pt = cwd.join("bundle").join("tor").join("pluggable_transports");
        assert_eq!(
            pt_path(&pt, cwd).unwrap(),
            format!("bundle{sep}tor{sep}pluggable_transports{sep}")
        );
        assert_eq!(
            pt_path(Path::new("/opt/pt"), cwd).unwrap(),
            format!("/opt/pt{sep}")
        );
        assert!(pt_path(Path::new("/opt/my pt"), cwd)
            .unwrap_err()
            .starts_with("tor_bad_transport_path"));
    }

    #[test]
    fn the_kinds_of_built_in_bridges() {
        let pt = PtConfig::parse(PT_JSON).unwrap();
        assert_eq!(pt.builtin_kinds(), vec!["meek", "obfs4", "snowflake"]);
        assert_eq!(pt.recommended_default, "obfs4");
        assert_eq!(pt.pluggable_transports.len(), 3);
        assert!(PtConfig::parse(r#"{"bridges": 1}"#).is_err());
        assert_eq!(PtConfig::parse("{}").unwrap(), PtConfig::default());
    }
}
