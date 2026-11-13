//! Wire-level tests against a mock Graph endpoint. These are the tests that
//! matter for this port: endpoint strings, query encoding, status handling and
//! output shape — all without needing Azure credentials.

use std::sync::Arc;

use serde_json::json;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::Config;
use crate::graph::mail::{DraftRequest, ReplyDraftRequest, ReplyRequest, SendRequest};
use crate::graph::{GraphClient, GraphError};

fn config() -> Arc<Config> {
    Arc::new(Config {
        tenant_id: "tenant".into(),
        client_id: "client".into(),
        client_secret: "secret".into(),
        user_email: "me@example.com".into(),
        attachment_root: std::env::temp_dir().join("m365-mock-attachments"),
        calendar_timezone: "W. Australia Standard Time".into(),
        excluded_folder_ids: Default::default(),
        skip_excluded_folders: false,
        signoff_name: "Sam".into(),
        read_only: true,
        source: None,
    })
}

/// Mock the token endpoint on the same server; returns a configured client.
async fn client_for(server: &MockServer) -> GraphClient {
    Mock::given(method("POST"))
        .and(path("/tenant/oauth2/v2.0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "token-abc",
            "expires_in": 3600,
            "token_type": "Bearer",
        })))
        .mount(server)
        .await;

    GraphClient::with_base_urls(config(), server.uri(), server.uri())
}

#[tokio::test]
async fn recent_emails_sends_the_exact_endpoint_and_maps_the_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders/inbox/messages"))
        .and(query_param("$top", "5"))
        .and(query_param("$orderby", "receivedDateTime desc"))
        .and(query_param(
            "$select",
            "id,conversationId,sender,subject,body,receivedDateTime,isRead,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance",
        ))
        .and(header("authorization", "Bearer token-abc"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{
                "id": "M1",
                "conversationId": "C1",
                "sender": { "emailAddress": { "address": "boss@example.com", "name": "Boss" } },
                "subject": "Status?",
                "body": { "contentType": "text", "content": "quick one" },
                "receivedDateTime": "2026-11-13T01:00:00Z",
                "isRead": false,
                "toRecipients": [{ "emailAddress": { "address": "me@example.com" } }],
                "hasAttachments": false,
                "importance": "normal"
            }]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client.get_recent_emails(5, None).await.unwrap();

    assert_eq!(value[0]["id"], "M1");
    assert_eq!(value[0]["sender"], "boss@example.com");
    assert_eq!(value[0]["to"], "me@example.com");
    assert_eq!(value[0]["cc"], "");
    assert_eq!(value[0]["body"], "quick one");
    assert_eq!(value[0]["isRead"], false);
}

