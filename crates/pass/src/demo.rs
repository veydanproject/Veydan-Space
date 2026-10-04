// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! The part of pass in the demo data: TOTP entries and passwords in English
//! or Russian, the same ids in both. The driver of the shell sets the demo
//! lock before any part seeds, so the key is open here.
//!
//! An entry links to profiles, workspaces and notes of other modules by the
//! ids their own parts give them; pass reads none of their tables. A link
//! is seeded only where the product has the owner of its kind ([`bindable`]).

use chrono::Utc;
use tauri::{AppHandle, Manager, Runtime};
use veydan_core::{AppError, BoxFuture, CmdResult, Core};
use veydan_lock::{Lock, SecretKey};

/// Seed the TOTP entries, then the passwords that link them.
pub(crate) fn seed<R: Runtime>(
    app: AppHandle<R>,
    locale: String,
) -> BoxFuture<'static, CmdResult<()>> {
    Box::pin(async move {
        let core = app.state::<Core>();
        let ru = locale.eq_ignore_ascii_case("ru");
        let now = Utc::now().to_rfc3339();
        seed_totp(&core, &now).await?;
        seed_bulk_totp(&core, &now).await?;
        let (key, vault_id) = app.state::<Lock>().require_open()?;
        seed_passwords(&core, ru, &key, &vault_id, &now).await?;
        seed_extra_passwords(&core, ru, &key, &vault_id, &now).await
    })
}

/// Empty the tables of pass. The key of the lock stays: it is the lock's.
pub(crate) fn clear<R: Runtime>(app: AppHandle<R>) -> BoxFuture<'static, CmdResult<()>> {
    Box::pin(async move {
        let db = &app.state::<Core>().db;
        for table in ["totp_entries", "passwords", "password_history"] {
            // Safe: the names are the constants above.
            let _ = sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
                .execute(db)
                .await;
        }
        Ok(())
    })
}

/// The kinds of entity a demo tag `kind:id` may name (the rest are labels).
const ENTITY_KINDS: &[&str] = &[
    "workspace",
    "profile",
    "proxy",
    "ssh",
    "note",
    "totp",
    "password",
];

/// The tags of a demo entry whose kind has an owner in this product; a label
/// stays. The entities a tag names are the owners' demo data, so in a product
/// without the owner (Pass alone: no workspaces, profiles or notes) they do
/// not exist, and a link to them would only show a kind and a short id
/// (spec 10.3).
fn bindable(core: &Core, tags: &[String]) -> Vec<String> {
    let owned = core.directory.kinds();
    tags.iter()
        .filter(|tag| match tag.split_once(':') {
            Some((kind, _)) if ENTITY_KINDS.contains(&kind) => owned.contains(&kind),
            _ => true,
        })
        .cloned()
        .collect()
}

fn t(ru: bool, en: &'static str, ru_s: &'static str) -> &'static str {
    if ru {
        ru_s
    } else {
        en
    }
}

struct DemoTotp {
    id: &'static str,
    name: &'static str,
    issuer: &'static str,
    secret: &'static str,
    tags: &'static [&'static str],
}

struct DemoPassword {
    id: &'static str,
    title: &'static str,
    username: &'static str,
    url: &'static str,
    password: &'static str,
    note: Option<&'static str>,
    totp_ids: &'static [&'static str],
    tags: &'static [&'static str],
    /// Notes of the notes module the entry names with a tag `note:{id}`.
    notes: &'static [&'static str],
}

/// The workspaces of the demo data: the default one and those the browser
/// part seeds.
const WS: &[&str] = &[
    "default",
    "demo-ws-smm",
    "demo-ws-dev",
    "demo-ws-devops",
    "demo-ws-biz",
    "demo-ws-freelance",
    "demo-ws-research",
];

/// The profiles the browser part seeds by hand; the rest are
/// `demo-pr-bulk-01` to `demo-pr-bulk-56`.
const PROFILES: &[&str] = &[
    "demo-pr-bank",
    "demo-pr-shop",
    "demo-pr-mail",
    "demo-pr-ig",
    "demo-pr-tt",
    "demo-pr-fb",
    "demo-pr-li",
    "demo-pr-gh",
    "demo-pr-qa",
    "demo-pr-docs",
    "demo-pr-aws",
    "demo-pr-grafana",
    "demo-pr-crm",
    "demo-pr-bankbiz",
];
const BULK_PROFILES: usize = 56;

