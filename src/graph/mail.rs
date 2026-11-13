//! Mail operations. Every method maps one-to-one onto a tool in the JS original
//! (`graphClient.js`), including its endpoint strings and output key order.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::out::{now_iso, put_json, put_opt_bool, put_opt_str, put_or_empty, put_str};

use super::attach;
use super::{
    GraphClient, GraphError, ItemBody, Recipient, enc, into_collection, join_addresses,
    odata_quote, to_recipients,
};

const RECENT_SELECT: &str = "id,conversationId,sender,subject,body,receivedDateTime,isRead,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance";
const ALL_FOLDERS_SELECT: &str = "id,conversationId,sender,subject,bodyPreview,receivedDateTime,isRead,parentFolderId,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance";
const UNREAD_SELECT: &str = "id,conversationId,sender,subject,body,receivedDateTime,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance";
const THREAD_SELECT: &str = "id,sender,subject,body,sentDateTime,isDraft,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance";
const SEARCH_SELECT: &str = "id,conversationId,sender,subject,body,receivedDateTime,isRead,toRecipients,ccRecipients,bccRecipients,hasAttachments,importance";
const SENT_SELECT: &str = "id,conversationId,toRecipients,ccRecipients,bccRecipients,subject,body,sentDateTime,hasAttachments,importance";
const PREVIEW_CHARS: usize = 300;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    pub id: Option<String>,
    pub conversation_id: Option<String>,
    pub sender: Option<Recipient>,
    pub subject: Option<String>,
    pub body: Option<ItemBody>,
    pub body_preview: Option<String>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub has_attachments: Option<bool>,
    pub importance: Option<String>,
    pub parent_folder_id: Option<String>,
    pub to_recipients: Option<Vec<Recipient>>,
    pub cc_recipients: Option<Vec<Recipient>>,
    pub bcc_recipients: Option<Vec<Recipient>>,
}

impl Message {
    fn sender_address(&self) -> Option<String> {
        self.sender
            .as_ref()
            .and_then(|recipient| recipient.email_address.as_ref())
            .and_then(|address| address.address.clone())
            .filter(|address| !address.is_empty())
    }

    fn sender_name(&self) -> Option<String> {
        self.sender
            .as_ref()
            .and_then(|recipient| recipient.email_address.as_ref())
            .and_then(|address| address.name.clone())
            .filter(|name| !name.is_empty())
    }

    fn subject_value(&self) -> Option<String> {
        self.subject.clone().filter(|subject| !subject.is_empty())
    }

    fn body_content(&self) -> Option<String> {
        self.body
            .as_ref()
            .and_then(|body| body.content.clone())
            .filter(|content| !content.is_empty())
    }
}

/// One variant per read tool; each shape mirrors the corresponding JS mapper.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MailShape {
    Recent,
    AllFolders,
    Unread,
    Thread,
    Search,
    Sent,
}

fn preview_from_body(message: &Message) -> String {
    message
        .body_content()
        .unwrap_or_default()
        .chars()
        .take(PREVIEW_CHARS)
        .collect()
}

