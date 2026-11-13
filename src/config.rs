//! Configuration: one TOML file for a single local user, env vars override.
//!
//! Precedence, lowest to highest: built-in defaults, config file, environment.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

pub const DEFAULT_TIMEZONE: &str = "GMT Standard Time";
pub const DEFAULT_SIGNOFF: &str = "The Team";

/// Flat on-disk shape. Every field is optional so a partial config file still loads.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    tenant_id: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    user_email: Option<String>,
    token_path: Option<String>,
    attachment_root: Option<String>,
    calendar_timezone: Option<String>,
    excluded_folder_ids: Option<Vec<String>>,
    skip_excluded_folders: Option<bool>,
    signoff_name: Option<String>,
    read_only: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub tenant_id: String,
    pub client_id: String,
    /// Optional: delegated device-code flow works as a public client (no secret).
    /// Set it only if the app registration is a confidential client.
    pub client_secret: Option<String>,
    pub user_email: String,
    /// Where delegated access/refresh tokens are cached (`0600`).
    pub token_path: PathBuf,
    pub attachment_root: PathBuf,
    pub calendar_timezone: String,
    pub excluded_folder_ids: HashSet<String>,
    pub skip_excluded_folders: bool,
    pub signoff_name: String,
    /// Hides and rejects every tool that writes to the mailbox or calendar.
    /// Defaults to `true`: an app registration with only Read scopes must not
    /// advertise write tools it cannot use.
    pub read_only: bool,
    /// Path the config was loaded from, for diagnostics only. Never logged with values.
    pub source: Option<PathBuf>,
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = explicit
            .map(Path::to_path_buf)
            .or_else(|| std::env::var_os("M365_CONFIG").map(PathBuf::from))
            .unwrap_or_else(default_config_path);

        let file = match path.exists() {
            true => {
                check_permissions(&path);
                let raw = std::fs::read_to_string(&path)
                    .with_context(|| format!("reading config file {}", path.display()))?;
                toml::from_str::<FileConfig>(&raw)
                    .with_context(|| format!("parsing config file {}", path.display()))?
            }
            false => FileConfig::default(),
        };

        let tz = env_str("OUTLOOK_CALENDAR_TIMEZONE")
            .or(file.calendar_timezone)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TIMEZONE.to_string());

        let attachment_root = env_str("OUTLOOK_ATTACHMENT_ROOT")
            .or(file.attachment_root)
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand_tilde(&s))
            .unwrap_or_else(default_attachment_root);

        let token_path = env_str("M365_TOKEN_PATH")
            .or(file.token_path)
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand_tilde(&s))
            .unwrap_or_else(default_token_path);

        let excluded = env_str("OUTLOOK_EXCLUDED_FOLDER_IDS")
            .or_else(|| file.excluded_folder_ids.map(|v| v.join(",")))
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        let skip = env_str("OUTLOOK_SKIP_EXCLUDED_FOLDERS")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .or(file.skip_excluded_folders)
            .unwrap_or(false);

        Ok(Self {
            tenant_id: env_str("AZURE_TENANT_ID")
                .or(file.tenant_id)
                .unwrap_or_default(),
            client_id: env_str("AZURE_CLIENT_ID")
                .or(file.client_id)
                .unwrap_or_default(),
            client_secret: env_str("AZURE_CLIENT_SECRET")
                .or(file.client_secret)
                .filter(|s| !s.trim().is_empty()),
            user_email: env_str("OUTLOOK_USER_EMAIL")
                .or(file.user_email)
                .unwrap_or_default(),
            token_path,
            attachment_root,
            calendar_timezone: tz,
            excluded_folder_ids: excluded,
            skip_excluded_folders: skip,
            read_only: env_str("OUTLOOK_READ_ONLY")
                .map(|value| !(value.eq_ignore_ascii_case("false") || value == "0"))
                .or(file.read_only)
                .unwrap_or(true),
            signoff_name: env_str("OUTLOOK_SIGNOFF_NAME")
                .or(file.signoff_name)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_SIGNOFF.to_string()),
            source: path.exists().then_some(path),
        })
    }

    /// Delegated mode needs a tenant, client id and the target user; the client
    /// secret is optional (public client) and `login` supplies the tokens at
    /// runtime.
    pub fn is_configured(&self) -> bool {
        !self.tenant_id.is_empty()
            && !self.client_id.is_empty()
            && !self.user_email.is_empty()
            && !self.tenant_id.contains("your_")
    }

    pub fn summary(&self) -> String {
        format!(
            "configured={} read_only={} auth=delegated mailbox={} tz={} attachment_root={} tokens={} config={}",
            self.is_configured(),
            self.read_only,
            if self.user_email.is_empty() {
                "<unset>"
            } else {
                &self.user_email
            },
            self.calendar_timezone,
            self.attachment_root.display(),
            self.token_path.display(),
            self.source
                .as_ref()
                .map_or("<defaults>".to_string(), |p| p.display().to_string()),
        )
    }
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn default_config_path() -> PathBuf {
    home().join(".config").join("m365-mcp").join("config.toml")
}

fn default_attachment_root() -> PathBuf {
    home().join("Downloads")
}

fn default_token_path() -> PathBuf {
    home().join(".config").join("m365-mcp").join("tokens.json")
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

fn expand_tilde(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if raw == "~" => home(),
        None => PathBuf::from(raw),
    }
}

/// Warn (don't fail) when the secret sits in a world-readable file.
fn check_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                tracing::warn!(
                    path = %path.display(),
                    mode = format!("{mode:o}"),
                    "config file is readable by other users; run: chmod 600 {}",
                    path.display()
                );
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands() {
        assert!(expand_tilde("~/Documents").is_absolute());
        assert_eq!(expand_tilde("/tmp/x"), PathBuf::from("/tmp/x"));
    }

    #[test]
    fn placeholder_tenant_is_not_configured() {
        let cfg = Config {
            tenant_id: "your_tenant_id".into(),
            client_id: "c".into(),
            client_secret: Some("s".into()),
            user_email: "u@example.com".into(),
            token_path: PathBuf::from("/tmp/tokens.json"),
            attachment_root: PathBuf::from("/tmp"),
            calendar_timezone: DEFAULT_TIMEZONE.into(),
            excluded_folder_ids: HashSet::new(),
            skip_excluded_folders: false,
            signoff_name: DEFAULT_SIGNOFF.into(),
            read_only: true,
            source: None,
        };
        assert!(!cfg.is_configured());
    }
}
