//! Microsoft Graph transport: client-credentials token cache, one `graph_fetch`,
//! and the query-string rules Graph actually requires.

pub mod attach;
pub mod calendar;
pub mod mail;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tracing::warn;

use crate::config::Config;

const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_LOGIN_BASE: &str = "https://login.microsoftonline.com";
const TOKEN_SKEW_MS: i64 = 60_000;

/// `encodeURIComponent`'s escape set. Graph's OData query syntax breaks if you
/// hand-roll this with form encoding (`+` for spaces) — see `enc`.
const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// Percent-encode a value the way the JS original does with `encodeURIComponent`.
pub fn enc(raw: &str) -> String {
    utf8_percent_encode(raw, URI_COMPONENT).to_string()
}

/// Double single quotes for OData literals. The JS original interpolates
/// conversation IDs raw; escaping is a deliberate hardening, not a behaviour change
/// for any well-formed Graph ID.
pub fn odata_quote(raw: &str) -> String {
    raw.replace('\'', "''")
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error(
        "Microsoft Graph is not configured — set tenant_id, client_id, client_secret and user_email (config file or AZURE_*/OUTLOOK_USER_EMAIL env vars)"
    )]
    NotConfigured,
    #[error("Could not obtain an access token: {0}")]
    Token(String),
    #[error("Graph API {status}: {message}")]
    Http { status: u16, message: String },
    #[error("Network error talking to Microsoft Graph: {0}")]
    Network(String),
    #[error("Unexpected response from Microsoft Graph: {0}")]
    Decode(String),
    #[error("{0}")]
    Message(String),
}

impl GraphError {
    pub fn message(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

struct CachedToken {
    value: String,
    expires_at_ms: i64,
}

pub struct GraphClient {
    http: reqwest::Client,
    cfg: Arc<Config>,
    token: Mutex<Option<CachedToken>>,
    /// Overridable only so tests can point at a mock; set M365_GRAPH_BASE /
    /// M365_LOGIN_BASE to redirect a real run.
    graph_base: String,
    login_base: String,
}

impl GraphClient {
    pub fn new(cfg: Arc<Config>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("m365-mcp-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build the HTTP client");
        let graph_base =
            std::env::var("M365_GRAPH_BASE").unwrap_or_else(|_| DEFAULT_GRAPH_BASE.to_string());
        let login_base =
            std::env::var("M365_LOGIN_BASE").unwrap_or_else(|_| DEFAULT_LOGIN_BASE.to_string());
        Self {
            http,
            cfg,
            token: Mutex::new(None),
            graph_base,
            login_base,
        }
    }

    #[cfg(test)]
    pub fn with_base_urls(
        cfg: Arc<Config>,
        graph_base: impl Into<String>,
        login_base: impl Into<String>,
    ) -> Self {
        let mut client = Self::new(cfg);
        client.graph_base = graph_base.into();
        client.login_base = login_base.into();
        client
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    async fn access_token(&self) -> Result<String, GraphError> {
        {
            let guard = self.token.lock().await;
            if let Some(cached) = guard.as_ref()
                && Utc::now().timestamp_millis() < cached.expires_at_ms - TOKEN_SKEW_MS
            {
                return Ok(cached.value.clone());
            }
        }

        if !self.cfg.is_configured() {
            return Err(GraphError::NotConfigured);
        }

        let url = format!(
            "{}/{}/oauth2/v2.0/token",
            self.login_base, self.cfg.tenant_id
        );
        let form = [
            ("client_id", self.cfg.client_id.as_str()),
            ("client_secret", self.cfg.client_secret.as_str()),
            ("scope", "https://graph.microsoft.com/.default"),
            ("grant_type", "client_credentials"),
        ];

        let response = self
            .http
            .post(&url)
            .form(&form)
            .send()
            .await
            .map_err(|err| {
                warn!(error = %err, "token request failed");
                GraphError::Network(err.to_string())
            })?;

        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);

        if status.is_success()
            && let Some(token) = body.get("access_token").and_then(Value::as_str)
        {
            let expires_in = body
                .get("expires_in")
                .and_then(Value::as_i64)
                .unwrap_or(3600);
            let mut guard = self.token.lock().await;
            *guard = Some(CachedToken {
                value: token.to_string(),
                expires_at_ms: Utc::now().timestamp_millis() + expires_in * 1000,
            });
            return Ok(token.to_string());
        }

        let detail = body
            .get("error_description")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("no error_description returned");
        warn!(
            status = status.as_u16(),
            detail, "token endpoint rejected the request"
        );
        Err(GraphError::Token(format!(
            "HTTP {} — {detail}",
            status.as_u16()
        )))
    }

    /// The single Graph entry point. `endpoint` is either a path relative to
    /// `/users/{mailbox}` or an absolute URL (upload-session style).
    ///
    /// Non-error results are always JSON. 202/204 short-circuit to
    /// `{"success": true}` **before** any body parse — every send/move/cancel
    /// depends on that (JS original: graphClient.js:170-175).
    pub async fn graph_fetch(
        &self,
        endpoint: &str,
        method: Method,
        body: Option<&Value>,
        extra_headers: &[(&str, &str)],
        user_email: Option<&str>,
    ) -> Result<Value, GraphError> {
        let token = self.access_token().await?;
        let target = user_email
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.cfg.user_email.clone());

        if target.is_empty() {
            return Err(GraphError::NotConfigured);
        }

        let url = if endpoint.starts_with("http") {
            endpoint.to_string()
        } else {
            format!("{}/users/{target}{endpoint}", self.graph_base)
        };

        let mut request = self
            .http
            .request(method.clone(), &url)
            .bearer_auth(&token)
            .header(CONTENT_TYPE, "application/json");
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }
        if let Some(payload) = body {
            request = request.body(payload.to_string());
        }

        let response = request.send().await.map_err(|err| {
            warn!(%method, endpoint, error = %err, "graph request failed");
            GraphError::Network(err.to_string())
        })?;

        let status = response.status();
        if status.is_success() {
            if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
                return Ok(json!({ "success": true }));
            }
            let text = response.text().await.unwrap_or_default();
            if text.trim().is_empty() {
                return Ok(json!({ "success": true }));
            }
            return serde_json::from_str::<Value>(&text).map_err(|err| {
                GraphError::Decode(format!("{} (status {})", err, status.as_u16()))
            });
        }

