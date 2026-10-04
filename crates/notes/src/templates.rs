// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Note templates: any note inside the `Templates` folder. Placeholders:
//! `{{date}}`, `{{time}}`, `{{datetime}}`, `{{title}}`, `{{profile}}`,
//! `{{workspace}}`, `{{url}}`, `{{domain}}`, `{{proxy}}`, `{{proxy_type}}`,
//! `{{proxy_host}}`, `{{proxy_port}}`, `{{proxy_region}}`, `{{ssh}}`,
//! `{{ssh_host}}`, `{{ssh_port}}`, `{{ssh_user}}`, `{{totp}}`.
//! Only non-secret fields are ever exposed: the values come from the
//! entity directory, whose owners give public fields alone.

use super::binding::BindingKind;
use super::entities::{field, name};
use super::files::{read_note_file, resolve_note_abs_path};
use chrono::Local;
use std::collections::HashMap;
use tauri::Runtime;
use veydan_core::{AppError, CmdResult};
use veydan_core::{Core, Directory};

#[derive(Debug, Default, Clone)]
pub(crate) struct TemplateVars {
    pub title: String,
    pub profile: String,
    pub workspace: String,
    pub url: String,
    pub domain: String,
    pub proxy: String,
    pub proxy_type: String,
    pub proxy_host: String,
    pub proxy_port: String,
    pub proxy_region: String,
    pub ssh: String,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub totp: String,
    pub password: String,
}

impl TemplateVars {
    /// Fill entity names and public fields from bindings.
    pub(crate) async fn from_bindings<R: Runtime>(
        title: &str,
        bindings: &[String],
        directory: &Directory<R>,
    ) -> Self {
        let mut vars = Self {
            title: title.to_string(),
            ..Default::default()
        };
        for b in bindings {
            let Some((kind, value)) = BindingKind::parse(b) else {
                continue;
            };
            match kind {
                BindingKind::Profile => vars.profile = name(directory, kind, value).await,
                BindingKind::Workspace => vars.workspace = name(directory, kind, value).await,
                BindingKind::Totp => vars.totp = name(directory, kind, value).await,
                BindingKind::Password => vars.password = name(directory, kind, value).await,
                BindingKind::Url => vars.url = value.to_string(),
                BindingKind::Domain => vars.domain = value.to_string(),
                BindingKind::Proxy => {
                    if let Some(label) = directory.label(kind.name(), value).await {
                        let get = |f| field(directory, kind, value, f);
                        vars.proxy = label.name;
                        vars.proxy_type = get("proxy_type").await;
                        vars.proxy_host = get("host").await;
                        vars.proxy_port = get("port").await;
                        vars.proxy_region = [get("country").await, get("city").await]
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(", ");
                    }
                }
                BindingKind::Ssh => {
                    if let Some(label) = directory.label(kind.name(), value).await {
                        let get = |f| field(directory, kind, value, f);
                        vars.ssh = label.name;
                        vars.ssh_host = get("host").await;
                        vars.ssh_port = get("port").await;
                        vars.ssh_user = get("username").await;
                    }
                }
            }
        }
        vars
    }
}

/// Current value of every placeholder.
fn pairs(vars: &TemplateVars) -> [(&'static str, String); 19] {
    let now = Local::now();
    [
        ("date", now.format("%Y-%m-%d").to_string()),
        ("time", now.format("%H:%M").to_string()),
        ("datetime", now.format("%Y-%m-%d %H:%M").to_string()),
        ("title", vars.title.clone()),
        ("profile", vars.profile.clone()),
        ("workspace", vars.workspace.clone()),
        ("url", vars.url.clone()),
        ("domain", vars.domain.clone()),
        ("proxy", vars.proxy.clone()),
        ("proxy_type", vars.proxy_type.clone()),
        ("proxy_host", vars.proxy_host.clone()),
        ("proxy_port", vars.proxy_port.clone()),
        ("proxy_region", vars.proxy_region.clone()),
        ("ssh", vars.ssh.clone()),
        ("ssh_host", vars.ssh_host.clone()),
        ("ssh_port", vars.ssh_port.clone()),
        ("ssh_user", vars.ssh_user.clone()),
        ("totp", vars.totp.clone()),
        ("password", vars.password.clone()),
    ]
}

/// Replace `{{name}}` placeholders; unknown ones are left as-is.
pub(crate) fn render(body: &str, vars: &TemplateVars) -> String {
    let mut out = body.to_string();
    for (key, value) in pairs(vars) {
        out = out
            .replace(&format!("{{{{{key}}}}}"), &value)
            .replace(&format!("{{{{ {key} }}}}"), &value);
    }
    out
}

/// Placeholder name -> value for a note with `title` and `bindings`, e.g. to expand `{{date}}` in place.
#[tauri::command]
pub async fn note_placeholder_values(
    title: String,
    bindings: Vec<String>,
    core: tauri::State<'_, Core>,
) -> CmdResult<HashMap<String, String>> {
    Ok(placeholder_values(&core.directory, &title, &bindings).await)
}

/// The value of every placeholder for a note with `title` and `bindings`.
pub async fn placeholder_values<R: Runtime>(
    directory: &Directory<R>,
    title: &str,
    bindings: &[String],
) -> HashMap<String, String> {
    let vars = TemplateVars::from_bindings(title, bindings, directory).await;
    pairs(&vars)
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

/// Body of a template note.
pub(crate) async fn template_body(template_id: &str, core: &Core) -> Result<String, AppError> {
    let file_path: String =
        sqlx::query_scalar("SELECT file_path FROM notes WHERE id = ? AND deleted = 0")
            .bind(template_id)
            .fetch_optional(&core.db)
            .await
            .map_err(AppError::db)?
            .ok_or_else(|| AppError::not_found(format!("Template {template_id}")))?;
    let (_, _, body) = read_note_file(&resolve_note_abs_path(&core.app_data_dir, &file_path))?;
    Ok(body)
}

/// Render a template into content for a new note.
pub(crate) async fn render_template(
    template_id: &str,
    vars: &TemplateVars,
    core: &Core,
) -> Result<String, AppError> {
    let body = template_body(template_id, core).await?;
    Ok(render(&body, vars))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_known_placeholders_and_keeps_unknown() {
        let vars = TemplateVars {
            title: "T".into(),
            profile: "P".into(),
            ..Default::default()
        };
        let out = render("# {{title}} / {{ profile }} / {{unknown}}", &vars);
        assert_eq!(out, "# T / P / {{unknown}}");
    }
}
