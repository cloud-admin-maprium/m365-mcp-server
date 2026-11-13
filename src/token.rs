//! On-disk storage for delegated OAuth tokens.
//!
//! One small JSON file per install, mode `0600`, holding the access token, the
//! refresh token and the expiry. Tokens are secrets: never log the values
//! (`Config::summary()` and this module are the only sanctioned describers).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Delegated OAuth tokens bound to a single signed-in user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Absolute expiry of `access_token`, milliseconds since the Unix epoch.
    pub expires_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// UPN of the signed-in user, when the id_token carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl StoredTokens {
    /// True while the access token is still usable after subtracting `skew_ms`.
    pub fn is_fresh(&self, now_ms: i64, skew_ms: i64) -> bool {
        now_ms < self.expires_at_ms - skew_ms
    }
}

/// Read the token file, returning `None` when it is absent or unreadable.
/// A corrupt file is treated as "no tokens": `login` rewrites it.
pub fn load(path: &Path) -> Option<StoredTokens> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Persist tokens, creating the parent directory and restricting the file to
/// `0600`. This is enforced, unlike the config file's advisory check.
pub fn save(path: &Path, tokens: &StoredTokens) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let data = serde_json::to_vec_pretty(tokens).context("serialising tokens")?;
    std::fs::write(path, data).with_context(|| format!("writing {}", path.display()))?;
    restrict(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting {}", path.display()))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_restricts_permissions() {
        let dir = std::env::temp_dir().join(format!("m365-token-test-{}", std::process::id()));
        let path = dir.join("tokens.json");
        let tokens = StoredTokens {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at_ms: 42,
            scope: Some("s".into()),
            account: Some("me@example.com".into()),
        };
        save(&path, &tokens).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.access_token, "a");
        assert_eq!(loaded.refresh_token.as_deref(), Some("r"));
        assert_eq!(loaded.account.as_deref(), Some("me@example.com"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn freshness_accounts_for_skew() {
        let t = StoredTokens {
            access_token: "a".into(),
            refresh_token: None,
            expires_at_ms: 10_000,
            scope: None,
            account: None,
        };
        assert!(t.is_fresh(5_000, 1_000));
        assert!(!t.is_fresh(9_500, 1_000));
    }
}