        let text = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| {
                if text.trim().is_empty() {
                    format!("HTTP {}", status.as_u16())
                } else {
                    text.chars().take(300).collect()
                }
            });

        warn!(%method, endpoint, status = status.as_u16(), message, "graph request rejected");
        Err(GraphError::Http {
            status: status.as_u16(),
            message,
        })
    }

    pub(crate) async fn get(
        &self,
        endpoint: &str,
        user_email: Option<&str>,
    ) -> Result<Value, GraphError> {
        self.graph_fetch(endpoint, Method::GET, None, &[], user_email)
            .await
    }

    pub(crate) async fn post(
        &self,
        endpoint: &str,
        body: Option<&Value>,
        user_email: Option<&str>,
    ) -> Result<Value, GraphError> {
        self.graph_fetch(endpoint, Method::POST, body, &[], user_email)
            .await
    }

    pub(crate) async fn patch(
        &self,
        endpoint: &str,
        body: Option<&Value>,
        user_email: Option<&str>,
    ) -> Result<Value, GraphError> {
        self.graph_fetch(endpoint, Method::PATCH, body, &[], user_email)
            .await
    }

    pub(crate) async fn delete(
        &self,
        endpoint: &str,
        user_email: Option<&str>,
    ) -> Result<Value, GraphError> {
        self.graph_fetch(endpoint, Method::DELETE, None, &[], user_email)
            .await
    }
}

/// `{ "value": [...] }` envelope used by every Graph collection endpoint.
#[derive(Debug, Deserialize)]
pub struct Collection<T> {
    #[serde(default = "Vec::new")]
    pub value: Vec<T>,
}

pub fn into_collection<T: DeserializeOwned>(value: &Value) -> Result<Vec<T>, GraphError> {
    serde_json::from_value::<Collection<T>>(value.clone())
        .map(|collection| collection.value)
        .map_err(|err| GraphError::Decode(err.to_string()))
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct EmailAddress {
    pub address: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Recipient {
    pub email_address: Option<EmailAddress>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ItemBody {
    pub content: Option<String>,
    pub content_type: Option<String>,
}

/// JS joins recipient addresses with ", " and drops empties:
/// `.map(r => r.emailAddress?.address).filter(Boolean).join(', ')`
pub fn join_addresses(recipients: &Option<Vec<Recipient>>) -> String {
    recipients
        .as_ref()
        .map(|list| {
            list.iter()
                .filter_map(|recipient| recipient.email_address.as_ref())
                .filter_map(|address| address.address.as_deref())
                .filter(|address| !address.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Split a comma-separated recipient list into Graph `recipient` objects.
/// Mirrors `toRecipients()`: accepts one or many, trims, drops empties.
pub fn to_recipients(value: Option<&str>) -> Vec<Value> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(|address| json!({ "emailAddress": { "address": address } }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_encode_uri_component() {
        assert_eq!(enc("hello world"), "hello%20world");
        assert_eq!(enc("a@b.com"), "a%40b.com");
        // encodeURIComponent leaves these alone.
        assert_eq!(enc("a-b_c.d!e~f*g'h(i)"), "a-b_c.d!e~f*g'h(i)");
        assert_eq!(enc("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn odata_literals_double_quotes() {
        assert_eq!(odata_quote("abc'def"), "abc''def");
        assert_eq!(odata_quote("abc"), "abc");
    }

    #[test]
    fn recipients_split_and_trim() {
        let list = to_recipients(Some(" a@x.com , b@x.com ,,"));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["emailAddress"]["address"], "a@x.com");
        assert!(to_recipients(None).is_empty());
        assert!(to_recipients(Some("")).is_empty());
    }

    #[test]
    fn address_join_drops_empty_addresses() {
        let recipients = Some(vec![
            Recipient {
                email_address: Some(EmailAddress {
                    address: Some("a@x.com".into()),
                    name: None,
                }),
            },
            Recipient {
                email_address: Some(EmailAddress {
                    address: None,
                    name: Some("No Address".into()),
                }),
            },
            Recipient {
                email_address: None,
            },
            Recipient {
                email_address: Some(EmailAddress {
                    address: Some("".into()),
                    name: None,
                }),
            },
            Recipient {
                email_address: Some(EmailAddress {
                    address: Some("b@x.com".into()),
                    name: None,
                }),
            },
        ]);
        assert_eq!(join_addresses(&recipients), "a@x.com, b@x.com");
        assert_eq!(join_addresses(&None), "");
    }
}
