// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

#[allow(dead_code)]
pub struct FingerprintPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub user_agent: &'static str,
    pub app_version: &'static str,
    pub oscpu: &'static str,
    pub platform: &'static str,
    pub languages: &'static str,
    pub locale: &'static str,
    pub screen_width: i64,
    pub screen_height: i64,
    pub default_webgl_vendor: &'static str,
    pub default_webgl_renderer: &'static str,
}

pub const PRESETS: &[FingerprintPreset] = &[
    FingerprintPreset {
        id: "win10",
        label: "Windows 10 + Firefox",
        user_agent:
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:127.0) Gecko/20100101 Firefox/127.0",
        app_version: "5.0 (Windows)",
        oscpu: "Windows NT 10.0; Win64; x64",
        platform: "Win32",
        languages: "en-US,en",
        locale: "en-US",
        screen_width: 1920,
        screen_height: 1080,
        default_webgl_vendor: "Intel",
        default_webgl_renderer: "Intel(R) UHD Graphics 630",
    },
    FingerprintPreset {
        id: "win11",
        label: "Windows 11 + Firefox",
        user_agent:
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:127.0) Gecko/20100101 Firefox/127.0",
        app_version: "5.0 (Windows)",
        oscpu: "Windows NT 10.0; Win64; x64",
        platform: "Win32",
        languages: "en-US,en",
        locale: "en-US",
        screen_width: 1920,
        screen_height: 1080,
        default_webgl_vendor: "Intel",
        default_webgl_renderer: "Intel(R) UHD Graphics 630",
    },
    FingerprintPreset {
        id: "macos",
        label: "macOS + Firefox",
        user_agent:
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:127.0) Gecko/20100101 Firefox/127.0",
        app_version: "5.0 (Macintosh)",
        oscpu: "Intel Mac OS X 14.5",
        platform: "MacIntel",
        languages: "en-US,en",
        locale: "en-US",
        screen_width: 2560,
        screen_height: 1600,
        default_webgl_vendor: "Apple",
        default_webgl_renderer: "Apple M1, or similar",
    },
    FingerprintPreset {
        id: "linux",
        label: "Linux + Firefox",
        user_agent: "Mozilla/5.0 (X11; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0",
        app_version: "5.0 (X11; Linux x86_64)",
        oscpu: "Linux x86_64",
        platform: "Linux x86_64",
        languages: "en-US,en",
        locale: "en-US",
        screen_width: 1920,
        screen_height: 1080,
        default_webgl_vendor: "",
        default_webgl_renderer: "",
    },
];

pub fn get_preset(id: &str) -> Option<&'static FingerprintPreset> {
    PRESETS.iter().find(|p| p.id == id)
}

#[derive(serde::Serialize)]
pub struct PresetInfo {
    pub id: &'static str,
    pub label: &'static str,
}

pub fn list_presets() -> Vec<PresetInfo> {
    PRESETS
        .iter()
        .map(|p| PresetInfo {
            id: p.id,
            label: p.label,
        })
        .collect()
}

#[tauri::command]
pub fn fingerprint_presets() -> Vec<PresetInfo> {
    list_presets()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_preset_known_ids() {
        let win10 = get_preset("win10").expect("win10 preset exists");
        assert_eq!(win10.platform, "Win32");
        assert!(win10.user_agent.contains("Windows NT 10.0"));
        assert_eq!((win10.screen_width, win10.screen_height), (1920, 1080));

        let macos = get_preset("macos").expect("macos preset exists");
        assert_eq!(macos.platform, "MacIntel");
        assert_eq!((macos.screen_width, macos.screen_height), (2560, 1600));

        let linux = get_preset("linux").expect("linux preset exists");
        assert_eq!(linux.platform, "Linux x86_64");
        // Linux preset deliberately ships empty WebGL defaults (native GL used)
        assert!(linux.default_webgl_vendor.is_empty());
    }

    #[test]
    fn get_preset_unknown_is_none() {
        assert!(get_preset("win95").is_none());
        assert!(get_preset("").is_none());
    }

    #[test]
    fn list_presets_matches_table() {
        let infos = list_presets();
        assert_eq!(infos.len(), PRESETS.len());
        assert!(infos.iter().any(|p| p.id == "win11"));
    }
}