fn bulk_profile(i: usize) -> String {
    format!("demo-pr-bulk-{i:02}")
}

/// The notes the notes part seeds and keeps out of the trash, in the order
/// it makes them: `demo-note-NN` is the note at that place of its list, those
/// without links first and those that link others after them; then its bulk
/// notes, `demo-note-bulk-NNN`, but for the one in the trash.
const NOTES: &[usize] = &[
    0, 1, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 2, 8, 9, 10, 16,
];
const BULK_NOTES: usize = 144;
const BULK_NOTE_IN_TRASH: usize = 73;

fn live_notes() -> Vec<String> {
    NOTES
        .iter()
        .map(|i| format!("demo-note-{i:02}"))
        .chain(
            (1..=BULK_NOTES)
                .filter(|i| *i != BULK_NOTE_IN_TRASH)
                .map(|i| format!("demo-note-bulk-{i:03}")),
        )
        .collect()
}

fn totp_entries() -> Vec<DemoTotp> {
    vec![
        DemoTotp {
            id: "demo-totp-gh",
            name: "github-work",
            issuer: "GitHub",
            secret: "JBSWY3DPEHPK3PXP",
            tags: &["profile:demo-pr-gh", "workspace:demo-ws-dev"],
        },
        DemoTotp {
            id: "demo-totp-aws",
            name: "aws-root",
            issuer: "Amazon",
            secret: "HXDMVJECJJWSRB3H",
            tags: &["profile:demo-pr-aws", "workspace:demo-ws-devops"],
        },
        DemoTotp {
            id: "demo-totp-cf",
            name: "cloudflare",
            issuer: "Cloudflare",
            secret: "GEZDGNBVGY3TQOJQ",
            tags: &["workspace:demo-ws-devops"],
        },
        DemoTotp {
            id: "demo-totp-gads",
            name: "google-ads",
            issuer: "Google",
            secret: "MFRGG2LTMVZHG2LH",
            tags: &["profile:demo-pr-fb", "workspace:demo-ws-smm"],
        },
        DemoTotp {
            id: "demo-totp-binance",
            name: "binance",
            issuer: "Binance",
            secret: "NBSWY3DPO5XXE3DE",
            tags: &["workspace:default"],
        },
        DemoTotp {
            id: "demo-totp-tg",
            name: "telegram",
            issuer: "Telegram",
            secret: "KRSXG5DTMVRXEZLU",
            tags: &["workspace:demo-ws-smm"],
        },
        DemoTotp {
            id: "demo-totp-1p",
            name: "1password-bridge",
            issuer: "1Password",
            secret: "MFZWIIDDN5SW4Y3F",
            tags: &["workspace:demo-ws-biz"],
        },
        DemoTotp {
            id: "demo-totp-gl",
            name: "gitlab-ci",
            issuer: "GitLab",
            secret: "ORUGS4ZANFZSAYLO",
            tags: &["workspace:demo-ws-dev"],
        },
        DemoTotp {
            id: "demo-totp-hetzner",
            name: "hetzner",
            issuer: "Hetzner",
            secret: "KRUGS4ZANFZSAYJA",
            tags: &["workspace:demo-ws-devops"],
        },
        DemoTotp {
            id: "demo-totp-bw",
            name: "bitwarden",
            issuer: "Bitwarden",
            secret: "IFBEGRCFIZDUQYLC",
            tags: &["workspace:default"],
        },
        DemoTotp {
            id: "demo-totp-do",
            name: "digitalocean",
            issuer: "DigitalOcean",
            secret: "KRSXG5BAON2HE2LO",
            tags: &["workspace:demo-ws-devops"],
        },
        DemoTotp {
            id: "demo-totp-stripe",
            name: "stripe",
            issuer: "Stripe",
            secret: "MJQXGZJTGIZTILLC",
            tags: &["workspace:demo-ws-biz"],
        },
    ]
}

