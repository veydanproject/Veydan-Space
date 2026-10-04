// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Bilingual demo catalog (EN / RU) of the modules of this crate. Same
//! entity ids; locale picks labels. The TOTP entries and passwords are
//! pass's (`veydan_pass`), the notes, folders and tags notes' (`veydan_notes`):
//! a capture rule names a folder of notes by its id.

#[derive(Clone)]
pub struct DemoPack {
    pub default_workspace_name: &'static str,
    pub default_columns: Vec<DemoColumn>,
    pub workspaces: Vec<DemoWorkspace>,
    pub proxies: Vec<DemoProxy>,
    pub profiles: Vec<DemoProfile>,
    #[cfg(desktop)]
    pub ssh_keys: Vec<DemoSshKey>,
    #[cfg(desktop)]
    pub ssh_connections: Vec<DemoSshConn>,
    #[cfg(desktop)]
    pub capture_rules: Vec<DemoCapture>,
}

#[derive(Clone)]
pub struct DemoColumn {
    pub name: &'static str,
    pub tag_name: &'static str,
    pub color: &'static str,
}

#[derive(Clone)]
pub struct DemoWorkspace {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub color: &'static str,
    pub icon: &'static str,
    pub columns: Vec<DemoColumn>,
}

#[derive(Clone)]
pub struct DemoProxy {
    pub id: &'static str,
    pub name: &'static str,
    pub proxy_type: &'static str,
    pub host: &'static str,
    pub port: i64,
    pub username: Option<&'static str>,
    pub password: Option<&'static str>,
    pub country: Option<&'static str>,
    pub city: Option<&'static str>,
    pub tags: &'static [&'static str],
}

#[derive(Clone)]
pub struct DemoProfile {
    pub id: &'static str,
    pub name: &'static str,
    pub workspace_id: &'static str,
    pub proxy_id: Option<&'static str>,
    pub fingerprint_preset: &'static str,
    pub locale: &'static str,
    pub languages: &'static str,
    pub timezone: Option<&'static str>,
    pub notes: Option<&'static str>,
    pub tags: &'static [&'static str],
    pub kanban_status: &'static str,
    pub kanban_order: i64,
}

#[cfg(desktop)]
#[derive(Clone)]
pub struct DemoSshKey {
    pub id: &'static str,
    pub name: &'static str,
    pub algorithm: &'static str,
}

#[cfg(desktop)]
#[derive(Clone)]
pub struct DemoSshConn {
    pub id: &'static str,
    pub name: &'static str,
    pub host: &'static str,
    pub port: i64,
    pub username: &'static str,
    pub auth_type: &'static str,
    pub password: Option<&'static str>,
    pub ssh_key_id: Option<&'static str>,
    pub requires_2fa: bool,
    pub totp_entry_id: Option<&'static str>,
    pub proxy_id: Option<&'static str>,
    pub workspace_ids: &'static [&'static str],
    pub profile_ids: &'static [&'static str],
}

#[cfg(desktop)]
#[derive(Clone)]
pub struct DemoCapture {
    pub domain: &'static str,
    pub folder_id: &'static str,
    pub tags: &'static [&'static str],
}

fn t(ru: bool, en: &'static str, ru_s: &'static str) -> &'static str {
    if ru {
        ru_s
    } else {
        en
    }
}

/// Build the demo pack for `ru` or `en` (anything else → en).
pub fn pack(locale: &str) -> DemoPack {
    let ru = locale.eq_ignore_ascii_case("ru");
    DemoPack {
        default_workspace_name: t(ru, "Personal", "Личное"),
        default_columns: vec![
            DemoColumn {
                name: t(ru, "Inbox", "Inbox"),
                tag_name: "inbox",
                color: "#94a3b8",
            },
            DemoColumn {
                name: t(ru, "Active", "В работе"),
                tag_name: "active",
                color: "#3b82f6",
            },
            DemoColumn {
                name: t(ru, "Done", "Готово"),
                tag_name: "done",
                color: "#22c55e",
            },
        ],
        workspaces: workspaces(ru),
        proxies: proxies(),
        profiles: profiles(ru),
        #[cfg(desktop)]
        ssh_keys: ssh_keys(),
        #[cfg(desktop)]
        ssh_connections: ssh_connections(),
        #[cfg(desktop)]
        capture_rules: capture_rules(),
    }
}