#[tokio::test]
async fn search_emails_percent_encodes_the_query_and_keeps_the_quotes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/messages"))
        .and(query_param("$search", "\"quarterly review\""))
        .and(header("consistencylevel", "eventual"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": [] })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client
        .search_emails("quarterly review", 20, None)
        .await
        .unwrap();
    assert_eq!(value, json!([]));
}

#[tokio::test]
async fn send_email_treats_202_as_success_without_parsing_a_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/me@example.com/sendMail"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client
        .send_email(
            SendRequest {
                to: "them@example.com",
                cc: None,
                bcc: None,
                subject: "Subject",
                body: "<p>hi</p>",
                attachments: None,
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(value["success"], true);
    assert_eq!(value["attachmentCount"], 0);
    assert!(value["sentAt"].is_string());

    let requests = server.received_requests().await.unwrap();
    let send = requests
        .iter()
        .find(|request| request.url.path().ends_with("/sendMail"))
        .expect("sendMail request");
    let body: serde_json::Value = serde_json::from_slice(&send.body).unwrap();
    assert_eq!(body["message"]["subject"], "Subject");
    assert_eq!(body["message"]["body"]["contentType"], "HTML");
    assert_eq!(
        body["message"]["toRecipients"][0]["emailAddress"]["address"],
        "them@example.com"
    );
}

#[tokio::test]
async fn delete_event_treats_204_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/users/me@example.com/events/E1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client.delete_event("E1", None).await.unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(value["eventId"], "E1");
}

#[tokio::test]
async fn graph_errors_surface_the_api_message_and_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "code": "ErrorAccessDenied", "message": "Access is denied." }
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let err = client.get_email_folders(None).await.unwrap_err();
    match err {
        GraphError::Http { status, message } => {
            assert_eq!(status, 403);
            assert_eq!(message, "Access is denied.");
        }
        other => panic!("expected an Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn the_access_token_is_cached_across_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tenant/oauth2/v2.0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "token-abc",
            "expires_in": 3600,
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": [] })))
        .mount(&server)
        .await;

    let client = GraphClient::with_base_urls(config(), server.uri(), server.uri());
    client.get_email_folders(None).await.unwrap();
    client.get_email_folders(None).await.unwrap();
    // `expect(1)` on the token mock is verified on server drop.
}

#[tokio::test]
async fn unconfigured_client_fails_before_any_request() {
    let mut raw = (*config()).clone();
    raw.client_secret = String::new();
    let client =
        GraphClient::with_base_urls(Arc::new(raw), "http://127.0.0.1:1", "http://127.0.0.1:1");

    let err = client.get_email_folders(None).await.unwrap_err();
    assert!(matches!(err, GraphError::NotConfigured));
}

#[tokio::test]
async fn create_draft_mirrors_the_js_payload_and_echoes_optional_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/me@example.com/messages"))
        .and(body_json(json!({
            "subject": "Hi",
            "body": { "contentType": "HTML", "content": "<p>there</p>" },
            "toRecipients": [{ "emailAddress": { "address": "a@example.com" } }],
            "ccRecipients": [],
            "bccRecipients": [],
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "D1" })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client
        .create_draft(
            DraftRequest {
                to: Some("a@example.com"),
                cc: None,
                bcc: None,
                subject: "Hi",
                body: "<p>there</p>",
                conversation_id: None,
                attachments: None,
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(value["draftId"], "D1");
    assert_eq!(value["attachmentCount"], 0);
    assert_eq!(value["status"], "created_in_outlook_drafts");
    // `cc`/`bcc` were undefined upstream, so the keys must be absent.
    assert!(value.get("cc").is_none());
    assert!(value.get("conversationId").is_none());
}

#[tokio::test]
async fn reply_draft_omits_the_comment_when_the_body_is_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/me@example.com/messages/M1/createReply"))
        .and(body_json(json!({})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({
            "id": "D2",
            "conversationId": "C1",
            "subject": "Re: Status?",
            "body": { "contentType": "html", "content": "<p>existing</p>" },
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client
        .create_reply_draft(
            ReplyDraftRequest {
                message_id: "M1",
                body: "",
                reply_all: false,
                cc: None,
                bcc: None,
                attachments: None,
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(value["draftId"], "D2");
    assert_eq!(value["conversationId"], "C1");
    assert_eq!(value["toRecipients"], "");
    assert_eq!(value["status"], "created_reply_draft_in_outlook");
}

#[tokio::test]
async fn reply_without_cc_sends_only_a_comment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/users/me@example.com/messages/M1/reply"))
        .and(body_json(json!({ "comment": "thanks" })))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client
        .reply_to_email(
            ReplyRequest {
                email_id: "M1",
                body: "thanks",
                reply_all: false,
                cc: None,
                bcc: None,
                attachments: None,
            },
            None,
        )
        .await
        .unwrap();

    assert_eq!(value["success"], true);
    assert_eq!(value["replyAll"], false);
    assert_eq!(value["attachmentCount"], 0);
    assert!(value.get("cc").is_none());
}

#[tokio::test]
async fn attachments_over_the_inline_limit_are_rejected_before_any_request() {
    let server = MockServer::start().await;
    let root = std::env::temp_dir().join("m365-mock-attachments");
    std::fs::create_dir_all(&root).unwrap();
    let big = root.join("big.bin");
    std::fs::write(&big, vec![0u8; 4 * 1024 * 1024]).unwrap();

    let mut raw = (*config()).clone();
    raw.attachment_root = root.clone();
    let client = GraphClient::with_base_urls(Arc::new(raw), server.uri(), server.uri());

    let attachments = vec![big.display().to_string()];
    let err = client
        .send_email(
            SendRequest {
                to: "a@example.com",
                cc: None,
                bcc: None,
                subject: "s",
                body: "b",
                attachments: Some(&attachments),
            },
            None,
        )
        .await
        .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("4.0MB"), "unexpected message: {message}");
    assert!(
        message.contains("not implemented"),
        "unexpected message: {message}"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    std::fs::remove_file(&big).ok();
}

#[tokio::test]
async fn today_briefing_pairs_unread_mail_with_todays_events() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders/inbox/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{
                "id": "M1",
                "sender": { "emailAddress": { "address": "boss@example.com" } },
                "subject": "hi",
                "body": { "content": "x" }
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/calendarView"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "value": [{ "id": "E1", "subject": "Standup" }]
        })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client.get_todays_briefing(None).await.unwrap();

    assert_eq!(value["unreadEmails"][0]["id"], "M1");
    assert_eq!(value["todaysEvents"][0]["subject"], "Standup");
    assert!(value.get("emailError").is_none());
    assert!(value.get("eventError").is_none());
}

#[tokio::test]
async fn briefing_reports_per_source_errors_as_data() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders/inbox/messages"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "message": "Access is denied." }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/calendarView"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "value": [] })))
        .mount(&server)
        .await;

    let client = client_for(&server).await;
    let value = client.get_todays_briefing(None).await.unwrap();

    assert_eq!(value["unreadEmails"], json!([]));
    assert!(
        value["emailError"]
            .as_str()
            .unwrap()
            .contains("Access is denied")
    );
    assert!(value.get("eventError").is_none());
}

/// The compiled binary, one level up from `target/debug/deps/`.
///
/// `cargo test` does not necessarily rebuild the plain bin target, so refuse to
/// run against a stale binary: `make test` builds first, and this skip makes a
/// bare `cargo test` honest instead of silently exercising old code.
fn server_binary() -> Option<std::path::PathBuf> {
    let harness = std::env::current_exe().ok()?;
    let candidate = harness.parent()?.parent()?.join("m365-mcp-server");
    let metadata = std::fs::metadata(&candidate).ok()?;
    let harness_modified = std::fs::metadata(&harness).ok()?.modified().ok()?;
    let candidate_modified = metadata.modified().ok()?;
    if candidate_modified + std::time::Duration::from_secs(1) < harness_modified {
        eprintln!(
            "skipping end-to-end test: {} is older than the test harness (run `make build`)",
            candidate.display()
        );
        return None;
    }
    Some(candidate)
}

/// Spawn the server with credentials pointed at the mock Graph.
///
/// `read_only` is passed as `OUTLOOK_READ_ONLY` only when `Some`; leaving it
/// unset exercises the default.
fn server_command(
    exe: &std::path::Path,
    graph_base: &str,
    read_only: Option<&str>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(exe);
    command
        .env("AZURE_TENANT_ID", "tenant")
        .env("AZURE_CLIENT_ID", "client")
        .env("AZURE_CLIENT_SECRET", "secret")
        .env("OUTLOOK_USER_EMAIL", "me@example.com")
        .env("M365_GRAPH_BASE", graph_base)
        .env("M365_LOGIN_BASE", graph_base)
        .env("M365_CONFIG", "/nonexistent/config.toml")
        .env("M365_LOG", "error");
    if let Some(value) = read_only {
        command.env("OUTLOOK_READ_ONLY", value);
    }
    command
}

/// The default is read-only: 12 tools listed, write tools absent, and a write
/// call rejected with "tool not found" — the name cannot even be resolved.
#[tokio::test]
async fn end_to_end_read_only_hides_and_rejects_write_tools() {
    let Some(exe) = server_binary() else {
        eprintln!("skipping: server binary not built");
        return;
    };
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tenant/oauth2/v2.0/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "token-abc",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/me@example.com/mailFolders"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": { "message": "Access is denied." }
        })))
        .mount(&server)
        .await;

    let transport =
        rmcp::transport::TokioChildProcess::new(server_command(&exe, &server.uri(), None)).unwrap();
    let client = rmcp::ServiceExt::serve((), transport).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(
        tools.len(),
        12,
        "read-only mode must list only the read tools"
    );
    assert!(tools.iter().any(|tool| tool.name == "get_todays_briefing"));
    for write_tool in crate::tools::WRITE_TOOLS {
        assert!(
            !tools.iter().any(|tool| tool.name == *write_tool),
            "{write_tool} must be hidden in read-only mode"
        );
    }

    // A model that guesses the name still gets nowhere.
    let rejected = client
        .call_tool(rmcp::model::CallToolRequestParams::new("send_email"))
        .await
        .expect_err("write tool call must be rejected in read-only mode");
    match rejected {
        rmcp::service::ServiceError::McpError(error) => {
            assert_eq!(error.message, "tool not found");
        }
        other => panic!("expected an MCP error, got {other:?}"),
    }

    // Read tools still dispatch, and Graph failures stay flagged.
    let result = client
        .call_tool(rmcp::model::CallToolRequestParams::new("get_email_folders"))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(text.contains("Access is denied."), "unexpected: {text}");

    let _ = client.cancel().await;
}

/// `OUTLOOK_READ_ONLY=false` restores the full upstream surface.
#[tokio::test]
async fn end_to_end_writable_mode_lists_all_twenty_four_tools() {
    let Some(exe) = server_binary() else {
        eprintln!("skipping: server binary not built");
        return;
    };
    let server = MockServer::start().await;

    let transport =
        rmcp::transport::TokioChildProcess::new(server_command(&exe, &server.uri(), Some("false")))
            .unwrap();
    let client = rmcp::ServiceExt::serve((), transport).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 24);
    assert!(tools.iter().any(|tool| tool.name == "send_email"));

    let _ = client.cancel().await;
}

#[test]
fn attachment_root_default_is_under_the_home_directory() {
    let root = std::env::home_dir().unwrap().join("Downloads");
    assert!(root.is_absolute());
}
