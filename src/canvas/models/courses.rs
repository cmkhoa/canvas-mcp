//! People, courses and enrollments.
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
    /// With `include[]=restrict_quantitative_data`: Canvas hides points and scores from students.
    #[serde(default)]
    pub restrict_quantitative_data: Option<bool>,
    /// The course's own letter scheme, if it has one.
    #[serde(default)]
    pub grading_standard_id: Option<u64>,
    /// With `include[]=grading_scheme` (undocumented): `[["A", 0.94], …]` or `[{name, value}, …]`.
    #[serde(default)]
    pub grading_scheme: Option<serde_json::Value>,
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

/// A person as Canvas lists them to classmates (`/groups/:id/users`). Emails are never modeled.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct User {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub short_name: Option<String>,
}

/// A Canvas group (`/users/self/groups`, `/groups/:id`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Group {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub course_id: Option<u64>,
    #[serde(default)]
    pub group_category_id: Option<u64>,
    #[serde(default)]
    pub members_count: Option<u64>,
    #[serde(default)]
    pub context_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Author {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub display_name: Option<String>,
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
