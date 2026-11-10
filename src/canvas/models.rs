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
    /// Short human label: course code, else name, else the numeric ID.
    pub fn label(&self) -> String {
        self.course_code
            .as_deref()
            .or(self.name.as_deref())
            .map_or_else(|| format!("course {}", self.id), str::to_owned)
    }
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
    fn parses_date_restricted_course() {
        let c: Course =
            serde_json::from_value(json!({ "id": 7, "access_restricted_by_date": true })).unwrap();
        assert_eq!(c.label(), "course 7");
        assert_eq!(c.access_restricted_by_date, Some(true));
    }
}