fn workspaces(ru: bool) -> Vec<DemoWorkspace> {
    vec![
        DemoWorkspace {
            id: "demo-ws-smm",
            name: "SMM",
            description: t(ru, "Social media accounts", "Аккаунты соцсетей"),
            color: "#ec4899",
            icon: "megaphone",
            columns: vec![
                DemoColumn {
                    name: t(ru, "Ideas", "Идеи"),
                    tag_name: "ideas",
                    color: "#f9a8d4",
                },
                DemoColumn {
                    name: t(ru, "In progress", "В работе"),
                    tag_name: "in_progress",
                    color: "#ec4899",
                },
                DemoColumn {
                    name: t(ru, "Published", "Опубликовано"),
                    tag_name: "published",
                    color: "#22c55e",
                },
            ],
        },
        DemoWorkspace {
            id: "demo-ws-dev",
            name: t(ru, "Development", "Разработка"),
            description: t(ru, "Product and engineering", "Продукт и разработка"),
            color: "#3b82f6",
            icon: "code",
            columns: vec![
                DemoColumn {
                    name: "Backlog",
                    tag_name: "backlog",
                    color: "#94a3b8",
                },
                DemoColumn {
                    name: "Coding",
                    tag_name: "coding",
                    color: "#3b82f6",
                },
                DemoColumn {
                    name: "Review",
                    tag_name: "review",
                    color: "#f59e0b",
                },
                DemoColumn {
                    name: "Done",
                    tag_name: "done_dev",
                    color: "#22c55e",
                },
            ],
        },
        DemoWorkspace {
            id: "demo-ws-devops",
            name: "DevOps",
            description: t(ru, "Infrastructure and releases", "Инфраструктура и релизы"),
            color: "#10b981",
            icon: "server",
            columns: vec![
                DemoColumn {
                    name: "Plan",
                    tag_name: "plan",
                    color: "#94a3b8",
                },
                DemoColumn {
                    name: "Deploy",
                    tag_name: "deploy",
                    color: "#10b981",
                },
                DemoColumn {
                    name: "Monitor",
                    tag_name: "monitor",
                    color: "#06b6d4",
                },
            ],
        },
        DemoWorkspace {
            id: "demo-ws-biz",
            name: t(ru, "Business", "Бизнес"),
            description: t(ru, "Deals and partners", "Сделки и партнёры"),
            color: "#f59e0b",
            icon: "briefcase",
            columns: vec![
                DemoColumn {
                    name: "Lead",
                    tag_name: "lead",
                    color: "#94a3b8",
                },
                DemoColumn {
                    name: t(ru, "Negotiation", "Переговоры"),
                    tag_name: "negotiation",
                    color: "#f59e0b",
                },
                DemoColumn {
                    name: "Closed",
                    tag_name: "closed",
                    color: "#22c55e",
                },
            ],
        },
    ]
}

fn proxies() -> Vec<DemoProxy> {
    vec![
        DemoProxy {
            id: "demo-px-de",
            name: "DE Residential SOCKS5",
            proxy_type: "socks5",
            host: "proxy-de.example",
            port: 1080,
            username: Some("demo-de"),
            password: Some("demo-pass-de"),
            country: Some("DE"),
            city: Some("Berlin"),
            tags: &["workspace:demo-ws-devops"],
        },
        DemoProxy {
            id: "demo-px-us",
            name: "US Datacenter HTTP",
            proxy_type: "http",
            host: "proxy-us.example",
            port: 8080,
            username: Some("demo-us"),
            password: Some("demo-pass-us"),
            country: Some("US"),
            city: Some("New York"),
            tags: &["workspace:demo-ws-smm"],
        },
        DemoProxy {
            id: "demo-px-nl",
            name: "NL HTTPS Mobile",
            proxy_type: "https",
            host: "proxy-nl.example",
            port: 443,
            username: Some("demo-nl"),
            password: Some("demo-pass-nl"),
            country: Some("NL"),
            city: Some("Amsterdam"),
            tags: &["workspace:demo-ws-smm"],
        },
        DemoProxy {
            id: "demo-px-sg",
            name: "SG Office HTTP",
            proxy_type: "http",
            host: "proxy-sg.example",
            port: 3128,
            username: Some("demo-sg"),
            password: Some("demo-pass-sg"),
            country: Some("SG"),
            city: Some("Singapore"),
            tags: &["workspace:demo-ws-biz"],
        },
        DemoProxy {
            id: "demo-px-home",
            name: "Home SOCKS5",
            proxy_type: "socks5",
            host: "127.0.0.1",
            port: 9050,
            username: None,
            password: None,
            country: None,
            city: None,
            tags: &["workspace:default"],
        },
        DemoProxy {
            id: "demo-px-jump",
            name: "Jump SSH Bastion",
            proxy_type: "ssh",
            host: "bastion.example",
            port: 22,
            username: Some("jump"),
            password: Some("demo-pass-jump"),
            country: Some("DE"),
            city: Some("Falkenstein"),
            tags: &["workspace:demo-ws-devops"],
        },
        DemoProxy {
            id: "demo-px-uk",
            name: "UK Socks Marketing",
            proxy_type: "socks5",
            host: "proxy-uk.example",
            port: 1080,
            username: Some("demo-uk"),
            password: Some("demo-pass-uk"),
            country: Some("GB"),
            city: Some("London"),
            tags: &["workspace:demo-ws-smm"],
        },
        DemoProxy {
            id: "demo-px-fail",
            name: "Fail Check Demo",
            proxy_type: "http",
            host: "dead.proxy.invalid",
            port: 9999,
            username: Some("none"),
            password: Some("none"),
            country: None,
            city: None,
            tags: &[],
        },
    ]
}

