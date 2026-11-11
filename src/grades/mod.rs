//! How Canvas treats a student's submissions when it computes grades. Shared by the score
//! listing and (Phase 5) the grade calculator.
pub mod calc;

use crate::canvas::models::{Assignment, Submission};

/// Canvas's student-visible grade ignores a submission without `posted_at` (and its excusal),
/// so it counts like ungraded work. Only applies when Canvas sent the key.
pub fn is_unposted(sub: &Submission) -> bool {
    matches!(sub.posted_at, Some(None))
}

/// The instructor graded or excused this but hasn't released it, so Canvas hides the score.
/// Without a `posted_at` key (older payloads) the only sign is a graded submission with no score.
pub fn grade_not_posted(sub: &Submission) -> bool {
    let graded = sub.workflow_state.as_deref() == Some("graded");
    match sub.posted_at {
        Some(None) => sub.excused == Some(true) || graded,
        Some(Some(_)) => false,
        None => graded && sub.score.is_none() && sub.excused != Some(true),
    }
}

/// Whether Canvas counts the assignment toward the course grade.
pub fn assignment_counts(a: &Assignment) -> bool {
    a.omit_from_final_grade != Some(true)
        && a.grading_type.as_deref() != Some("not_graded")
        && !a.submission_types.iter().any(|t| t == "not_graded")
        && a.published != Some(false)
}

/// Plain-language status labels for one of the caller's assignments.
pub fn status_labels(a: &Assignment) -> Vec<String> {
    let mut labels = Vec::new();
    if a.omit_from_final_grade == Some(true) {
        labels.push("not counted toward final grade".to_owned());
    }
    if a.grading_type.as_deref() == Some("not_graded")
        || a.submission_types.iter().any(|t| t == "not_graded")
    {
        labels.push("not graded".to_owned());
    }
    let Some(sub) = &a.submission else {
        labels.push("no submission record visible".to_owned());
        return labels;
    };
    let main = if grade_not_posted(sub) {
        "grade not posted yet (hidden from you)"
    } else if sub.excused == Some(true) {
        "excused"
    } else if sub.workflow_state.as_deref() == Some("pending_review") {
        "pending review (needs manual grading)"
    } else if sub.score.is_some() {
        "graded"
    } else if sub.missing == Some(true) {
        "missing"
    } else if sub.submitted_at.is_some() {
        "submitted, not graded yet"
    } else {
        "unsubmitted"
    };
    labels.push(main.to_owned());
    if sub.score.is_some() && sub.missing == Some(true) {
        labels.push("marked missing".to_owned());
    }
    if sub.late == Some(true) {
        labels.push("late".to_owned());
    }
    if let Some(d) = sub.points_deducted.filter(|d| *d > 0.0) {
        labels.push(format!("late penalty -{} pts", crate::format::points(d)));
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assignment(v: serde_json::Value) -> Assignment {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn unposted_grades_are_hidden_but_absent_key_is_not_unposted() {
        let hidden = assignment(json!({ "id": 1, "submission": {
            "workflow_state": "graded", "posted_at": null, "score": null } }));
        assert_eq!(
            status_labels(&hidden),
            vec!["grade not posted yet (hidden from you)"]
        );
        assert!(is_unposted(hidden.submission.as_ref().unwrap()));

        let old_payload = assignment(json!({ "id": 2, "submission": {
            "workflow_state": "graded", "score": 9.0 } }));
        assert!(!is_unposted(old_payload.submission.as_ref().unwrap()));
        assert_eq!(status_labels(&old_payload), vec!["graded"]);
    }

    #[test]
    fn labels_cover_missing_late_and_penalties() {
        let a = assignment(json!({ "id": 3, "submission": {
            "score": 7.0, "missing": true, "late": true, "points_deducted": 1.5,
            "posted_at": "2026-01-01T00:00:00Z" } }));
        assert_eq!(
            status_labels(&a),
            vec!["graded", "marked missing", "late", "late penalty -1.5 pts"]
        );
    }

    #[test]
    fn omitted_and_ungraded_assignments_do_not_count() {
        assert!(!assignment_counts(&assignment(
            json!({ "id": 4, "omit_from_final_grade": true })
        )));
        assert!(!assignment_counts(&assignment(
            json!({ "id": 5, "grading_type": "not_graded" })
        )));
        assert!(assignment_counts(&assignment(
            json!({ "id": 6, "published": true })
        )));
    }
}