fn passwords(ru: bool) -> Vec<DemoPassword> {
    vec![
        DemoPassword {
            id: "demo-pw-ig",
            title: "Instagram Brand A",
            username: "brand.a",
            url: "https://instagram.com",
            password: "demo-ig-BrandA",
            note: Some(t(
                ru,
                "Ad account for Brand A. 2FA is the Telegram code.",
                "Рекламный кабинет Brand A. 2FA — код Telegram.",
            )),
            totp_ids: &["demo-totp-tg", "demo-totp-cf"],
            tags: &[
                "profile:demo-pr-ig",
                "workspace:demo-ws-smm",
                "client/brand-a",
                "work/smm",
            ],
            // Client brief Brand A, Content plan week
            notes: &["demo-note-04", "demo-note-02"],
        },
        DemoPassword {
            id: "demo-pw-gh",
            title: "GitHub",
            username: "octocat",
            url: "https://github.com",
            password: "demo-gh-work",
            note: Some(t(ru, "Work org login.", "Вход в рабочую организацию.")),
            totp_ids: &["demo-totp-gh"],
            tags: &["profile:demo-pr-gh", "workspace:demo-ws-dev", "work/dev"],
            // Repo onboarding, ADR auth
            notes: &["demo-note-14", "demo-note-09"],
        },
        DemoPassword {
            id: "demo-pw-aws",
            title: "AWS root",
            username: "root",
            url: "https://console.aws.amazon.com",
            password: "demo-aws-root",
            note: Some(t(ru, "Root. Prefer IAM.", "Root. Лучше IAM.")),
            totp_ids: &["demo-totp-aws"],
            tags: &[
                "profile:demo-pr-aws",
                "workspace:demo-ws-devops",
                "infra/prod",
            ],
            // Deploy runbook
            notes: &["demo-note-16"],
        },
        DemoPassword {
            id: "demo-pw-cf",
            title: "Cloudflare",
            username: "ops@veydan.test",
            url: "https://dash.cloudflare.com",
            password: "demo-cf-ops",
            note: None,
            totp_ids: &["demo-totp-cf"],
            tags: &["workspace:demo-ws-devops", "infra/prod", "work/devops"],
            // Nginx snippet, Monitoring alerts
            notes: &["demo-note-18", "demo-note-19"],
        },
        DemoPassword {
            id: "demo-pw-stripe",
            title: "Stripe",
            username: "billing@veydan.test",
            url: "https://dashboard.stripe.com",
            password: "demo-stripe-bill",
            note: Some(t(ru, "Billing owner.", "Владелец биллинга.")),
            totp_ids: &["demo-totp-stripe"],
            tags: &["workspace:demo-ws-biz", "work/biz"],
            // Deal pipeline
            notes: &["demo-note-23"],
        },
        DemoPassword {
            id: "demo-pw-bank",
            title: t(ru, "Bank", "Банк"),
            username: "admin@veydan.net",
            url: "https://veydan.net",
            password: "demo-bank-admin",
            note: Some(t(
                ru,
                "Personal banking. Vault lock is demo.",
                "Личный банк. Пароль хранилища — demo.",
            )),
            totp_ids: &[],
            tags: &["profile:demo-pr-bank", "workspace:default", "personal"],
            // Health log, WiFi codes home
            notes: &["demo-note-31", "demo-note-32"],
        },
    ]
}

