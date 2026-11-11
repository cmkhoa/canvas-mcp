//! Slim views of Canvas objects: only the fields the tools use. Serde ignores the rest.
//! Canvas omits or nulls many fields, so anything not guaranteed is an `Option`.
mod activity;
mod assignments;
mod content;
mod courses;
mod serde_helpers;

pub use activity::*;
pub use assignments::*;
pub use content::*;
pub use courses::*;

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
    fn posted_at_distinguishes_absent_from_null() {
        let absent: Submission = serde_json::from_value(json!({})).unwrap();
        let null: Submission = serde_json::from_value(json!({ "posted_at": null })).unwrap();
        let set: Submission =
            serde_json::from_value(json!({ "posted_at": "2026-01-01T00:00:00Z" })).unwrap();
        assert_eq!(absent.posted_at, None);
        assert_eq!(null.posted_at, Some(None));
        assert!(matches!(set.posted_at, Some(Some(_))));
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
