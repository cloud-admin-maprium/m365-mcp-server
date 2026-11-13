//! The 24 MCP tools. Names, descriptions and parameter docs match the JS
//! original (`src/mcp/outlookServer.js`) so client configs and skills keep working.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::schemars::JsonSchema;
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::Value;

use crate::graph::GraphClient;
use crate::graph::GraphError;
use crate::graph::calendar::{EventRequest, EventUpdates};
use crate::graph::mail::{DraftRequest, DraftUpdate, ReplyDraftRequest, ReplyRequest, SendRequest};
use crate::out::{put_str, tool_err, tool_ok};
use crate::triage::{self, TriageInput};

const DRAFT_DISPLAY_INSTRUCTION: &str = "Render the full draft contents (From, To, CC, Subject, and full Body text) in the chat response — do not return an uninformative one-liner like \"draft created\".";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRecentEmailsParams {
    /// Max emails (default: 10)
    pub limit: Option<u32>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetAllFoldersParams {
    /// Max emails (default: 10)
    pub limit: Option<u32>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MailboxOnlyParams {
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThreadParams {
    /// Outlook Conversation ID
    pub conversation_id: String,
    pub limit: Option<u32>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchEmailsParams {
    /// Search query
    pub query: String,
    pub limit: Option<u32>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SentEmailsParams {
    pub limit: Option<u32>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TriageEmailParams {
    pub id: String,
    pub sender: String,
    pub sender_name: Option<String>,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateDraftParams {
    pub to: String,
    /// Comma-separated CC email address(es)
    pub cc: Option<String>,
    /// Comma-separated BCC email address(es)
    pub bcc: Option<String>,
    pub subject: String,
    pub body: String,
    pub conversation_id: Option<String>,
    /// Absolute file paths under the configured attachment root (default ~/Downloads) to attach. Up to 3MB combined is supported inline; larger uploads are rejected by this build.
    pub attachments: Option<Vec<String>>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateReplyDraftParams {
    /// Outlook message ID of the email to reply to
    pub message_id: String,
    /// Text/HTML reply content to populate in the draft
    pub body: Option<String>,
    /// Reply to all recipients (default: false)
    pub reply_all: Option<bool>,
    /// Comma-separated CC email address(es)
    pub cc: Option<String>,
    /// Comma-separated BCC email address(es)
    pub bcc: Option<String>,
    /// Absolute file paths under the configured attachment root (default ~/Downloads) to attach. Up to 3MB combined is supported inline; larger uploads are rejected by this build.
    pub attachments: Option<Vec<String>>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDraftParams {
    /// Outlook draft message ID
    pub draft_id: String,
    /// Updated body content (HTML)
    pub body: Option<String>,
    /// Updated subject line
    pub subject: Option<String>,
    /// Updated recipient email address
    pub to: Option<String>,
    /// Comma-separated CC email address(es)
    pub cc: Option<String>,
    /// Comma-separated BCC email address(es)
    pub bcc: Option<String>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendDraftParams {
    /// Outlook draft message ID to send
    pub draft_id: String,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplyToEmailParams {
    pub email_id: String,
    pub body: String,
    pub reply_all: Option<bool>,
    /// Comma-separated CC email address(es)
    pub cc: Option<String>,
    /// Comma-separated BCC email address(es)
    pub bcc: Option<String>,
    /// Absolute file paths under the configured attachment root (default ~/Downloads) to attach. Up to 3MB combined is supported inline; larger uploads are rejected by this build.
    pub attachments: Option<Vec<String>>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendEmailParams {
    pub to: String,
    /// Comma-separated CC email address(es)
    pub cc: Option<String>,
    /// Comma-separated BCC email address(es)
    pub bcc: Option<String>,
    pub subject: String,
    pub body: String,
    /// Absolute file paths under the configured attachment root (default ~/Downloads) to attach. Up to 3MB combined is supported inline; larger uploads are rejected by this build.
    pub attachments: Option<Vec<String>>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MoveEmailParams {
    pub email_id: String,
    pub destination_folder_id: String,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EmailAttachmentsParams {
    /// Outlook/Graph message ID
    pub id: String,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkEmailReadParams {
    pub email_id: String,
    pub is_read: Option<bool>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpcomingEventsParams {
    pub days: Option<i64>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventsParams {
    /// ISO 8601 start
    pub start_time: String,
    /// ISO 8601 end
    pub end_time: String,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateEventParams {
    pub subject: String,
    pub start_time: String,
    pub end_time: String,
    pub attendees: Option<Vec<String>>,
    pub location: Option<String>,
    pub body: Option<String>,
    /// Mark the event private — hides subject, location, and attendees from anyone with shared calendar access except delegates with the private-item override. Maps to Graph API sensitivity=private.
    pub is_private: Option<bool>,
    /// Skip the conflict check and create anyway. Only set true after explicit user approval to double-book.
    pub force: Option<bool>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEventParams {
    pub event_id: String,
    pub subject: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub location: Option<String>,
    pub body: Option<String>,
    /// Full replacement list of attendee email addresses.
    pub attendees: Option<Vec<String>>,
    /// Mark the event private — hides subject, location, and attendees from anyone with shared calendar access except delegates with the private-item override. Maps to Graph API sensitivity=private.
    pub is_private: Option<bool>,
    /// Skip the conflict check and update anyway. Only set true after explicit user approval to double-book.
    pub force: Option<bool>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CancelEventParams {
    pub event_id: String,
    pub comment: Option<String>,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEventParams {
    pub event_id: String,
    /// Target Outlook email address / mailbox. Defaults to primary configured user email if omitted.
    pub mailbox: Option<String>,
}

/// Tools that mutate the mailbox or calendar. Hidden and rejected in read-only
/// mode, which is the default.
pub const WRITE_TOOLS: &[&str] = &[
    "create_email_draft",
    "create_reply_draft",
    "update_email_draft",
    "send_draft",
    "reply_to_email",
    "send_email",
    "move_email",
    "mark_email_read",
    "create_calendar_event",
    "update_calendar_event",
    "cancel_calendar_event",
    "delete_calendar_event",
];

/// Every read tool, in the order the docs list them. Used by the tests to pin
/// the read-only surface; keep it in sync with `#[tool]` definitions.
#[cfg(test)]
const READ_TOOLS: &[&str] = &[
    "get_recent_emails",
    "get_recent_emails_all_folders",
    "get_unread_emails",
    "get_conversation_thread",
    "search_emails",
    "get_sent_emails",
    "get_email_folders",
    "get_email_attachments",
    "triage_email",
    "get_upcoming_events",
    "get_calendar_events",
    "get_todays_briefing",
];

/// Free function (not a method) so tests can build both routers without a
/// `GraphClient`.
fn build_router(read_only: bool) -> ToolRouter<McpService> {
    let mut router = McpService::tool_router();
    if read_only {
        for name in WRITE_TOOLS {
            router.disable_route(*name);
        }
    }
    router
}

#[derive(Clone)]
pub struct McpService {
    client: Arc<GraphClient>,
    read_only: bool,
}

impl McpService {
    pub fn new(client: Arc<GraphClient>, read_only: bool) -> Self {
        Self { client, read_only }
    }

    /// The single chokepoint for read-only enforcement: a disabled route is
    /// absent from `tools/list` *and* rejected by `tools/call`, so a model that
    /// guesses a write tool name still cannot reach it.
    fn router(&self) -> ToolRouter<Self> {
        build_router(self.read_only)
    }

    /// `withDraftDisplay()` in the original: draft tools must hand the model the
    /// full draft, not a one-liner.
    fn draft_display(
        &self,
        result: Result<Value, GraphError>,
        mailbox: Option<&str>,
    ) -> CallToolResult {
        match result {
            Ok(Value::Object(mut map)) => {
                put_str(
                    &mut map,
                    "from",
                    mailbox.unwrap_or(self.client.config().user_email.as_str()),
                );
                put_str(
                    &mut map,
                    "instruction_for_assistant",
                    DRAFT_DISPLAY_INSTRUCTION,
                );
                tool_ok(Value::Object(map))
            }
            Ok(other) => tool_ok(other),
            Err(err) => tool_err(err.to_string()),
        }
    }
}

fn finish(result: Result<Value, GraphError>) -> CallToolResult {
    match result {
        Ok(value) => tool_ok(value),
        Err(err) => tool_err(err.to_string()),
    }
}

#[tool_router]
impl McpService {
    #[tool(description = "Fetch recently received emails from Outlook inbox")]
    async fn get_recent_emails(
        &self,
        Parameters(params): Parameters<GetRecentEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_recent_emails(params.limit.unwrap_or(10), params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(
        description = "Fetch recently received emails across the entire mailbox (all folders, not just Inbox), excluding Deleted Items (and its subfolders), Junk, Junk Emails, and Drafts. Returns bodyPreview (short text snippet) instead of full HTML body to keep payload small."
    )]
    async fn get_recent_emails_all_folders(
        &self,
        Parameters(params): Parameters<GetAllFoldersParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_recent_emails_all_folders(
                    params.limit.unwrap_or(10),
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Fetch all unread emails from Outlook inbox for triage")]
    async fn get_unread_emails(
        &self,
        Parameters(params): Parameters<MailboxOnlyParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_unread_emails(params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "Retrieve full email conversation history by conversation ID")]
    async fn get_conversation_thread(
        &self,
        Parameters(params): Parameters<ConversationThreadParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_conversation_thread(
                    &params.conversation_id,
                    params.limit.unwrap_or(10),
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Search emails by keyword, sender, subject or topic")]
    async fn search_emails(
        &self,
        Parameters(params): Parameters<SearchEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .search_emails(
                    &params.query,
                    params.limit.unwrap_or(20),
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Fetch sent emails to track outbox and detect missing replies")]
    async fn get_sent_emails(
        &self,
        Parameters(params): Parameters<SentEmailsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_sent_emails(params.limit.unwrap_or(25), params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "List all Outlook mail folders with unread counts")]
    async fn get_email_folders(
        &self,
        Parameters(params): Parameters<MailboxOnlyParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_email_folders(params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "Classify an email for urgency, category and suggest a reply")]
    async fn triage_email(
        &self,
        Parameters(params): Parameters<TriageEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(tool_ok(triage::triage(
            TriageInput {
                id: &params.id,
                sender: &params.sender,
                sender_name: params.sender_name.as_deref(),
                subject: &params.subject,
                body: &params.body,
            },
            self.client.config().signoff_name.as_str(),
        )))
    }

    #[tool(
        description = "Create a standalone draft email in Outlook Drafts folder for review before sending. Attaches up to 3MB combined (larger uploads are not supported in this build)."
    )]
    async fn create_email_draft(
        &self,
        Parameters(params): Parameters<CreateDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .client
            .create_draft(
                DraftRequest {
                    to: Some(&params.to),
                    cc: params.cc.as_deref(),
                    bcc: params.bcc.as_deref(),
                    subject: &params.subject,
                    body: &params.body,
                    conversation_id: params.conversation_id.as_deref(),
                    attachments: params.attachments.as_deref(),
                },
                params.mailbox.as_deref(),
            )
            .await;
        Ok(self.draft_display(result, params.mailbox.as_deref()))
    }

    #[tool(
        description = "Create an unsent draft reply attached to an existing email thread using Microsoft Graph createReply API (POST /messages/{id}/createReply). Attaches up to 3MB combined (larger uploads are not supported in this build)."
    )]
    async fn create_reply_draft(
        &self,
        Parameters(params): Parameters<CreateReplyDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        let body = params.body.as_deref().unwrap_or_default();
        let result = self
            .client
            .create_reply_draft(
                ReplyDraftRequest {
                    message_id: &params.message_id,
                    body,
                    reply_all: params.reply_all.unwrap_or(false),
                    cc: params.cc.as_deref(),
                    bcc: params.bcc.as_deref(),
                    attachments: params.attachments.as_deref(),
                },
                params.mailbox.as_deref(),
            )
            .await;
        Ok(self.draft_display(result, params.mailbox.as_deref()))
    }

    #[tool(
        description = "Update/edit an existing email draft subject, body, or recipient before sending (PATCH /messages/{draftId})"
    )]
    async fn update_email_draft(
        &self,
        Parameters(params): Parameters<UpdateDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .client
            .update_draft(
                DraftUpdate {
                    draft_id: &params.draft_id,
                    subject: params.subject.as_deref(),
                    body: params.body.as_deref(),
                    to: params.to.as_deref(),
                    cc: params.cc.as_deref(),
                    bcc: params.bcc.as_deref(),
                },
                params.mailbox.as_deref(),
            )
            .await;
        Ok(self.draft_display(result, params.mailbox.as_deref()))
    }

    #[tool(
        description = "Send an existing draft email from Outlook Drafts folder by its draftId. Supports optional mailbox parameter."
    )]
    async fn send_draft(
        &self,
        Parameters(params): Parameters<SendDraftParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .send_draft(&params.draft_id, params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(
        description = "Reply to an existing email thread. Requires prior approval. Attaches up to 3MB combined (larger uploads are not supported in this build)."
    )]
    async fn reply_to_email(
        &self,
        Parameters(params): Parameters<ReplyToEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .reply_to_email(
                    ReplyRequest {
                        email_id: &params.email_id,
                        body: &params.body,
                        reply_all: params.reply_all.unwrap_or(false),
                        cc: params.cc.as_deref(),
                        bcc: params.bcc.as_deref(),
                        attachments: params.attachments.as_deref(),
                    },
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(
        description = "Send an approved email. Requires explicit approval from the client first. Attaches up to 3MB combined (larger uploads are not supported in this build)."
    )]
    async fn send_email(
        &self,
        Parameters(params): Parameters<SendEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .send_email(
                    SendRequest {
                        to: &params.to,
                        cc: params.cc.as_deref(),
                        bcc: params.bcc.as_deref(),
                        subject: &params.subject,
                        body: &params.body,
                        attachments: params.attachments.as_deref(),
                    },
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Move an email to a specific folder")]
    async fn move_email(
        &self,
        Parameters(params): Parameters<MoveEmailParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .move_email(
                    &params.email_id,
                    &params.destination_folder_id,
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(
        description = "Fetches the content of all the file attachments for a given Outlook messageId. Returns array [{id, name, contentType, size, isInline, contentBytes}]."
    )]
    async fn get_email_attachments(
        &self,
        Parameters(params): Parameters<EmailAttachmentsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_email_attachments(&params.id, params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "Mark an email as read or unread")]
    async fn mark_email_read(
        &self,
        Parameters(params): Parameters<MarkEmailReadParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .mark_email_read(
                    &params.email_id,
                    params.is_read.unwrap_or(true),
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Get upcoming calendar events for the next N days")]
    async fn get_upcoming_events(
        &self,
        Parameters(params): Parameters<UpcomingEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_upcoming_events(params.days.unwrap_or(7), params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "Get calendar events between two datetimes")]
    async fn get_calendar_events(
        &self,
        Parameters(params): Parameters<CalendarEventsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_calendar_events(
                    &params.start_time,
                    &params.end_time,
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(
        description = "Create a new calendar event. Requires approval. Automatically checks for conflicting events in that time range first and refuses (returning conflict:true and the conflicting events) unless force:true is passed — only pass force:true after the user has explicitly approved double-booking."
    )]
    async fn create_calendar_event(
        &self,
        Parameters(params): Parameters<CreateEventParams>,
    ) -> Result<CallToolResult, McpError> {
        let attendees = params.attendees.unwrap_or_default();
        Ok(finish(
            self.client
                .create_event(
                    EventRequest {
                        subject: &params.subject,
                        start_time: &params.start_time,
                        end_time: &params.end_time,
                        attendees: &attendees,
                        location: params.location.as_deref().unwrap_or_default(),
                        body: params.body.as_deref().unwrap_or_default(),
                        is_private: params.is_private.unwrap_or(false),
                        force: params.force.unwrap_or(false),
                    },
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(
        description = "Update an existing calendar event. If both startTime and endTime are being changed, automatically checks for conflicting events in the new time range first and refuses (returning conflict:true and the conflicting events) unless force:true is passed — only pass force:true after the user has explicitly approved double-booking."
    )]
    async fn update_calendar_event(
        &self,
        Parameters(params): Parameters<UpdateEventParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .update_event(
                    &params.event_id,
                    EventUpdates {
                        subject: params.subject.as_deref(),
                        start_time: params.start_time.as_deref(),
                        end_time: params.end_time.as_deref(),
                        location: params.location.as_deref(),
                        body: params.body.as_deref(),
                        attendees: params.attendees.as_deref(),
                        is_private: params.is_private,
                        force: params.force,
                    },
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(description = "Cancel a calendar event and notify attendees")]
    async fn cancel_calendar_event(
        &self,
        Parameters(params): Parameters<CancelEventParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .cancel_event(
                    &params.event_id,
                    params.comment.as_deref().unwrap_or_default(),
                    params.mailbox.as_deref(),
                )
                .await,
        ))
    }

    #[tool(
        description = "Permanently delete a calendar event from the calendar. Requires approval. If the mailbox is the organizer of a meeting, Outlook automatically sends a cancellation notice to attendees (without a custom message) — use cancel_calendar_event instead to include a comment."
    )]
    async fn delete_calendar_event(
        &self,
        Parameters(params): Parameters<DeleteEventParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .delete_event(&params.event_id, params.mailbox.as_deref())
                .await,
        ))
    }

    #[tool(description = "Get today's unread emails and calendar events in one call")]
    async fn get_todays_briefing(
        &self,
        Parameters(params): Parameters<MailboxOnlyParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(finish(
            self.client
                .get_todays_briefing(params.mailbox.as_deref())
                .await,
        ))
    }
}

#[tool_handler(router = self.router(), name = "OutlookMCPServer", version = "0.1.0")]
impl ServerHandler for McpService {}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(router: &ToolRouter<McpService>) -> Vec<String> {
        router
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    fn sorted(values: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = values.iter().map(|v| v.to_string()).collect();
        out.sort();
        out
    }

    #[test]
    fn the_read_surface_is_exactly_the_twelve_read_tools() {
        let listed = names(&build_router(true));
        assert_eq!(listed, sorted(READ_TOOLS));
        assert_eq!(listed.len(), 12);
        for write_tool in WRITE_TOOLS {
            assert!(
                !listed.contains(&write_tool.to_string()),
                "{write_tool} must not be listed in read-only mode"
            );
        }
    }

    #[test]
    fn disabling_read_only_restores_the_full_twenty_four() {
        let listed = names(&build_router(false));
        assert_eq!(listed.len(), 24);
        for write_tool in WRITE_TOOLS {
            assert!(
                listed.contains(&write_tool.to_string()),
                "{write_tool} must be listed when read_only is false"
            );
        }
    }

    #[test]
    fn write_and_read_lists_are_disjoint_and_cover_every_tool() {
        let all = sorted(
            &names(&build_router(false))
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let mut union = sorted(READ_TOOLS);
        union.extend(sorted(WRITE_TOOLS));
        union.sort();
        assert_eq!(
            union, all,
            "WRITE_TOOLS + READ_TOOLS must equal the tool set"
        );
        assert_eq!(WRITE_TOOLS.len(), 12);

        // No name appears in both halves.
        for read_tool in READ_TOOLS {
            assert!(
                !WRITE_TOOLS.contains(read_tool),
                "{read_tool} is classified as both read and write"
            );
        }
    }

    #[test]
    fn a_disabled_route_is_not_resolvable_by_name() {
        let read_only = build_router(true);
        for write_tool in WRITE_TOOLS {
            assert!(
                read_only.get(write_tool).is_none(),
                "{write_tool} must not resolve in read-only mode"
            );
            assert!(read_only.is_disabled(write_tool));
        }
        let writable = build_router(false);
        assert!(writable.get("send_email").is_some());
    }
}