fn mail_item(message: &Message, shape: MailShape) -> Value {
    let mut item = Map::new();
    match shape {
        MailShape::Recent | MailShape::AllFolders | MailShape::Unread => {
            put_opt_str(&mut item, "id", message.id.clone());
            put_opt_str(&mut item, "conversationId", message.conversation_id.clone());
            put_str(
                &mut item,
                "sender",
                message.sender_address().unwrap_or_else(|| "unknown".into()),
            );
            put_str(
                &mut item,
                "senderName",
                message.sender_name().unwrap_or_else(|| "Unknown".into()),
            );
            put_str(
                &mut item,
                "subject",
                message
                    .subject_value()
                    .unwrap_or_else(|| "No Subject".into()),
            );
            if shape == MailShape::AllFolders {
                put_or_empty(&mut item, "bodyPreview", message.body_preview.clone());
            } else {
                put_or_empty(&mut item, "body", message.body_content());
            }
            put_opt_str(
                &mut item,
                "receivedDateTime",
                message.received_date_time.clone(),
            );
            if shape != MailShape::Unread {
                put_opt_bool(&mut item, "isRead", message.is_read);
            }
            put_str(&mut item, "to", join_addresses(&message.to_recipients));
            put_str(&mut item, "cc", join_addresses(&message.cc_recipients));
            put_str(&mut item, "bcc", join_addresses(&message.bcc_recipients));
            put_opt_bool(&mut item, "hasAttachments", message.has_attachments);
            put_opt_str(&mut item, "importance", message.importance.clone());
        }
        MailShape::Thread => {
            put_opt_str(&mut item, "id", message.id.clone());
            put_or_empty(&mut item, "sender", message.sender_address());
            put_or_empty(&mut item, "senderName", message.sender_name());
            put_or_empty(&mut item, "subject", message.subject_value());
            put_or_empty(&mut item, "body", message.body_content());
            put_opt_str(&mut item, "sentDateTime", message.sent_date_time.clone());
            put_opt_bool(&mut item, "isDraft", message.is_draft);
            put_str(&mut item, "to", join_addresses(&message.to_recipients));
            put_str(&mut item, "cc", join_addresses(&message.cc_recipients));
            put_str(&mut item, "bcc", join_addresses(&message.bcc_recipients));
            put_opt_bool(&mut item, "hasAttachments", message.has_attachments);
            put_opt_str(&mut item, "importance", message.importance.clone());
        }
        MailShape::Search => {
            put_opt_str(&mut item, "id", message.id.clone());
            put_opt_str(&mut item, "conversationId", message.conversation_id.clone());
            put_str(
                &mut item,
                "sender",
                message.sender_address().unwrap_or_else(|| "unknown".into()),
            );
            put_str(
                &mut item,
                "senderName",
                message.sender_name().unwrap_or_else(|| "Unknown".into()),
            );
            put_str(
                &mut item,
                "subject",
                message
                    .subject_value()
                    .unwrap_or_else(|| "No Subject".into()),
            );
            put_str(&mut item, "bodyPreview", preview_from_body(message));
            put_opt_str(
                &mut item,
                "receivedDateTime",
                message.received_date_time.clone(),
            );
            put_opt_bool(&mut item, "isRead", message.is_read);
            put_str(&mut item, "to", join_addresses(&message.to_recipients));
            put_str(&mut item, "cc", join_addresses(&message.cc_recipients));
            put_str(&mut item, "bcc", join_addresses(&message.bcc_recipients));
            put_opt_bool(&mut item, "hasAttachments", message.has_attachments);
            put_opt_str(&mut item, "importance", message.importance.clone());
        }
        MailShape::Sent => {
            put_opt_str(&mut item, "id", message.id.clone());
            put_opt_str(&mut item, "conversationId", message.conversation_id.clone());
            put_str(&mut item, "to", join_addresses(&message.to_recipients));
            put_str(&mut item, "cc", join_addresses(&message.cc_recipients));
            put_str(&mut item, "bcc", join_addresses(&message.bcc_recipients));
            put_str(
                &mut item,
                "subject",
                message
                    .subject_value()
                    .unwrap_or_else(|| "No Subject".into()),
            );
            put_str(&mut item, "bodyPreview", preview_from_body(message));
            put_opt_str(&mut item, "sentDateTime", message.sent_date_time.clone());
            put_opt_bool(&mut item, "hasAttachments", message.has_attachments);
            put_opt_str(&mut item, "importance", message.importance.clone());
        }
    }
    Value::Object(item)
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MailFolder {
    pub id: Option<String>,
    pub display_name: Option<String>,
    pub unread_item_count: Option<i64>,
    pub total_item_count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Attachment {
    #[serde(rename = "@odata.type")]
    pub odata_type: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: Option<bool>,
    pub content_bytes: Option<String>,
}

pub struct DraftRequest<'a> {
    pub to: Option<&'a str>,
    pub cc: Option<&'a str>,
    pub bcc: Option<&'a str>,
    pub subject: &'a str,
    pub body: &'a str,
    pub conversation_id: Option<&'a str>,
    pub attachments: Option<&'a [String]>,
}

pub struct DraftUpdate<'a> {
    pub draft_id: &'a str,
    pub subject: Option<&'a str>,
    pub body: Option<&'a str>,
    pub to: Option<&'a str>,
    pub cc: Option<&'a str>,
    pub bcc: Option<&'a str>,
}

