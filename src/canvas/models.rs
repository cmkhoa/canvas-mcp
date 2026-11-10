//! Slim views of Canvas objects: only the fields the tools use. Serde ignores the rest.
//! Canvas omits or nulls many fields, so anything not guaranteed is an `Option`.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `GET /users/self/profile`. Email is deliberately not modeled.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub login_id: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
}

/// `GET /courses`. Courses closed by date come back with only `id` and
/// `access_restricted_by_date`, so even `name` is optional.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Course {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub course_code: Option<String>,
    #[serde(default)]
    pub sis_course_id: Option<String>,
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
    /// Present with `include[]=term`.
    #[serde(default)]
    pub term: Option<Term>,
    #[serde(default)]
    pub access_restricted_by_date: Option<bool>,
    /// The caller's own enrollments; Canvas includes them on `/courses` by default.
    #[serde(default)]
    pub enrollments: Option<Vec<Enrollment>>,
    /// Present with `include[]=syllabus_body`.
    #[serde(default)]
    pub syllabus_body: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub default_view: Option<String>,
    #[serde(default)]
    pub apply_assignment_group_weights: Option<bool>,
}

/// One of the caller's enrollments, as embedded in a course. Score fields need
/// `include[]=total_scores` (and `current_grading_period_scores` for the period ones).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Enrollment {
    /// `student`, `teacher`, `ta`, `observer` or `designer`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Role name; differs from the default (e.g. `StudentEnrollment`) for custom roles.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub enrollment_state: Option<String>,
    #[serde(default)]
    pub computed_current_score: Option<f64>,
    #[serde(default)]
    pub computed_final_score: Option<f64>,
    #[serde(default)]
    pub computed_current_grade: Option<String>,
    #[serde(default)]
    pub computed_final_grade: Option<String>,
    #[serde(default)]
    pub has_grading_periods: Option<bool>,
    #[serde(default)]
    pub current_grading_period_title: Option<String>,
    #[serde(default)]
    pub current_period_computed_current_score: Option<f64>,
    #[serde(default)]
    pub current_period_computed_current_grade: Option<String>,
}

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
    /// Absent while the instructor hasn't released the grade.
    #[serde(default)]
    pub posted_at: Option<DateTime<Utc>>,
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

/// A Canvas file (course/group files, submission attachments).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct File {
    pub id: u64,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, rename = "content-type")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Signed download link; may point at a different host than the API.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
}

impl File {
    pub fn name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.filename.as_deref())
            .unwrap_or("unnamed file")
    }
}

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

/// `GET /users/self/todo`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TodoItem {
    /// `submitting` for students.
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
}

/// Accepts `false` (or null) as `None`, otherwise deserializes `T`.
fn false_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Bool(false) | serde_json::Value::Null => Ok(None),
        other => serde_json::from_value(other)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Term {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub start_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
}

impl Course {
    /// The caller's roles in this course, e.g. `["student"]`.
    pub fn my_roles(&self) -> Vec<String> {
        let mut roles: Vec<String> = self
            .enrollments
            .iter()
            .flatten()
            .filter_map(|e| {
                let kind = e.kind.clone()?;
                let default_role = format!("{}Enrollment", capitalize(&kind));
                Some(match &e.role {
                    Some(role)
                        if *role != default_role && !role.eq_ignore_ascii_case("TaEnrollment") =>
                    {
                        format!("{kind} ({role})")
                    }
                    _ => kind,
                })
            })
            .collect();
        roles.dedup();
        roles
    }

    /// Short human label: course code, else name, else the numeric ID.
    pub fn label(&self) -> String {
        self.course_code
            .as_deref()
            .or(self.name.as_deref())
            .map_or_else(|| format!("course {}", self.id), str::to_owned)
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_full_course_with_term() {
        let c: Course = serde_json::from_value(json!({
            "id": 101,
            "name": "Introduction to Computer Science",
            "course_code": "CS 101",
            "workflow_state": "available",
            "start_at": "2026-08-24T05:00:00Z",
            "end_at": null,
            "enrollment_term_id": 9,
            "term": { "id": 9, "name": "Fall 2026", "start_at": "2026-08-20T05:00:00Z", "end_at": null },
            "enrollments": [{ "type": "student" }]
        }))
        .unwrap();
        assert_eq!(c.label(), "CS 101");
        assert_eq!(c.term.unwrap().name.as_deref(), Some("Fall 2026"));
        assert!(c.start_at.is_some() && c.end_at.is_none());
    }

    #[test]
    fn roles_show_custom_role_names_only() {
        let c: Course = serde_json::from_value(json!({
            "id": 1,
            "enrollments": [
                { "type": "student", "role": "StudentEnrollment" },
                { "type": "ta", "role": "TaEnrollment" },
                { "type": "student", "role": "Auditor" }
            ]
        }))
        .unwrap();
        assert_eq!(c.my_roles(), vec!["student", "ta", "student (Auditor)"]);
    }

    #[test]
    fn planner_submissions_false_means_none() {
        let items: Vec<PlannerItem> = serde_json::from_value(json!([
            { "plannable_type": "wiki_page", "submissions": false },
            { "plannable_type": "assignment", "submissions": { "submitted": true, "late": true } }
        ]))
        .unwrap();
        assert!(items[0].submissions.is_none());
        let subs = items[1].submissions.as_ref().unwrap();
        assert!(subs.submitted && subs.late && !subs.missing);
    }

    #[test]
    fn parses_date_restricted_course() {
        let c: Course =
            serde_json::from_value(json!({ "id": 7, "access_restricted_by_date": true })).unwrap();
        assert_eq!(c.label(), "course 7");
        assert_eq!(c.access_restricted_by_date, Some(true));
    }
}