fn profiles(ru: bool) -> Vec<DemoProfile> {
    vec![
        DemoProfile {
            id: "demo-pr-bank",
            name: t(ru, "Banking", "Банк"),
            workspace_id: "default",
            proxy_id: Some("demo-px-home"),
            fingerprint_preset: "win11",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("Europe/Berlin"),
            notes: Some(t(ru, "Personal banking only", "Только личный банк")),
            tags: &["active"],
            kanban_status: "active",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-shop",
            name: t(ru, "Shopping", "Покупки"),
            workspace_id: "default",
            proxy_id: None,
            fingerprint_preset: "macos",
            locale: "en-GB",
            languages: "en-GB,en",
            timezone: Some("Europe/London"),
            notes: None,
            tags: &["inbox"],
            kanban_status: "inbox",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-mail",
            name: t(ru, "Personal Mail", "Личная почта"),
            workspace_id: "default",
            proxy_id: None,
            fingerprint_preset: "linux",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("UTC"),
            notes: None,
            tags: &["done"],
            kanban_status: "done",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-ig",
            name: "Instagram Brand A",
            workspace_id: "demo-ws-smm",
            proxy_id: Some("demo-px-us"),
            fingerprint_preset: "win11",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("America/New_York"),
            notes: Some("client:brand-a"),
            tags: &["in_progress", "client:brand-a"],
            kanban_status: "in_progress",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-tt",
            name: "TikTok Brand A",
            workspace_id: "demo-ws-smm",
            proxy_id: Some("demo-px-nl"),
            fingerprint_preset: "win10",
            locale: "nl-NL",
            languages: "nl-NL,en",
            timezone: Some("Europe/Amsterdam"),
            notes: None,
            tags: &["ideas", "client:brand-a"],
            kanban_status: "ideas",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-fb",
            name: "FB Ads EU",
            workspace_id: "demo-ws-smm",
            proxy_id: Some("demo-px-uk"),
            fingerprint_preset: "macos",
            locale: "en-GB",
            languages: "en-GB,en",
            timezone: Some("Europe/London"),
            notes: None,
            tags: &["published"],
            kanban_status: "published",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-li",
            name: "LinkedIn Outreach",
            workspace_id: "demo-ws-smm",
            proxy_id: Some("demo-px-us"),
            fingerprint_preset: "win11",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("America/Chicago"),
            notes: None,
            tags: &["in_progress"],
            kanban_status: "in_progress",
            kanban_order: 1,
        },
        DemoProfile {
            id: "demo-pr-gh",
            name: "GitHub Work",
            workspace_id: "demo-ws-dev",
            proxy_id: None,
            fingerprint_preset: "linux",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("UTC"),
            notes: Some("env:prod"),
            tags: &["coding", "env:prod"],
            kanban_status: "coding",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-qa",
            name: "Staging QA",
            workspace_id: "demo-ws-dev",
            proxy_id: None,
            fingerprint_preset: "win10",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("UTC"),
            notes: Some("env:staging"),
            tags: &["review", "env:staging"],
            kanban_status: "review",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-docs",
            name: "Docs Browse",
            workspace_id: "demo-ws-dev",
            proxy_id: None,
            fingerprint_preset: "macos",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("America/Los_Angeles"),
            notes: None,
            tags: &["backlog"],
            kanban_status: "backlog",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-aws",
            name: "AWS Console",
            workspace_id: "demo-ws-devops",
            proxy_id: Some("demo-px-de"),
            fingerprint_preset: "linux",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("Europe/Berlin"),
            notes: Some("infra:prod"),
            tags: &["deploy", "infra:prod"],
            kanban_status: "deploy",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-grafana",
            name: "Grafana",
            workspace_id: "demo-ws-devops",
            proxy_id: Some("demo-px-de"),
            fingerprint_preset: "linux",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("UTC"),
            notes: None,
            tags: &["monitor"],
            kanban_status: "monitor",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-crm",
            name: "CRM Client Portal",
            workspace_id: "demo-ws-biz",
            proxy_id: Some("demo-px-sg"),
            fingerprint_preset: "win11",
            locale: "en-SG",
            languages: "en-SG,en",
            timezone: Some("Asia/Singapore"),
            notes: None,
            tags: &["negotiation"],
            kanban_status: "negotiation",
            kanban_order: 0,
        },
        DemoProfile {
            id: "demo-pr-bankbiz",
            name: t(ru, "Bank Biz", "Банк (бизнес)"),
            workspace_id: "demo-ws-biz",
            proxy_id: Some("demo-px-sg"),
            fingerprint_preset: "win11",
            locale: "en-US",
            languages: "en-US,en",
            timezone: Some("Asia/Singapore"),
            notes: None,
            tags: &["lead"],
            kanban_status: "lead",
            kanban_order: 0,
        },
    ]
}