pub struct ReplyRequest<'a> {
    pub email_id: &'a str,
    pub body: &'a str,
    pub reply_all: bool,
    pub cc: Option<&'a str>,
    pub bcc: Option<&'a str>,
    pub attachments: Option<&'a [String]>,
}

pub struct SendRequest<'a> {
    pub to: &'a str,
    pub cc: Option<&'a str>,
    pub bcc: Option<&'a str>,
    pub subject: &'a str,
    pub body: &'a str,
    pub attachments: Option<&'a [String]>,
}

pub struct ReplyDraftRequest<'a> {
    pub message_id: &'a str,
    pub body: &'a str,
    pub reply_all: bool,
    pub cc: Option<&'a str>,
    pub bcc: Option<&'a str>,
    pub attachments: Option<&'a [String]>,
}

impl GraphClient {
    async fn resolve_attachments(
        &self,
        requested: Option<&[String]>,
    ) -> Result<Vec<attach::ResolvedFile>, GraphError> {
        let files = attach::resolve(requested.unwrap_or(&[]), &self.config().attachment_root)?;
        attach::check_limits(&files)?;
        Ok(files)
    }

    pub async fn get_recent_emails(
        &self,
        limit: u32,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let endpoint = format!(
            "/mailFolders/inbox/messages?$orderby=receivedDateTime desc&$top={limit}&$select={RECENT_SELECT}"
        );
        let value = self.get(&endpoint, mailbox).await?;
        let messages: Vec<Message> = into_collection(&value)?;
        Ok(Value::Array(
            messages
                .iter()
                .map(|message| mail_item(message, MailShape::Recent))
                .collect(),
        ))
    }

    pub async fn get_recent_emails_all_folders(
        &self,
        limit: u32,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let overfetch = (limit * 3).max(30);
        let endpoint = format!(
            "/messages?$orderby=receivedDateTime desc&$top={overfetch}&$select={ALL_FOLDERS_SELECT}"
        );
        let value = self.get(&endpoint, mailbox).await?;
        let messages: Vec<Message> = into_collection(&value)?;
        let config = self.config();

        Ok(Value::Array(
            messages
                .iter()
                .filter(|message| {
                    !config.skip_excluded_folders
                        || message
                            .parent_folder_id
                            .as_ref()
                            .is_none_or(|folder| !config.excluded_folder_ids.contains(folder))
                })
                .take(limit as usize)
                .map(|message| mail_item(message, MailShape::AllFolders))
                .collect(),
        ))
    }

    pub async fn get_unread_emails(&self, mailbox: Option<&str>) -> Result<Value, GraphError> {
        let endpoint = format!(
            "/mailFolders/inbox/messages?$filter=isRead eq false&$orderby=receivedDateTime desc&$top=25&$select={UNREAD_SELECT}"
        );
        let value = self.get(&endpoint, mailbox).await?;
        let messages: Vec<Message> = into_collection(&value)?;
        Ok(Value::Array(
            messages
                .iter()
                .map(|message| mail_item(message, MailShape::Unread))
                .collect(),
        ))
    }

