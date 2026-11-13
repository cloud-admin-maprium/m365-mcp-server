//! Delegated authentication via the Microsoft device-code flow.
//!
//! Produces a single-user access token (+ refresh token) that can only see the
//! mailbox of the account that completed consent. Tokens are persisted by
//! [`crate::token`]. Nothing here runs on the `serve` path: the server only ever
//! refreshes an existing token, so a missing token is a clear "run login" error.
//!
//! Device code was chosen over an authorization-code redirect because a local
//! stdio server has nowhere convenient to host a callback; the only setup cost
//! is ticking "Allow public client flows" on the app registration.

use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use serde_json::Value;

use crate::config::Config;
use crate::token::{self, StoredTokens};

const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const DEFAULT_LOGIN_BASE: &str = "https://login.microsoftonline.com";

/// Scopes requested for delegated access. Write scopes are omitted when the
/// server runs read-only, so the consent prompt matches what the tools can do.
/// `offline_access` is what yields a refresh token.
pub fn scopes(read_only: bool) -> String {
    let mut parts = vec![
        "offline_access",
        "openid",
        "profile",
        "https://graph.microsoft.com/Mail.Read",
        "https://graph.microsoft.com/Calendars.Read",
    ];
    if !read_only {
        parts.push("https://graph.microsoft.com/Mail.ReadWrite");
        parts.push("https://graph.microsoft.com/Calendars.ReadWrite");
    }
    parts.join(" ")
}

/// Run the device-code flow to completion and persist the resulting tokens.
pub async fn device_code_login(cfg: &Config) -> Result<StoredTokens> {
    let login = std::env::var("M365_LOGIN_BASE").unwrap_or_else(|_| DEFAULT_LOGIN_BASE.to_string());
    let http = reqwest::Client::new();
    let scope = scopes(cfg.read_only);

    let start: Value = {
        let mut form: Vec<(&str, &str)> = vec![
            ("client_id", cfg.client_id.as_str()),
            ("scope", scope.as_str()),
        ];
        if let Some(secret) = cfg.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        tracing::debug!(
            fields = form.len(),
            has_client_secret = form.iter().any(|(k, _)| *k == "client_secret"),
            "device code request form"
        );
        let response = http
            .post(format!("{login}/{}/oauth2/v2.0/devicecode", cfg.tenant_id))
            .form(&form)
            .send()
            .await
            .context("requesting a device code")?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = body
                .get("error_description")
                .or_else(|| body.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("no error_description returned");
            anyhow::bail!(
                "device-code request failed (HTTP {}): {detail}",
                status.as_u16()
            );
        }
        body
    };

    let device_code = start["device_code"]
        .as_str()
        .context("device-code response had no device_code")?;
    let message = start["message"].as_str().unwrap_or("");
    let interval = start["interval"].as_u64().unwrap_or(5).max(1);
    let expires_in = start["expires_in"].as_u64().unwrap_or(900);

    // Human-facing output goes to stderr: stdout belongs to the JSON-RPC serve path.
    eprintln!("\n{message}\n\nWaiting for sign-in (up to {expires_in}s)...");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    loop {
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("sign-in timed out — run `m365-mcp-server login` again");
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", DEVICE_CODE_GRANT),
            ("client_id", cfg.client_id.as_str()),
            ("device_code", device_code),
        ];
        if let Some(secret) = cfg.client_secret.as_deref() {
            form.push(("client_secret", secret));
        }
        let encoded: String = serde_urlencoded::to_string(&form)
            .unwrap_or_default();
        tracing::debug!(body = encoded, "token poll encoded body");

        let response = http
            .post(format!("{login}/{}/oauth2/v2.0/token", cfg.tenant_id))
            .form(&form)
            .send()
            .await
            .context("polling the token endpoint")?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);

        if status.is_success() {
            return persist(cfg, &body);
        }

        match body["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                tokio::time::sleep(Duration::from_secs(interval)).await;
                continue;
            }
            Some(other) => {
                let detail = body["error_description"].as_str().unwrap_or(other);
                anyhow::bail!("sign-in failed: {detail}");
            }
            None => anyhow::bail!("sign-in failed: HTTP {}", status.as_u16()),
        }
    }
}

fn persist(cfg: &Config, body: &Value) -> Result<StoredTokens> {
    let access_token = body["access_token"]
        .as_str()
        .context("token response had no access_token")?
        .to_string();
    let refresh_token = body["refresh_token"].as_str().map(str::to_string);
    let expires_in = body["expires_in"].as_i64().unwrap_or(3600);
    let scope = body["scope"].as_str().map(str::to_string);
    let account = body["id_token"].as_str().and_then(upn_from_id_token);

    let tokens = StoredTokens {
        access_token,
        refresh_token,
        expires_at_ms: chrono::Utc::now().timestamp_millis() + expires_in * 1000,
        scope,
        account: account.clone(),
    };
    token::save(&cfg.token_path, &tokens).context("persisting tokens")?;

    // Surface the identity and flag a mismatch early. `graph_fetch` enforces the
    // binding per request, but failing loudly at login is friendlier.
    match &account {
        Some(upn)
            if !cfg.user_email.trim().is_empty()
                && !upn.eq_ignore_ascii_case(cfg.user_email.trim()) =>
        {
            eprintln!(
                "warning: signed in as {upn}, but user_email is {} — set user_email to the signed-in account",
                cfg.user_email
            );
        }
        Some(upn) => eprintln!("signed in as {upn}"),
        None => eprintln!("signed in (tokens saved to {})", cfg.token_path.display()),
    }

    Ok(tokens)
}

/// Pull `preferred_username` out of the id_token JWT payload. No signature check
/// is needed: the token came straight from the tenant's token endpoint.
fn upn_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = decode_b64url(payload)?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    claims["preferred_username"]
        .as_str()
        .or_else(|| claims["upn"].as_str())
        .map(str::to_string)
}

/// Decode a JWT segment, tolerating missing base64url padding.
fn decode_b64url(raw: &str) -> Option<Vec<u8>> {
    if let Ok(bytes) = URL_SAFE_NO_PAD.decode(raw) {
        return Some(bytes);
    }
    let mut padded = raw.to_string();
    while !padded.len().is_multiple_of(4) {
        padded.push('=');
    }
    URL_SAFE.decode(padded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_scopes_have_no_write_permissions() {
        let read = scopes(true);
        assert!(read.contains("Mail.Read"));
        assert!(!read.contains("Mail.ReadWrite"));
        assert!(read.contains("offline_access"));
    }

    #[test]
    fn writable_scopes_add_write_permissions() {
        let write = scopes(false);
        assert!(write.contains("Mail.ReadWrite"));
        assert!(write.contains("Calendars.ReadWrite"));
    }

    #[test]
    fn id_token_upn_is_extracted() {
        // {"preferred_username":"me@example.com"} base64url, unsigned segment.
        let payload = "eyJwcmVmZXJyZWRfdXNlcm5hbWUiOiJtZUBleGFtcGxlLmNvbSJ9";
        let token = format!("header.{payload}.sig");
        assert_eq!(upn_from_id_token(&token).as_deref(), Some("me@example.com"));
    }
}
