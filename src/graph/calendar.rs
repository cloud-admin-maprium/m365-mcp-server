//! Calendar operations and the combined daily briefing.

use chrono::{Duration, Local, SecondsFormat, TimeZone, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::out::{now_iso, put_json, put_opt_bool, put_opt_str, put_str};

use super::{GraphClient, GraphError, Recipient, into_collection};

const EVENT_SELECT: &str = "id,subject,start,end,attendees,location,organizer,bodyPreview,isOnlineMeeting,onlineMeetingUrl,sensitivity,importance,isCancelled,isAllDay,showAs";

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DateTimeZone {
    pub date_time: Option<String>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Location {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ResponseStatus {
    pub response: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Attendee {
    pub email_address: Option<super::EmailAddress>,
    pub status: Option<ResponseStatus>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Event {
    pub id: Option<String>,
    pub subject: Option<String>,
    pub start: Option<DateTimeZone>,
    pub end: Option<DateTimeZone>,
    pub attendees: Option<Vec<Attendee>>,
    pub location: Option<Location>,
    pub organizer: Option<Recipient>,
    pub body_preview: Option<String>,
    pub is_online_meeting: Option<bool>,
    pub online_meeting_url: Option<String>,
    pub sensitivity: Option<String>,
    pub importance: Option<String>,
    pub is_cancelled: Option<bool>,
    pub is_all_day: Option<bool>,
    pub show_as: Option<String>,
}

impl Event {
    fn start_value(&self) -> Option<String> {
        self.start
            .as_ref()
            .and_then(|start| start.date_time.clone())
    }

    fn end_value(&self) -> Option<String> {
        self.end.as_ref().and_then(|end| end.date_time.clone())
    }

    fn location_value(&self) -> Option<String> {
        self.location
            .as_ref()
            .and_then(|location| location.display_name.clone())
            .filter(|name| !name.is_empty())
    }

    fn organizer_address(&self) -> Option<String> {
        self.organizer
            .as_ref()
            .and_then(|organizer| organizer.email_address.as_ref())
            .and_then(|address| address.address.clone())
            .filter(|address| !address.is_empty())
    }

    fn is_private(&self) -> bool {
        self.sensitivity.as_deref() == Some("private")
    }

    fn to_value(&self) -> Value {
        let mut item = Map::new();
        put_opt_str(&mut item, "id", self.id.clone());
        put_opt_str(&mut item, "subject", self.subject.clone());
        put_opt_str(&mut item, "start", self.start_value());
        put_opt_str(&mut item, "end", self.end_value());
        put_str(
            &mut item,
            "location",
            self.location_value().unwrap_or_default(),
        );
        put_str(
            &mut item,
            "organizer",
            self.organizer_address().unwrap_or_default(),
        );
        put_json(
            &mut item,
            "attendees",
            Value::Array(
                self.attendees
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|attendee| {
                        json!({
                            "address": attendee
                                .email_address
                                .as_ref()
                                .and_then(|address| address.address.clone()),
                            "name": attendee
                                .email_address
                                .as_ref()
                                .and_then(|address| address.name.clone())
                                .unwrap_or_default(),
                            "response": attendee
                                .status
                                .as_ref()
                                .and_then(|status| status.response.clone())
                                .unwrap_or_else(|| "none".to_string()),
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        put_str(
            &mut item,
            "bodyPreview",
            self.body_preview.clone().unwrap_or_default(),
        );
        put_opt_bool(&mut item, "isOnlineMeeting", self.is_online_meeting);
        put_str(
            &mut item,
            "onlineMeetingUrl",
            self.online_meeting_url.clone().unwrap_or_default(),
        );
        put_json(&mut item, "isPrivate", json!(self.is_private()));
        put_opt_str(&mut item, "importance", self.importance.clone());
        put_opt_bool(&mut item, "isCancelled", self.is_cancelled);
        put_opt_bool(&mut item, "isAllDay", self.is_all_day);
        put_opt_str(&mut item, "showAs", self.show_as.clone());
        Value::Object(item)
    }

    fn conflict_summary(&self) -> Value {
        json!({
            "id": self.id.clone(),
            "subject": self.subject.clone(),
            "start": self.start_value(),
            "end": self.end_value(),
        })
    }
}

pub struct EventRequest<'a> {
    pub subject: &'a str,
    pub start_time: &'a str,
    pub end_time: &'a str,
    pub attendees: &'a [String],
    pub location: &'a str,
    pub body: &'a str,
    pub is_private: bool,
    pub force: bool,
}

pub struct EventUpdates<'a> {
    pub subject: Option<&'a str>,
    pub start_time: Option<&'a str>,
    pub end_time: Option<&'a str>,
    pub location: Option<&'a str>,
    pub body: Option<&'a str>,
    pub attendees: Option<&'a [String]>,
    pub is_private: Option<bool>,
    pub force: Option<bool>,
}

impl GraphClient {
    async fn fetch_events(
        &self,
        start: &str,
        end: &str,
        mailbox: Option<&str>,
    ) -> Result<Vec<Event>, GraphError> {
        let endpoint = format!(
            "/calendarView?startDateTime={}&endDateTime={}&$select={EVENT_SELECT}&$orderby=start/dateTime",
            super::enc(start),
            super::enc(end)
        );
        let value = self.get(&endpoint, mailbox).await?;
        into_collection(&value)
    }

    pub async fn get_calendar_events(
        &self,
        start: &str,
        end: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let events = self.fetch_events(start, end, mailbox).await?;
        Ok(Value::Array(events.iter().map(Event::to_value).collect()))
    }

    pub async fn get_upcoming_events(
        &self,
        days: i64,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let start = now_iso();
        let end = (Utc::now() + Duration::days(days)).to_rfc3339_opts(SecondsFormat::Millis, true);
        self.get_calendar_events(&start, &end, mailbox).await
    }

    pub async fn create_event(
        &self,
        request: EventRequest<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        if !request.force {
            let existing = self
                .fetch_events(request.start_time, request.end_time, mailbox)
                .await?;
            if !existing.is_empty() {
                return Ok(json!({
                    "success": false,
                    "conflict": true,
                    "conflictingEvents": existing.iter().map(Event::conflict_summary).collect::<Vec<_>>(),
                    "error": format!(
                        "Conflict: {} existing event(s) overlap this time range. Present the conflict to the user before proceeding — pass force:true only after explicit approval to double-book.",
                        existing.len()
                    ),
                }));
            }
        }

        let mut payload = json!({
            "subject": request.subject,
            "body": { "contentType": "HTML", "content": request.body },
            "start": { "dateTime": request.start_time, "timeZone": self.config().calendar_timezone },
            "end": { "dateTime": request.end_time, "timeZone": self.config().calendar_timezone },
            "attendees": request.attendees.iter().map(|address| json!({
                "emailAddress": { "address": address },
                "type": "required",
            })).collect::<Vec<_>>(),
        });
        if !request.location.is_empty() {
            payload["location"] = json!({ "displayName": request.location });
        }
        if request.is_private {
            payload["sensitivity"] = json!("private");
        }

        let created = self.post("/events", Some(&payload), mailbox).await?;
        let Some(event_id) = created.get("id").and_then(Value::as_str) else {
            return Err(GraphError::message(
                "Failed to create calendar event — Graph API did not return an event ID. The event was NOT created.",
            ));
        };

        let mut result = Map::new();
        put_str(&mut result, "eventId", event_id);
        put_str(&mut result, "subject", request.subject);
        put_str(&mut result, "startTime", request.start_time);
        put_str(&mut result, "endTime", request.end_time);
        put_json(&mut result, "attendees", json!(request.attendees));
        put_str(&mut result, "location", request.location);
        put_str(
            &mut result,
            "sensitivity",
            if request.is_private {
                "private"
            } else {
                "normal"
            },
        );
        put_str(&mut result, "status", "scheduled");
        Ok(Value::Object(result))
    }

    pub async fn update_event(
        &self,
        event_id: &str,
        updates: EventUpdates<'_>,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let force = updates.force.unwrap_or(false);
        if let (Some(start_time), Some(end_time)) = (updates.start_time, updates.end_time)
            && !force
        {
            let existing = self.fetch_events(start_time, end_time, mailbox).await?;
            let conflicts: Vec<&Event> = existing
                .iter()
                .filter(|event| event.id.as_deref() != Some(event_id))
                .collect();
            if !conflicts.is_empty() {
                return Ok(json!({
                    "success": false,
                    "conflict": true,
                    "conflictingEvents": conflicts.iter().map(|event| event.conflict_summary()).collect::<Vec<_>>(),
                    "error": format!(
                        "Conflict: {} existing event(s) overlap the new time range. Present the conflict to the user before proceeding — pass force:true only after explicit approval to double-book.",
                        conflicts.len()
                    ),
                }));
            }
        }

        let mut payload = Map::new();
        if let Some(subject) = updates.subject {
            put_str(&mut payload, "subject", subject);
        }
        if let Some(start_time) = updates.start_time {
            put_json(
                &mut payload,
                "start",
                json!({ "dateTime": start_time, "timeZone": self.config().calendar_timezone }),
            );
        }
        if let Some(end_time) = updates.end_time {
            put_json(
                &mut payload,
                "end",
                json!({ "dateTime": end_time, "timeZone": self.config().calendar_timezone }),
            );
        }
        if let Some(location) = updates.location {
            put_json(&mut payload, "location", json!({ "displayName": location }));
        }
        if let Some(body) = updates.body {
            put_json(
                &mut payload,
                "body",
                json!({ "contentType": "HTML", "content": body }),
            );
        }
        if let Some(is_private) = updates.is_private {
            put_json(
                &mut payload,
                "sensitivity",
                json!(if is_private { "private" } else { "normal" }),
            );
        }
        if let Some(attendees) = updates.attendees {
            put_json(
                &mut payload,
                "attendees",
                json!(
                    attendees
                        .iter()
                        .map(|address| json!({
                            "emailAddress": { "address": address },
                            "type": "required",
                        }))
                        .collect::<Vec<_>>()
                ),
            );
        }

        let updated = self
            .patch(
                &format!("/events/{event_id}"),
                Some(&Value::Object(payload)),
                mailbox,
            )
            .await?;
        if updated.get("id").is_none() && updated.get("success").is_none() {
            return Err(GraphError::message(
                "Failed to update calendar event — Graph API returned no confirmation.",
            ));
        }

        // Echo the caller's updates back, `force` included when it was supplied.
        let mut echo = Map::new();
        put_opt_str(&mut echo, "subject", updates.subject.map(str::to_string));
        put_opt_str(
            &mut echo,
            "startTime",
            updates.start_time.map(str::to_string),
        );
        put_opt_str(&mut echo, "endTime", updates.end_time.map(str::to_string));
        put_opt_str(&mut echo, "location", updates.location.map(str::to_string));
        put_opt_str(&mut echo, "body", updates.body.map(str::to_string));
        if let Some(attendees) = updates.attendees {
            put_json(&mut echo, "attendees", json!(attendees));
        }
        if let Some(is_private) = updates.is_private {
            put_json(&mut echo, "isPrivate", json!(is_private));
        }
        if let Some(force) = updates.force {
            put_json(&mut echo, "force", json!(force));
        }

        let mut result = Map::new();
        put_json(&mut result, "success", json!(true));
        put_str(&mut result, "eventId", event_id);
        put_json(&mut result, "updates", Value::Object(echo));
        Ok(Value::Object(result))
    }

    pub async fn cancel_event(
        &self,
        event_id: &str,
        comment: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let payload = json!({ "comment": comment });
        let result = self
            .post(
                &format!("/events/{event_id}/cancel"),
                Some(&payload),
                mailbox,
            )
            .await?;
        if result.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({
                "success": true,
                "eventId": event_id,
                "cancelledAt": now_iso(),
            }));
        }
        Err(GraphError::message(
            "Failed to cancel calendar event — Graph API did not return 202 Accepted.",
        ))
    }

    pub async fn delete_event(
        &self,
        event_id: &str,
        mailbox: Option<&str>,
    ) -> Result<Value, GraphError> {
        let result = self.delete(&format!("/events/{event_id}"), mailbox).await?;
        if result.get("success").and_then(Value::as_bool) == Some(true) {
            return Ok(json!({
                "success": true,
                "eventId": event_id,
                "deletedAt": now_iso(),
            }));
        }
        Err(GraphError::message(
            "Failed to delete calendar event — Graph API did not return 204 No Content.",
        ))
    }

    /// `new Date()` local-midnight boundaries, exactly like the JS original
    /// (`graphClient.js:489-491`) — the mailbox timezone is not consulted.
    pub async fn get_todays_briefing(&self, mailbox: Option<&str>) -> Result<Value, GraphError> {
        let now = Local::now();
        let day = now.date_naive();
        let start = day
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .unwrap_or(now);
        let end = day
            .and_hms_opt(23, 59, 59)
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .map(|datetime| datetime + Duration::milliseconds(999))
            .unwrap_or(now);
        let start_iso = start.to_rfc3339_opts(SecondsFormat::Millis, true);
        let end_iso = end.to_rfc3339_opts(SecondsFormat::Millis, true);

        let (emails, events) = tokio::join!(
            self.get_unread_emails(mailbox),
            self.get_calendar_events(&start_iso, &end_iso, mailbox),
        );

        let mut result = Map::new();
        put_str(&mut result, "date", now_iso());
        match emails {
            Ok(value) => put_json(&mut result, "unreadEmails", json_array(value)),
            Err(err) => {
                put_json(&mut result, "unreadEmails", json!([]));
                put_str(&mut result, "emailError", err.to_string());
            }
        }
        match events {
            Ok(value) => put_json(&mut result, "todaysEvents", json_array(value)),
            Err(err) => {
                put_json(&mut result, "todaysEvents", json!([]));
                put_str(&mut result, "eventError", err.to_string());
            }
        }
        Ok(Value::Object(result))
    }
}

fn json_array(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items),
        _ => json!([]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_shape_matches_the_js_mapper() {
        let event: Event = serde_json::from_value(json!({
            "id": "E1",
            "subject": "Standup",
            "start": { "dateTime": "2026-11-13T09:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-11-13T09:15:00.0000000", "timeZone": "UTC" },
            "location": { "displayName": "Room 1" },
            "organizer": { "emailAddress": { "address": "boss@example.com" } },
            "attendees": [
                { "emailAddress": { "address": "me@example.com", "name": "Me" },
                  "status": { "response": "accepted" } },
                { "emailAddress": { "address": "other@example.com" } }
            ],
            "bodyPreview": "hi",
            "isOnlineMeeting": true,
            "onlineMeetingUrl": "https://teams.example/x",
            "sensitivity": "private",
            "importance": "normal",
            "isCancelled": false,
            "isAllDay": false,
            "showAs": "busy"
        }))
        .unwrap();

        let value = event.to_value();
        assert_eq!(value["start"], "2026-11-13T09:00:00.0000000");
        assert_eq!(value["location"], "Room 1");
        assert_eq!(value["organizer"], "boss@example.com");
        assert_eq!(value["isPrivate"], true);
        assert_eq!(value["attendees"][0]["response"], "accepted");
        assert_eq!(value["attendees"][0]["name"], "Me");
        assert_eq!(value["attendees"][1]["response"], "none");
        assert_eq!(value["attendees"][1]["name"], "");
        assert_eq!(value["importance"], "normal");
    }

    #[test]
    fn missing_optional_event_fields_are_omitted() {
        let event: Event = serde_json::from_value(json!({ "id": "E2" })).unwrap();
        let value = event.to_value();
        assert!(value.get("subject").is_none());
        assert!(value.get("showAs").is_none());
        assert!(value.get("isAllDay").is_none());
        // These always render in the JS mapper.
        assert_eq!(value["location"], "");
        assert_eq!(value["organizer"], "");
        assert_eq!(value["bodyPreview"], "");
        assert_eq!(value["onlineMeetingUrl"], "");
        assert_eq!(value["isPrivate"], false);
        assert_eq!(value["attendees"], json!([]));
    }
}
