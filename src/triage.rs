//! Keyword triage — a straight port of `emailTool.js`. No model call, no network.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use crate::out::{put_json, put_opt_str, put_str};

macro_rules! rule {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect("valid regex"));
    };
}

rule!(
    MEETING_REQUEST,
    r"(?i)meeting|schedule|call|discuss|zoom|teams|catch up|availability"
);
rule!(
    URGENT,
    r"(?i)urgent|asap|immediately|critical|emergency|deadline|today|by end of"
);
rule!(
    DOC_REVIEW,
    r"(?i)doc|guide|api|review|documentation|feedback|draft|contract|agreement"
);
rule!(
    ADDRESS_REQUEST,
    r"(?i)address|shipping|order|delivery|postal|zip"
);
rule!(
    REQUIRES_REPLY,
    r"(?i)please|kindly|reply|let me know|confirm|update|review|could you|would you"
);
rule!(
    INTRODUCTION,
    r"(?i)introduce|introduction|pleasure|reach out|connect|opportunity"
);
rule!(
    FYI,
    r"(?i)fyi|for your information|no action|just to let you know"
);

pub struct TriageInput<'a> {
    pub id: &'a str,
    pub sender: &'a str,
    pub sender_name: Option<&'a str>,
    pub subject: &'a str,
    pub body: &'a str,
}

pub fn triage(input: TriageInput<'_>, signoff: &str) -> Value {
    let text = format!("{} {}", input.subject, input.body);

    let is_meeting_request = MEETING_REQUEST.is_match(&text);
    let is_urgent = URGENT.is_match(&text);
    let is_doc_review = DOC_REVIEW.is_match(&text);
    let is_address_request = ADDRESS_REQUEST.is_match(&text);
    let requires_reply = REQUIRES_REPLY.is_match(&text);
    let is_introduction = INTRODUCTION.is_match(&text);
    let is_fyi = FYI.is_match(&text);

    let mut category = "General";
    let mut suggested_action = "Review & Draft Response";
    let mut urgency = "Normal";
    if is_urgent {
        urgency = "High";
    }
    // Branch order is load-bearing — FYI is last and also downgrades urgency.
    if is_meeting_request {
        category = "Meeting Request";
        suggested_action = "Check Calendar & Propose Time";
    } else if is_doc_review {
        category = "Document Review";
        suggested_action = "Review & Provide Feedback";
    } else if is_address_request {
        category = "Information Request";
        suggested_action = "Provide Requested Information";
    } else if is_introduction {
        category = "Introduction";
        suggested_action = "Draft Acknowledgement";
    } else if is_fyi {
        category = "FYI";
        suggested_action = "No Reply Needed";
        urgency = "Low";
    }

    let name = input.sender_name.unwrap_or("there");
    let suggestion = if is_meeting_request {
        format!(
            "Hi {name},\n\nThank you for reaching out. I'm checking availability and will come back to you shortly with a proposed time.\n\nBest regards,\n{signoff}"
        )
    } else if is_doc_review {
        format!(
            "Hi {name},\n\nThank you for sharing this. I'll review the material and get back to you.\n\nBest regards,\n{signoff}"
        )
    } else if is_urgent {
        format!(
            "Hi {name},\n\nThank you — I've received your message and will attend to this shortly.\n\nBest regards,\n{signoff}"
        )
    } else if !is_fyi {
        format!(
            "Hi {name},\n\nThank you for your message. I'll review and get back to you.\n\nBest regards,\n{signoff}"
        )
    } else {
        String::new()
    };

    let mut result = Map::new();
    put_str(&mut result, "id", input.id);
    put_str(&mut result, "subject", input.subject);
    put_str(&mut result, "sender", input.sender);
    put_opt_str(
        &mut result,
        "senderName",
        input.sender_name.map(str::to_string),
    );
    put_str(&mut result, "category", category);
    put_str(&mut result, "urgency", urgency);
    put_json(
        &mut result,
        "isMeetingRequest",
        Value::Bool(is_meeting_request),
    );
    put_json(&mut result, "isUrgent", Value::Bool(is_urgent));
    put_json(&mut result, "requiresReply", Value::Bool(requires_reply));
    put_str(&mut result, "suggestedAction", suggested_action);
    put_str(&mut result, "suggestedReply", suggestion);
    Value::Object(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(subject: &'a str, body: &'a str) -> TriageInput<'a> {
        TriageInput {
            id: "ID1",
            sender: "boss@example.com",
            sender_name: Some("Boss"),
            subject,
            body,
        }
    }

    #[test]
    fn meeting_request_wins_over_doc_review() {
        let value = triage(input("Meeting request", "can we schedule a call"), "Sam");
        assert_eq!(value["category"], "Meeting Request");
        assert_eq!(value["suggestedAction"], "Check Calendar & Propose Time");
        assert_eq!(value["isMeetingRequest"], true);
        assert!(
            value["suggestedReply"]
                .as_str()
                .unwrap()
                .contains("Hi Boss,")
        );
    }

    #[test]
    fn fyi_is_last_branch_and_lowers_urgency() {
        let value = triage(input("FYI", "no action needed"), "Sam");
        assert_eq!(value["category"], "FYI");
        assert_eq!(value["urgency"], "Low");
        assert_eq!(value["suggestedReply"], "");
    }

    #[test]
    fn urgent_branch_wins_over_the_fyi_reply_suppression() {
        // Faithful to the JS chain: `else if (isUrgent)` sits before `else if (!isFYI)`,
        // so an urgent FYI still gets the urgent template even though category is FYI.
        let value = triage(input("FYI", "urgent, no action needed"), "Sam");
        assert_eq!(value["category"], "FYI");
        assert_eq!(value["urgency"], "Low");
        assert!(
            value["suggestedReply"]
                .as_str()
                .unwrap()
                .contains("I've received your message")
        );
    }

    #[test]
    fn urgent_flag_sets_urgency_without_changing_generic_category() {
        let value = triage(input("Please confirm", "asap please"), "Sam");
        assert_eq!(value["category"], "General");
        assert_eq!(value["urgency"], "High");
        assert_eq!(value["requiresReply"], true);
    }

    #[test]
    fn missing_sender_name_falls_back_to_there() {
        let value = triage(
            TriageInput {
                id: "ID2",
                sender: "x@example.com",
                sender_name: None,
                subject: "Documentation review",
                body: "",
            },
            "Sam",
        );
        // put_opt_str omits senderName, matching `senderName: undefined`.
        assert!(value.get("senderName").is_none());
        assert!(
            value["suggestedReply"]
                .as_str()
                .unwrap()
                .contains("Hi there,")
        );
    }
}