    pub async fn get_conversation_thread(
        &self,
        conversation_id: &str,
        limit: u32,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let endpoint = format!(
            "/messages?$filter=conversationId eq '{}'&$top={limit}&$select={THREAD_SELECT}",
            odata_quote(conversation_id)
        );
        let value = self
            .graph_fetch(
                &endpoint,
                reqwest::Method::GET,
                None,
                &[("ConsistencyLevel", "eventual")],
                mailbox,
            )
            .await?;
        let messages: Vec<Message> = into_collection(&value)?;
        let mut ordered = messages;
        // Graph returns UTC ISO-8601, so lexicographic order is chronological.
        ordered.sort_by(|left, right| left.sent_date_time.cmp(&right.sent_date_time));
        Ok(Value::Array(
            ordered
                .iter()
                .map(|message| mail_item(message, MailShape::Thread))
                .collect(),
        ))
    }

    pub async fn search_emails(
        &self,
        query: &str,
        limit: u32,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let endpoint = format!(
            "/messages?$search=\"{}\"&$top={limit}&$select={SEARCH_SELECT}",
            enc(query)
        );
        let value = self
            .graph_fetch(
                &endpoint,
                reqwest::Method::GET,
                None,
                &[("ConsistencyLevel", "eventual")],
                mailbox,
            )
            .await?;
        let messages: Vec<Message> = into_collection(&value)?;
        Ok(Value::Array(
            messages
                .iter()
                .map(|message| mail_item(message, MailShape::Search))
                .collect(),
        ))
    }

    pub async fn get_sent_emails(
        &self,
        limit: u32,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let endpoint = format!(
            "/mailFolders/sentItems/messages?$orderby=sentDateTime desc&$top={limit}&$select={SENT_SELECT}"
        );
        let value = self.get(&endpoint, mailbox).await?;
        let messages: Vec<Message> = into_collection(&value)?;
        Ok(Value::Array(
            messages
                .iter()
                .map(|message| mail_item(message, MailShape::Sent))
                .collect(),
        ))
    }

    pub async fn get_email_folders(&self, mailbox: Option<&str>) -> Result<Value, GraphError> {
        let value = self.get("/mailFolders?$top=50", mailbox).await?;
        let folders: Vec<MailFolder> = into_collection(&value)?;
        Ok(Value::Array(
            folders
                .iter()
                .map(|folder| {
                    let mut item = Map::new();
                    put_opt_str(&mut item, "id", folder.id.clone());
                    put_opt_str(&mut item, "name", folder.display_name.clone());
                    put_json(
                        &mut item,
                        "unreadCount",
                        folder
                            .unread_item_count
                            .map_or(Value::Null, |count| json!(count)),
                    );
                    put_json(
                        &mut item,
                        "totalCount",
                        folder
                            .total_item_count
                            .map_or(Value::Null, |count| json!(count)),
                    );
                    Value::Object(item)
                })
                .collect(),
        ))
    }

    pub async fn move_email(
        &self,
        email_id: &str,
        destination_folder_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let payload = json!({ "destinationId": destination_folder_id });
        let result = self
            .post(
                &format!("/messages/{email_id}/move"),
                Some(&payload),
                mailbox,
            )
            .await?;
        if result.get("id").is_some() || result.get("success").is_some() {
            return Ok(json!({
                "success": true,
                "emailId": email_id,
                "destinationFolderId": destination_folder_id,
            }));
        }
        Err(GraphError::message(
            "Failed to move email — Graph API returned no confirmation.",
        ))
    }

    pub async fn mark_email_read(
        &self,
        email_id: &str,
        is_read: bool,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let payload = json!({ "isRead": is_read });
        let result = self
            .patch(&format!("/messages/{email_id}"), Some(&payload), mailbox)
            .await?;
        if result.get("id").is_some()
            || result.get("isRead").is_some()
            || result.get("success").is_some()
        {
            return Ok(json!({
                "success": true,
                "emailId": email_id,
                "isRead": is_read,
            }));
        }
        Err(GraphError::message(
            "Failed to mark email read/unread — Graph API returned no confirmation.",
        ))
    }

