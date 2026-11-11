//! Planner, calendar, inbox and the activity stream.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Assignment, File, serde_helpers::false_as_none};

/// `GET /planner/items`: one entry in the student planner.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlannerItem {
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub context_name: Option<String>,
    #[serde(default)]
    pub plannable_id: Option<u64>,
    /// `assignment`, `quiz`, `discussion_topic`, `wiki_page`, `planner_note`,
    /// `calendar_event`, `assessment_request` (a peer review), …
    #[serde(default)]
    pub plannable_type: Option<String>,
    #[serde(default)]
    pub plannable_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub plannable: Option<Plannable>,
    /// Canvas sends `false` when the item takes no submission.
    #[serde(default, deserialize_with = "false_as_none")]
    pub submissions: Option<PlannerSubmissions>,
    #[serde(default)]
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Plannable {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub todo_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub points_possible: Option<f64>,
    /// Peer reviews: the assignment the review belongs to.
    #[serde(default)]
    pub assignment_id: Option<u64>,
    /// Peer reviews: `assigned` or `completed`.
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub assessor_id: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PlannerSubmissions {
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub excused: bool,
    #[serde(default)]
    pub graded: bool,
    #[serde(default)]
    pub late: bool,
    #[serde(default)]
    pub missing: bool,
    #[serde(default)]
    pub has_feedback: bool,
}

/// `GET /calendar_events` (type `event` or `assignment`). Assignment events have string IDs
/// like `assignment_123`, so the ID stays raw JSON.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarEvent {
    pub id: serde_json::Value,
    #[serde(default)]
    pub title: Option<String>,
    /// HTML.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub all_day: Option<bool>,
    #[serde(default)]
    pub location_name: Option<String>,
    #[serde(default)]
    pub location_address: Option<String>,
    /// e.g. `course_123`, `user_45`, `group_6`.
    #[serde(default)]
    pub context_code: Option<String>,
    #[serde(default)]
    pub context_name: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    /// Assignment events only.
    #[serde(default)]
    pub assignment: Option<Assignment>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

impl CalendarEvent {
    /// The ID as text: `123` or `assignment_123`.
    pub fn id_text(&self) -> String {
        match &self.id {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        }
    }
}

/// `GET /planner_notes`: a personal to-do item.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlannerNote {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub todo_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub linked_object_type: Option<String>,
    #[serde(default)]
    pub linked_object_id: Option<u64>,
    #[serde(default)]
    pub linked_object_html_url: Option<String>,
}

/// An inbox conversation (`/conversations`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Conversation {
    pub id: u64,
    #[serde(default)]
    pub subject: Option<String>,
    /// `read`, `unread` or `archived`.
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub last_message: Option<String>,
    #[serde(default)]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub message_count: Option<u64>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub context_name: Option<String>,
    #[serde(default)]
    pub participants: Vec<Participant>,
    /// Only on `GET /conversations/:id`.
    #[serde(default)]
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Participant {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversationMessage {
    pub id: u64,
    #[serde(default)]
    pub author_id: Option<u64>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub attachments: Vec<File>,
    #[serde(default)]
    pub generated: Option<bool>,
}

/// `GET /search/recipients`: a person (`type` user) or a group of people (`context`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Recipient {
    /// Numeric for people; strings like `course_123_students` for contexts.
    pub id: serde_json::Value,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Course ID → roles the person holds there, e.g. `{"123": ["StudentEnrollment"]}`.
    #[serde(default)]
    pub common_courses: std::collections::HashMap<String, Vec<String>>,
}

/// `GET /users/self/activity_stream`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActivityItem {
    /// `Announcement`, `DiscussionTopic`, `DiscussionEntry`, `Conversation`, `Submission`,
    /// `Message`, `AssessmentRequest`, …
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub read_state: Option<bool>,
    #[serde(default)]
    pub notification_category: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub grade: Option<String>,
}

/// `GET /users/self/activity_stream/summary`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActivitySummary {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub unread_count: u64,
}