async fn insert_totp(
    core: &Core,
    id: &str,
    name: &str,
    issuer: &str,
    secret: &str,
    tags: &[String],
    now: &str,
) -> CmdResult<()> {
    let tags = serde_json::to_string(&bindable(core, tags)).map_err(AppError::other)?;
    sqlx::query(
        "INSERT INTO totp_entries (id, name, issuer, secret, algorithm, digits, period, tags, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'SHA1', 6, 30, ?, ?, ?)",
    )
    .bind(id)
    .bind(name)
    .bind(issuer)
    .bind(secret)
    .bind(&tags)
    .bind(now)
    .bind(now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

async fn seed_totp(core: &Core, now: &str) -> CmdResult<()> {
    for e in totp_entries() {
        let tags: Vec<String> = e.tags.iter().map(|s| s.to_string()).collect();
        insert_totp(core, e.id, e.name, e.issuer, e.secret, &tags, now).await?;
    }
    Ok(())
}

const ISSUERS: &[&str] = &[
    "GitHub",
    "GitLab",
    "Bitbucket",
    "AWS",
    "GCP",
    "Azure",
    "Cloudflare",
    "DigitalOcean",
    "Hetzner",
    "Vultr",
    "Linode",
    "Stripe",
    "PayPal",
    "Shopify",
    "Notion",
    "Slack",
    "Discord",
    "Telegram",
    "Twitter",
    "Meta",
    "Google",
    "Microsoft",
    "Apple",
    "Dropbox",
    "1Password",
    "Bitwarden",
    "Okta",
    "Auth0",
    "Twilio",
    "SendGrid",
    "Mailchimp",
    "HubSpot",
    "Salesforce",
    "Zendesk",
    "Jira",
    "Confluence",
    "Figma",
    "Notion",
    "Linear",
    "Vercel",
    "Netlify",
    "Heroku",
    "Railway",
    "Render",
    "Supabase",
    "PlanetScale",
    "MongoDB",
    "Redis",
];

fn demo_secret(n: u32) -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut s = String::with_capacity(16);
    let mut x = n.wrapping_mul(2654435761);
    for _ in 0..16 {
        s.push(ALPH[(x % 32) as usize] as char);
        x = x
            .wrapping_mul(2246822519)
            .wrapping_add(n.wrapping_mul(7) + 1);
    }
    s
}

/// Extra volume for a fuller video demo: 12 handcrafted + 48 bulk = 60.
async fn seed_bulk_totp(core: &Core, now: &str) -> CmdResult<()> {
    for i in 1..=48 {
        let issuer = ISSUERS[(i as usize - 1) % ISSUERS.len()];
        let id = format!("demo-totp-bulk-{i:02}");
        let name = format!("{}-{}", issuer.to_lowercase(), i);
        let ws = WS[(i as usize - 1) % WS.len()];
        let mut tags = vec![format!("workspace:{ws}")];
        if i % 2 == 0 {
            let profile = bulk_profile((i as usize - 1) % BULK_PROFILES + 1);
            tags.push(format!("profile:{profile}"));
        }
        insert_totp(core, &id, &name, issuer, &demo_secret(i), &tags, now).await?;
    }
    Ok(())
}

/// One entry as it is stored, before its secrets are encrypted.
struct Entry<'a> {
    id: &'a str,
    title: &'a str,
    username: &'a str,
    url: &'a str,
    password: &'a str,
    note: Option<&'a str>,
    totp_ids: Vec<String>,
    tags: Vec<String>,
}