    pub async fn create_draft(
        &self,
        request: DraftRequest<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let files = self.resolve_attachments(request.attachments).await?;
        let mut payload = json!({
            "subject": request.subject,
            "body": { "contentType": "HTML", "content": request.body },
            "toRecipients": to_recipients(request.to),
            "ccRecipients": to_recipients(request.cc),
            "bccRecipients": to_recipients(request.bcc),
        });
        if !files.is_empty() {
            payload["attachments"] = Value::Array(attach::build_inline(&files).await?);
        }

        let created = self.post("/messages", Some(&payload), mailbox).await?;
        let Some(draft_id) = created.get("id").and_then(Value::as_str) else {
            return Err(GraphError::message(
                "Failed to create draft in Outlook — Graph API did not return a draft ID. The draft was NOT saved.",
            ));
        };
        let attachment_count = payload
            .get("attachments")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);

        let mut result = Map::new();
        put_str(&mut result, "draftId", draft_id);
        put_opt_str(&mut result, "to", request.to.map(str::to_string));
        put_opt_str(&mut result, "cc", request.cc.map(str::to_string));
        put_opt_str(&mut result, "bcc", request.bcc.map(str::to_string));
        put_str(&mut result, "subject", request.subject);
        put_str(&mut result, "body", request.body);
        put_opt_str(
            &mut result,
            "conversationId",
            request.conversation_id.map(str::to_string),
        );
        put_json(&mut result, "attachmentCount", json!(attachment_count));
        put_str(&mut result, "status", "created_in_outlook_drafts");
        Ok(Value::Object(result))
    }

    pub async fn create_reply_draft(
        &self,
        request: ReplyDraftRequest<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let files = self.resolve_attachments(request.attachments).await?;
        let inline_attachments = !files.is_empty();
        let has_recipients = request.cc.is_some_and(|value| !value.is_empty())
            || request.bcc.is_some_and(|value| !value.is_empty());

        let mut payload = Map::new();
        if !request.body.is_empty() {
            put_str(&mut payload, "comment", request.body);
        }
        if has_recipients || inline_attachments {
            let mut message = json!({
                "ccRecipients": to_recipients(request.cc),
                "bccRecipients": to_recipients(request.bcc),
            });
            if inline_attachments {
                message["attachments"] = Value::Array(attach::build_inline(&files).await?);
            }
            put_json(&mut payload, "message", message);
        }
        let attachment_count = if inline_attachments { files.len() } else { 0 };

        let endpoint = if request.reply_all {
            format!("/messages/{}/createReplyAll", request.message_id)
        } else {
            format!("/messages/{}/createReply", request.message_id)
        };
        let created = self
            .post(&endpoint, Some(&Value::Object(payload)), mailbox)
            .await?;
        let Some(draft_id) = created.get("id").and_then(Value::as_str) else {
            return Err(GraphError::message(
                "Failed to create reply draft in Outlook — Graph API did not return a draft ID.",
            ));
        };
        let mut result = Map::new();
        put_str(&mut result, "draftId", draft_id);
        put_str(&mut result, "messageId", request.message_id);
        crate::out::put_null_str(
            &mut result,
            "conversationId",
            created
                .get("conversationId")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        put_or_empty(
            &mut result,
            "subject",
            created
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        put_or_empty(
            &mut result,
            "body",
            created
                .pointer("/body/content")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        put_str(
            &mut result,
            "toRecipients",
            join_value_recipients(created.get("toRecipients")),
        );
        put_str(
            &mut result,
            "ccRecipients",
            join_value_recipients(created.get("ccRecipients")),
        );
        put_str(
            &mut result,
            "bccRecipients",
            join_value_recipients(created.get("bccRecipients")),
        );
        put_json(&mut result, "attachmentCount", json!(attachment_count));
        put_str(&mut result, "status", "created_reply_draft_in_outlook");
        Ok(Value::Object(result))
    }

    pub async fn update_draft(
        &self,
        update: DraftUpdate<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let DraftUpdate {
            draft_id,
            subject,
            body,
            to,
            cc,
            bcc,
        } = update;
        let mut payload = Map::new();
        let mut updated_fields = Vec::new();
        if let Some(subject) = subject {
            put_str(&mut payload, "subject", subject);
            updated_fields.push("subject");
        }
        if let Some(body) = body {
            put_json(
                &mut payload,
                "body",
                json!({ "contentType": "HTML", "content": body }),
            );
            updated_fields.push("body");
        }
        if let Some(to) = to {
            put_json(&mut payload, "toRecipients", json!(to_recipients(Some(to))));
            updated_fields.push("toRecipients");
        }
        if let Some(cc) = cc {
            put_json(&mut payload, "ccRecipients", json!(to_recipients(Some(cc))));
            updated_fields.push("ccRecipients");
        }
        if let Some(bcc) = bcc {
            put_json(
                &mut payload,
                "bccRecipients",
                json!(to_recipients(Some(bcc))),
            );
            updated_fields.push("bccRecipients");
        }

        let updated = self
            .patch(
                &format!("/messages/{draft_id}"),
                Some(&Value::Object(payload)),
                mailbox,
            )
            .await?;
        if updated.get("id").is_none() && updated.get("success").is_none() {
            return Err(GraphError::message(
                "Failed to update draft in Outlook — Graph API returned no confirmation.",
            ));
        }

        let mut result = Map::new();
        put_json(&mut result, "success", json!(true));
        put_str(&mut result, "draftId", draft_id);
        put_json(
            &mut result,
            "updatedFields",
            Value::Array(
                updated_fields
                    .iter()
                    .map(|field| json!(field))
                    .collect::<Vec<_>>(),
            ),
        );
        put_str(
            &mut result,
            "subject",
            updated
                .get("subject")
                .and_then(Value::as_str)
                .or(subject)
                .unwrap_or_default(),
        );
        put_str(
            &mut result,
            "body",
            updated
                .pointer("/body/content")
                .and_then(Value::as_str)
                .or(body)
                .unwrap_or_default(),
        );
        put_str(
            &mut result,
            "to",
            join_value_recipients(updated.get("toRecipients")),
        );
        put_str(
            &mut result,
            "cc",
            join_value_recipients(updated.get("ccRecipients")),
        );
        put_str(
            &mut result,
            "bcc",
            join_value_recipients(updated.get("bccRecipients")),
        );
        Ok(Value::Object(result))
    }

    pub async fn send_draft(
        &self,
        draft_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let result = self
            .post(&format!("/messages/{draft_id}/send"), None, mailbox)
            .await?;
        if result.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({
                "success": true,
                "draftId": draft_id,
                "sentAt": now_iso(),
            }));
        }
        Err(GraphError::message(format!(
            "Failed to send draft {draft_id} — Graph API did not return 202 Accepted."
        )))
    }

    pub async fn reply_to_email(
        &self,
        request: ReplyRequest<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let ReplyRequest {
            email_id,
            body,
            reply_all,
            cc,
            bcc,
            attachments,
        } = request;
        let files = self.resolve_attachments(attachments).await?;

        let inline_attachments = !files.is_empty();
        let has_recipients =
            cc.is_some_and(|value| !value.is_empty()) || bcc.is_some_and(|value| !value.is_empty());

        let mut payload = Map::new();
        put_str(&mut payload, "comment", body);
        if has_recipients || inline_attachments {
            let mut message = json!({
                "ccRecipients": to_recipients(cc),
                "bccRecipients": to_recipients(bcc),
            });
            if inline_attachments {
                message["attachments"] = Value::Array(attach::build_inline(&files).await?);
            }
            put_json(&mut payload, "message", message);
        }
        let attachment_count = if inline_attachments { files.len() } else { 0 };

        let endpoint = if reply_all {
            format!("/messages/{email_id}/replyAll")
        } else {
            format!("/messages/{email_id}/reply")
        };
        let result = self
            .post(&endpoint, Some(&Value::Object(payload)), mailbox)
            .await?;
        if result.get("success").and_then(Value::as_bool) == Some(true) {
            let mut value = Map::new();
            put_json(&mut value, "success", json!(true));
            put_str(&mut value, "emailId", email_id);
            put_json(&mut value, "replyAll", json!(reply_all));
            put_opt_str(&mut value, "cc", cc.map(str::to_string));
            put_opt_str(&mut value, "bcc", bcc.map(str::to_string));
            put_json(&mut value, "attachmentCount", json!(attachment_count));
            put_str(&mut value, "sentAt", now_iso());
            return Ok(Value::Object(value));
        }
        Err(GraphError::message(
            "Failed to send reply — Graph API did not return 202 Accepted.",
        ))
    }

    pub async fn send_email(
        &self,
        request: SendRequest<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let SendRequest {
            to,
            cc,
            bcc,
            subject,
            body,
            attachments,
        } = request;
        let files = self.resolve_attachments(attachments).await?;

        let mut message = json!({
            "subject": subject,
            "body": { "contentType": "HTML", "content": body },
            "toRecipients": to_recipients(Some(to)),
            "ccRecipients": to_recipients(cc),
            "bccRecipients": to_recipients(bcc),
        });
        if !files.is_empty() {
            message["attachments"] = Value::Array(attach::build_inline(&files).await?);
        }
        let attachment_count = message
            .get("attachments")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let payload = json!({ "message": message });

        let result = self.post("/sendMail", Some(&payload), mailbox).await?;
        if result.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({
                "success": true,
                "attachmentCount": attachment_count,
                "sentAt": now_iso(),
            }));
        }
        Err(GraphError::message(
            "Failed to send email — Graph API did not return 202 Accepted.",
        ))
    }

    pub async fn get_attachments_for_message(
        &self,
        message_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Vec<Attachment>, GraphError> {
        if message_id.is_empty() {
            return Err(GraphError::message("Missing messageId argument"));
        }
        let value = self
            .get(&format!("/messages/{message_id}/attachments"), mailbox)
            .await?;
        into_collection(&value)
    }

    pub async fn get_attachment_content(
        &self,
        message_id: &str,
        attachment_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        if message_id.is_empty() || attachment_id.is_empty() {
            return Err(GraphError::message(
                "Missing required arguments: messageId, attachmentId",
            ));
        }
        self.get(
            &format!("/messages/{message_id}/attachments/{attachment_id}"),
            mailbox,
        )
        .await
    }

    /// Port of `getAttachments.js`: metadata pass, then a content fetch for any
    /// file attachment whose metadata omitted `contentBytes`.
    pub async fn get_email_attachments(
        &self,
        message_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        if message_id.is_empty() {
            return Err(GraphError::message(
                "Missing required argument: id (Outlook message id)",
            ));
        }
        let metadata = self
            .get_attachments_for_message(message_id, mailbox)
            .await?;

        let mut attachments = Vec::with_capacity(metadata.len());
        for meta in &metadata {
            let odata_type = meta.odata_type.clone().unwrap_or_default();
            let is_file_attachment = odata_type.is_empty()
                || odata_type.contains("fileAttachment")
                || meta.content_bytes.is_some();

            let mut content = meta.content_bytes.clone();
            if is_file_attachment
                && content.is_none()
                && let Some(attachment_id) = meta.id.as_deref()
            {
                match self
                    .get_attachment_content(message_id, attachment_id, mailbox)
                    .await
                {
                    Ok(full) => {
                        content = full
                            .get("contentBytes")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    Err(err) => {
                        tracing::warn!(
                            attachment_id,
                            error = %err,
                            "could not fetch attachment content"
                        );
                    }
                }
            }

            let mut item = Map::new();
            put_str(&mut item, "id", meta.id.clone().unwrap_or_default());
            if is_file_attachment {
                put_str(
                    &mut item,
                    "name",
                    meta.name
                        .clone()
                        .unwrap_or_else(|| "unnamed_attachment".into()),
                );
                put_str(
                    &mut item,
                    "contentType",
                    meta.content_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into()),
                );
            } else {
                put_str(
                    &mut item,
                    "name",
                    meta.name
                        .clone()
                        .unwrap_or_else(|| "item_attachment".into()),
                );
                put_str(
                    &mut item,
                    "contentType",
                    if odata_type.is_empty() {
                        "itemAttachment".to_string()
                    } else {
                        odata_type.clone()
                    },
                );
            }
            put_json(
                &mut item,
                "size",
                meta.size.map_or(Value::Null, |size| json!(size)),
            );
            put_json(
                &mut item,
                "isInline",
                json!(meta.is_inline.unwrap_or(false)),
            );
            crate::out::put_null_str(&mut item, "contentBytes", content);
            attachments.push(Value::Object(item));
        }

        Ok(json!({
            "id": message_id,
            "attachments": attachments,
            "count": attachments.len(),
        }))
    }
}