#[cfg(desktop)]
fn ssh_keys() -> Vec<DemoSshKey> {
    vec![
        DemoSshKey {
            id: "demo-key-deploy",
            name: "deploy-ed25519",
            algorithm: "ed25519",
        },
        DemoSshKey {
            id: "demo-key-laptop",
            name: "laptop-rsa",
            algorithm: "rsa",
        },
    ]
}

#[cfg(desktop)]
fn ssh_connections() -> Vec<DemoSshConn> {
    vec![
        DemoSshConn {
            id: "demo-ssh-prod",
            name: "prod-web-01",
            host: "prod.example.com",
            port: 22,
            username: "deploy",
            auth_type: "key",
            password: None,
            ssh_key_id: Some("demo-key-deploy"),
            requires_2fa: true,
            totp_entry_id: Some("demo-totp-hetzner"),
            proxy_id: None,
            workspace_ids: &["demo-ws-devops"],
            profile_ids: &["demo-pr-aws"],
        },
        DemoSshConn {
            id: "demo-ssh-staging",
            name: "staging-api",
            host: "staging.example.com",
            port: 22,
            username: "ubuntu",
            auth_type: "password",
            password: Some("demo-pass-staging"),
            ssh_key_id: None,
            requires_2fa: false,
            totp_entry_id: None,
            proxy_id: None,
            workspace_ids: &["demo-ws-devops"],
            profile_ids: &[],
        },
        DemoSshConn {
            id: "demo-ssh-bastion",
            name: "bastion",
            host: "bastion.example.com",
            port: 22,
            username: "jump",
            auth_type: "key",
            password: None,
            ssh_key_id: Some("demo-key-laptop"),
            requires_2fa: false,
            totp_entry_id: None,
            proxy_id: Some("demo-px-jump"),
            workspace_ids: &["demo-ws-devops"],
            profile_ids: &[],
        },
        DemoSshConn {
            id: "demo-ssh-nas",
            name: "home-nas",
            host: "nas.lan",
            port: 22,
            username: "admin",
            auth_type: "password",
            password: Some("demo-pass-nas"),
            ssh_key_id: None,
            requires_2fa: false,
            totp_entry_id: None,
            proxy_id: None,
            workspace_ids: &["default"],
            profile_ids: &[],
        },
        DemoSshConn {
            id: "demo-ssh-ci",
            name: "ci-runner",
            host: "ci.example.com",
            port: 22,
            username: "runner",
            auth_type: "key",
            password: None,
            ssh_key_id: Some("demo-key-deploy"),
            requires_2fa: false,
            totp_entry_id: None,
            proxy_id: None,
            workspace_ids: &["demo-ws-devops"],
            profile_ids: &["demo-pr-grafana"],
        },
    ]
}

#[cfg(desktop)]
fn capture_rules() -> Vec<DemoCapture> {
    vec![
        DemoCapture {
            domain: "instagram.com",
            folder_id: "demo-folder-smm-content",
            tags: &["work/smm"],
        },
        DemoCapture {
            domain: "github.com",
            folder_id: "demo-folder-dev",
            tags: &["work/dev"],
        },
    ]
}
