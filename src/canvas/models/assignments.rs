//! Assignments, submissions, grading.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Author, File, serde_helpers::present};

/// `GET /courses/:id/assignments`. With a student token, dates already reflect the
/// student's own overrides.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Assignment {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unlock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub lock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub points_possible: Option<f64>,
    #[serde(default)]
    pub grading_type: Option<String>,
    #[serde(default)]
    pub submission_types: Vec<String>,
    /// -1 or absent means unlimited.
    #[serde(default)]
    pub allowed_attempts: Option<i64>,
    #[serde(default)]
    pub allowed_extensions: Vec<String>,
    #[serde(default)]
    pub published: Option<bool>,
    #[serde(default)]
    pub omit_from_final_grade: Option<bool>,
    #[serde(default)]
    pub assignment_group_id: Option<u64>,
    #[serde(default)]
    pub peer_reviews: Option<bool>,
    #[serde(default)]
    pub is_quiz_assignment: Option<bool>,
    #[serde(default)]
    pub quiz_id: Option<u64>,
    /// A New Quiz (an LTI assignment).
    #[serde(default)]
    pub is_quiz_lti_assignment: Option<bool>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
    #[serde(default)]
    pub lock_explanation: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    /// Present with `include[]=submission`: the caller's own submission.
    #[serde(default)]
    pub submission: Option<Submission>,
}

impl Assignment {
    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or("Untitled assignment")
    }

    /// Completed in an external tool (e.g. Gradescope), so Canvas may not see the submission.
    pub fn is_external_tool(&self) -> bool {
        self.submission_types.iter().any(|t| t == "external_tool")
    }

    pub fn needs_online_submission(&self) -> bool {
        !self.submission_types.is_empty()
            && !self
                .submission_types
                .iter()
                .all(|t| t == "none" || t == "on_paper" || t == "not_graded")
    }
}

/// The caller's submission for one assignment.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Submission {
    #[serde(default)]
    pub assignment_id: Option<u64>,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub grade: Option<String>,
    /// Attempts used so far.
    #[serde(default)]
    pub attempt: Option<u32>,
    /// `unsubmitted`, `submitted`, `graded` or `pending_review`.
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub submission_type: Option<String>,
    #[serde(default)]
    pub late: Option<bool>,
    #[serde(default)]
    pub missing: Option<bool>,
    #[serde(default)]
    pub excused: Option<bool>,
    /// `None`: Canvas didn't send the key. `Some(None)`: sent as null, meaning the
    /// instructor hasn't posted (released) the grade.
    #[serde(default, deserialize_with = "present")]
    pub posted_at: Option<Option<DateTime<Utc>>>,
    #[serde(default)]
    pub points_deducted: Option<f64>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub attachments: Vec<File>,
    /// Present with `include[]=submission_comments`.
    #[serde(default)]
    pub submission_comments: Vec<SubmissionComment>,
    /// Present with `include[]=assignment`.
    #[serde(default)]
    pub assignment: Option<Box<Assignment>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubmissionComment {
    #[serde(default)]
    pub author_name: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

/// `GET /users/self/todo`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TodoItem {
    /// `submitting` (your own work) or `grading` (courses you teach or TA).
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub assignment: Option<Assignment>,
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub context_name: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    /// `grading` items: submissions waiting for you to grade.
    #[serde(default)]
    pub needs_grading_count: Option<u64>,
}

/// `GET /courses/:id/assignments/:id/peer_reviews`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PeerReview {
    pub id: u64,
    #[serde(default)]
    pub assessor_id: Option<u64>,
    /// The student whose work is reviewed.
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub workflow_state: Option<String>,
    /// With `include[]=user`; absent for anonymous peer reviews.
    #[serde(default)]
    pub user: Option<Author>,
}

/// `GET /courses/:id/grading_standards/:id`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GradingStandard {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub points_based: Option<bool>,
    #[serde(default)]
    pub scaling_factor: Option<f64>,
    #[serde(default)]
    pub grading_scheme: Vec<SchemeEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchemeEntry {
    pub name: String,
    /// Minimum fraction (0.94 for 94%).
    pub value: f64,
}

/// `GET /courses/:id/assignment_groups?include[]=assignments&include[]=submission`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssignmentGroup {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    /// Percent weight; only applies when the course uses weighted groups.
    #[serde(default)]
    pub group_weight: Option<f64>,
    #[serde(default)]
    pub rules: Option<GroupRules>,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GroupRules {
    #[serde(default)]
    pub drop_lowest: Option<u32>,
    #[serde(default)]
    pub drop_highest: Option<u32>,
    #[serde(default)]
    pub never_drop: Vec<u64>,
}
