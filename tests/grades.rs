mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[tokio::test]
async fn course_grades_show_current_final_and_period() {
    let h = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .and(query_param("include[]", "total_scores"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 101, "course_code": "CS 101", "enrollments": [{
                "type": "student", "computed_current_score": 88.43, "computed_current_grade": "B+",
                "computed_final_score": 61.0, "has_grading_periods": true,
                "current_grading_period_title": "Quarter 2",
                "current_period_computed_current_score": 91.0, "current_period_computed_current_grade": "A-" }] },
            { "id": 210, "course_code": "CHEM 210", "enrollments": [{ "type": "student" }] }
        ])))
        .mount(&h.mock)
        .await;

    let out = h.ok("get_my_course_grades", json!({})).await;
    assert!(
        out.contains(
            "- CS 101 (id 101): current B+ (88.4%) · final 61.0% · \"Quarter 2\": A- (91.0%)"
        ),
        "{out}"
    );
    assert!(
        out.contains("- CHEM 210 (id 210): current no grade yet"),
        "{out}"
    );
}

#[tokio::test]
async fn assignment_scores_group_with_weights_rules_and_statuses() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101",
        json!({ "id": 101, "course_code": "CS 101", "restrict_quantitative_data": false,
                "apply_assignment_group_weights": true }),
    )
    .await;
    h.get(
        "courses/101/assignment_groups",
        json!([
            { "id": 1, "name": "Labs", "group_weight": 40.0, "rules": { "drop_lowest": 1 },
              "assignments": [
                { "id": 11, "name": "Lab 1", "points_possible": 10.0,
                  "submission": { "score": 9.0, "workflow_state": "graded", "posted_at": "2026-09-01T00:00:00Z" } },
                { "id": 12, "name": "Lab 2", "points_possible": 10.0,
                  "submission": { "workflow_state": "graded", "posted_at": null } },
                { "id": 13, "name": "Lab 3", "points_possible": 10.0,
                  "submission": { "excused": true, "posted_at": "2026-09-01T00:00:00Z" } }
              ] },
            { "id": 2, "name": "Exams", "group_weight": 60.0, "assignments": [
                { "id": 21, "name": "Midterm", "points_possible": 100.0,
                  "submission": { "missing": true, "workflow_state": "unsubmitted" } } ] }
        ]),
    )
    .await;

    let out = h
        .ok(
            "get_my_assignment_scores",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(out.contains("Weighting: by assignment group"), "{out}");
    assert!(
        out.contains("== \"Labs\" | weight 40% | drop lowest 1"),
        "{out}"
    );
    assert!(
        out.contains("\"Lab 1\" (id 11): 9/10 (90.0%) | graded"),
        "{out}"
    );
    assert!(
        out.contains("\"Lab 2\" (id 12): -/10 | grade not posted yet (hidden from you)"),
        "{out}"
    );
    assert!(
        out.contains("\"Lab 3\" (id 13): excused | excused"),
        "{out}"
    );
    assert!(
        out.contains("\"Midterm\" (id 21): -/100 | missing"),
        "{out}"
    );
    assert!(
        out.contains(
            "Summary: 1 excused, 1 grade not posted yet (hidden from you), 1 graded, 1 missing"
        ),
        "{out}"
    );
}

#[tokio::test]
async fn assignment_scores_refuse_restricted_or_unknown_courses() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101",
        json!({ "id": 101, "restrict_quantitative_data": true }),
    )
    .await;
    h.get("courses/210", json!({ "id": 210 })).await;

    let (is_error, text) = h
        .call(
            "get_my_assignment_scores",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        is_error && text.contains("hides points and scores"),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "get_my_assignment_scores",
            json!({ "course_identifier": "CHEM 210" }),
        )
        .await;
    assert!(is_error && text.contains("won't guess"), "{text}");
}