fn join_value_recipients(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|recipient| recipient.pointer("/emailAddress/address"))
                .filter_map(Value::as_str)
                .filter(|address| !address.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_json() -> Value {
        json!({
            "id": "AAA",
            "conversationId": "CC1",
            "sender": { "emailAddress": { "address": "boss@example.com", "name": "Boss" } },
            "subject": "",
            "body": { "contentType": "html", "content": "x".repeat(400) },
            "receivedDateTime": "2026-11-13T01:02:03Z",
            "isRead": false,
            "toRecipients": [{ "emailAddress": { "address": "me@example.com" } }],
            "ccRecipients": [],
            "bccRecipients": [{ "emailAddress": { "name": "No Address" } }],
            "hasAttachments": true,
            "importance": "high"
        })
    }

    fn parse(value: Value) -> Message {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn recent_shape_uses_no_subject_fallback() {
        let item = mail_item(&parse(message_json()), MailShape::Recent);
        assert_eq!(item["subject"], "No Subject");
        assert_eq!(item["sender"], "boss@example.com");
        assert_eq!(item["senderName"], "Boss");
        assert_eq!(item["to"], "me@example.com");
        assert_eq!(item["cc"], "");
        assert_eq!(item["bcc"], "");
        assert_eq!(item["isRead"], false);
        assert_eq!(item["hasAttachments"], true);
        assert_eq!(item["importance"], "high");
    }

    #[test]
    fn recent_shape_omits_undefined_but_keeps_empty_strings() {
        let mut value = message_json();
        value["receivedDateTime"] = Value::Null;
        value.as_object_mut().unwrap().remove("importance");
        let item = mail_item(&parse(value), MailShape::Recent);
        assert!(item.get("receivedDateTime").is_none());
        assert!(item.get("importance").is_none());
        assert_eq!(item["cc"], "");
    }

    #[test]
    fn unread_shape_drops_is_read() {
        let item = mail_item(&parse(message_json()), MailShape::Unread);
        assert!(item.get("isRead").is_none());
        assert!(item.get("body").is_some());
    }

    #[test]
    fn all_folders_shape_uses_body_preview() {
        let item = mail_item(&parse(message_json()), MailShape::AllFolders);
        assert!(item.get("body").is_none());
        assert_eq!(item["bodyPreview"], "");
    }

    #[test]
    fn search_preview_truncates_to_300_chars() {
        let item = mail_item(&parse(message_json()), MailShape::Search);
        assert_eq!(item["bodyPreview"].as_str().unwrap().chars().count(), 300);
    }

    #[test]
    fn thread_shape_uses_empty_fallbacks() {
        let item = mail_item(&parse(message_json()), MailShape::Thread);
        assert_eq!(item["subject"], "");
        assert!(item.get("conversationId").is_none());
        assert!(item.get("receivedDateTime").is_none());
    }

    #[test]
    fn sent_shape_drops_sender() {
        let item = mail_item(&parse(message_json()), MailShape::Sent);
        assert!(item.get("sender").is_none());
        assert!(item.get("isRead").is_none());
        assert!(item["sentDateTime"].is_null());
    }
}