async fn insert_password(
    core: &Core,
    key: &SecretKey,
    vault_id: &str,
    entry: Entry<'_>,
    now: &str,
) -> CmdResult<()> {
    let Entry {
        id,
        title,
        username,
        url,
        password,
        note,
        totp_ids,
        tags,
    } = entry;
    let tags_json = serde_json::to_string(&bindable(core, &tags)).map_err(AppError::other)?;
    let totp_json = serde_json::to_string(&totp_ids).map_err(AppError::other)?;
    let password_enc = veydan_lock::encrypt_field(key, id, "password", password)?;
    let note_enc = match note {
        Some(note) if !note.is_empty() => Some(veydan_lock::encrypt_field(key, id, "note", note)?),
        _ => None,
    };
    sqlx::query(
        "INSERT INTO passwords (
            id, title, username, url, password_enc, note_enc, totp_ids, tags, vault_id, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(title)
    .bind(username)
    .bind(url)
    .bind(&password_enc)
    .bind(&note_enc)
    .bind(&totp_json)
    .bind(&tags_json)
    .bind(vault_id)
    .bind(now)
    .bind(now)
    .execute(&core.db)
    .await
    .map_err(AppError::db)?;
    Ok(())
}

/// Encrypted entries linked to profiles, workspaces, TOTP and notes.
async fn seed_passwords(
    core: &Core,
    ru: bool,
    key: &SecretKey,
    vault_id: &str,
    now: &str,
) -> CmdResult<()> {
    for entry in passwords(ru) {
        let mut tags: Vec<String> = entry.tags.iter().map(|s| s.to_string()).collect();
        tags.extend(entry.notes.iter().map(|id| format!("note:{id}")));
        let stored = Entry {
            id: entry.id,
            title: entry.title,
            username: entry.username,
            url: entry.url,
            password: entry.password,
            note: entry.note,
            totp_ids: entry.totp_ids.iter().map(|s| s.to_string()).collect(),
            tags,
        };
        insert_password(core, key, vault_id, stored, now).await?;
    }
    Ok(())
}

const EXTRA_LOGINS: &[(&str, &str, &str)] = &[
    ("Notion", "ops@veydan.test", "https://notion.so"),
    ("Figma", "design@veydan.test", "https://figma.com"),
    ("Slack", "team@veydan.test", "https://slack.com"),
    ("Jira", "dev@veydan.test", "https://atlassian.net"),
    ("Linear", "pm@veydan.test", "https://linear.app"),
    ("Sentry", "ops@veydan.test", "https://sentry.io"),
    ("Datadog", "sre@veydan.test", "https://datadoghq.com"),
    ("Vercel", "dev@veydan.test", "https://vercel.com"),
    ("Netlify", "dev@veydan.test", "https://netlify.com"),
    ("GitLab", "ci@veydan.test", "https://gitlab.com"),
    ("Hetzner", "root", "https://console.hetzner.cloud"),
    (
        "DigitalOcean",
        "ops@veydan.test",
        "https://cloud.digitalocean.com",
    ),
    (
        "Bitwarden",
        "admin@veydan.test",
        "https://vault.bitwarden.com",
    ),
    ("1Password", "admin@veydan.test", "https://my.1password.com"),
    ("Google Ads", "ads@veydan.test", "https://ads.google.com"),
    ("Binance", "trader@veydan.test", "https://binance.com"),
    ("Grafana", "ops@veydan.test", "https://grafana.veydan.test"),
    ("Mail", "inbox@veydan.test", "https://mail.veydan.test"),
    ("TikTok Ads", "brand.a", "https://ads.tiktok.com"),
    ("LinkedIn", "brand.a", "https://linkedin.com"),
    ("Facebook Ads", "brand.a", "https://business.facebook.com"),
    ("Shopify", "shop@veydan.test", "https://admin.shopify.com"),
    ("npm", "dev@veydan.test", "https://npmjs.com"),
    ("Docker Hub", "dev@veydan.test", "https://hub.docker.com"),
    ("OpenAI", "api@veydan.test", "https://platform.openai.com"),
    ("Statuspage", "ops@veydan.test", "https://statuspage.io"),
    ("PagerDuty", "oncall@veydan.test", "https://pagerduty.com"),
    ("HubSpot", "sales@veydan.test", "https://app.hubspot.com"),
    ("Zoom", "meet@veydan.test", "https://zoom.us"),
    ("Miro", "design@veydan.test", "https://miro.com"),
    ("Dropbox", "files@veydan.test", "https://dropbox.com"),
];

const EXTRA_LABELS: &[&str] = &[
    "work/smm",
    "work/dev",
    "work/devops",
    "work/biz",
    "personal",
    "client/brand-a",
    "infra/prod",
    "infra/staging",
];

/// More entries spread over the workspaces, profiles, TOTP entries and notes.
async fn seed_extra_passwords(
    core: &Core,
    ru: bool,
    key: &SecretKey,
    vault_id: &str,
    now: &str,
) -> CmdResult<()> {
    let note_ids = live_notes();
    let totp_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM totp_entries ORDER BY id")
        .fetch_all(&core.db)
        .await
        .map_err(AppError::db)?;
    let mut profile_ids: Vec<String> = PROFILES
        .iter()
        .map(|s| s.to_string())
        .chain((1..=BULK_PROFILES).map(bulk_profile))
        .collect();
    profile_ids.sort_unstable();
    let mut ws_ids = WS.to_vec();
    ws_ids.sort_unstable();

    for i in 1..=EXTRA_LOGINS.len() {
        let (title, username, url) = EXTRA_LOGINS[i - 1];
        let id = format!("demo-pw-x{i:02}");
        let mut tags = vec![format!("workspace:{}", ws_ids[(i - 1) % ws_ids.len()])];
        if i % 2 == 1 {
            tags.push(format!(
                "profile:{}",
                profile_ids[(i - 1) % profile_ids.len()]
            ));
        }
        tags.push(EXTRA_LABELS[(i - 1) % EXTRA_LABELS.len()].to_string());
        let note_a = &note_ids[(i - 1) % note_ids.len()];
        tags.push(format!("note:{note_a}"));
        if i % 3 == 0 {
            let note_b = &note_ids[(i + 11) % note_ids.len()];
            if note_b != note_a {
                tags.push(format!("note:{note_b}"));
            }
        }
        let mut linked: Vec<String> = Vec::new();
        if i % 2 == 0 && !totp_ids.is_empty() {
            linked.push(totp_ids[(i - 1) % totp_ids.len()].clone());
        }
        if i % 6 == 0 && totp_ids.len() > 1 {
            let second = totp_ids[i % totp_ids.len()].clone();
            if !linked.contains(&second) {
                linked.push(second);
            }
        }
        let secret_note = if i % 4 == 0 {
            None
        } else if ru {
            Some(format!("Демо-доступ #{i}. Пароль хранилища — demo."))
        } else {
            Some(format!("Demo login #{i}. Vault lock is demo."))
        };
        let password = format!("demo-extra-{i:02}");
        let stored = Entry {
            id: &id,
            title,
            username,
            url,
            password: &password,
            note: secret_note.as_deref(),
            totp_ids: linked,
            tags,
        };
        insert_password(core, key, vault_id, stored, now).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    async fn count(core: &Core, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(&core.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_part_of_pass_is_seeded_under_the_open_key_and_cleared() {
        let (app, _dir) = testing::app().await;
        let core = app.state::<Core>();
        let lock = app.state::<Lock>();
        lock.ensure_key().await.unwrap();
        for locale in ["en", "ru"] {
            clear(app.handle().clone()).await.unwrap();
            seed(app.handle().clone(), locale.into()).await.unwrap();
            assert_eq!(count(&core, "totp_entries").await, 60, "{locale}");
            assert_eq!(count(&core, "passwords").await, 37, "{locale}");
        }
        let (key, vault_id) = lock.require_open().unwrap();
        let (enc, row_vault): (String, String) = sqlx::query_as(
            "SELECT password_enc, vault_id FROM passwords WHERE id = 'demo-pw-bank'",
        )
        .fetch_one(&core.db)
        .await
        .unwrap();
        assert_eq!(row_vault, vault_id);
        assert_eq!(
            veydan_lock::decrypt_field(&key, "demo-pw-bank", "password", &enc).unwrap(),
            "demo-bank-admin"
        );

        clear(app.handle().clone()).await.unwrap();
        for table in ["totp_entries", "passwords", "password_history"] {
            assert_eq!(count(&core, table).await, 0, "{table}");
        }
    }

    /// Pass alone owns no workspaces, profiles or notes: its demo entries do
    /// not link to them, and keep their labels and their TOTP codes.
    #[tokio::test]
    async fn a_product_without_their_owners_seeds_no_links_to_them() {
        let (app, _dir) = testing::app().await;
        let core = app.state::<Core>();
        app.state::<Lock>().ensure_key().await.unwrap();
        seed(app.handle().clone(), "en".into()).await.unwrap();
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT tags FROM passwords UNION ALL SELECT tags FROM totp_entries",
        )
        .fetch_all(&core.db)
        .await
        .unwrap();
        for tags in &rows {
            for kind in ["workspace:", "profile:", "note:"] {
                assert!(!tags.contains(kind), "{tags}");
            }
        }
        let (tags, totp): (String, String) =
            sqlx::query_as("SELECT tags, totp_ids FROM passwords WHERE id = 'demo-pw-gh'")
                .fetch_one(&core.db)
                .await
                .unwrap();
        assert_eq!(tags, r#"["work/dev"]"#);
        assert_eq!(totp, r#"["demo-totp-gh"]"#);
    }

    #[test]
    fn the_notes_an_entry_names_are_notes_of_the_demo_data() {
        let live = live_notes();
        assert_eq!(live.len(), 35 + 143);
        for entry in passwords(false) {
            for note in entry.notes {
                assert!(live.iter().any(|l| l == note), "{note}");
            }
        }
    }
}
