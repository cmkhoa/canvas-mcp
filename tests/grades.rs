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

/// Labs (40%, drop lowest 1): 9/10, 5/10, 10/10 → 19/20 = 95%.
/// Exams (60%): midterm 75/100, final (id 30) ungraded out of 100.
/// Current = 0.4·95 + 0.6·75 = 83 (B). Final = 38 + 0.6·37.5 = 60.5.
async fn scenario_course(h: &Harness, canvas_current: f64) {
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101",
        json!({ "id": 101, "course_code": "CS 101", "restrict_quantitative_data": false,
                "apply_assignment_group_weights": true, "grading_standard_id": 5,
                "enrollments": [{ "type": "student", "computed_current_score": canvas_current,
                                  "computed_current_grade": "B", "computed_final_score": 60.5 }] }),
    )
    .await;
    h.get(
        "courses/101/grading_standards/5",
        json!({ "id": 5, "title": "CS scale", "grading_scheme": [
            { "name": "A", "value": 0.9 }, { "name": "B", "value": 0.8 }, { "name": "C", "value": 0.7 }, { "name": "F", "value": 0.0 } ] }),
    )
    .await;
    let posted = "2026-10-01T00:00:00Z";
    h.get(
        "courses/101/assignment_groups",
        json!([
            { "id": 1, "name": "Labs", "group_weight": 40.0, "rules": { "drop_lowest": 1 }, "assignments": [
                { "id": 11, "name": "Lab 1", "points_possible": 10.0, "submission": { "score": 9.0, "posted_at": posted } },
                { "id": 12, "name": "Lab 2", "points_possible": 10.0, "submission": { "score": 5.0, "posted_at": posted } },
                { "id": 13, "name": "Lab 3", "points_possible": 10.0, "submission": { "score": 10.0, "posted_at": posted } } ] },
            { "id": 2, "name": "Exams", "group_weight": 60.0, "assignments": [
                { "id": 20, "name": "Midterm", "points_possible": 100.0, "submission": { "score": 75.0, "posted_at": posted } },
                { "id": 30, "name": "Final exam", "points_possible": 100.0, "submission": { "workflow_state": "unsubmitted" } } ] }
        ]),
    )
    .await;
}

#[tokio::test]
async fn scenarios_recompute_and_solve_for_a_letter() {
    let h = Harness::new().await;
    scenario_course(&h, 83.0).await;

    let out = h
        .ok(
            "calculate_grade_scenarios",
            json!({ "course_identifier": "CS 101", "target_letter": "a" }),
        )
        .await;
    assert!(
        out.contains("Letters: \"CS scale\" grading scheme"),
        "{out}"
    );
    assert!(
        out.contains("Recomputed: current 83.00% (B), final 60.50% (F)"),
        "{out}"
    );
    assert!(!out.contains("differs from Canvas"), "{out}");
    assert!(
        out.contains("- \"Labs\" (weight 40%): 19/20 = 95.00% · dropped \"Lab 2\""),
        "{out}"
    );
    // 60.5 + 30x ≥ 90 → x ≥ 98.333…%, reported rounded up.
    assert!(out.contains("To reach A (90.00%): about 98.34% on each of the 1 remaining assignments (100 points in all)."), "{out}");
    assert!(out.contains("Remaining: \"Final exam\" (100 pts)"), "{out}");
}

#[tokio::test]
async fn scenarios_apply_what_ifs_and_flag_disagreement() {
    let h = Harness::new().await;
    scenario_course(&h, 85.0).await;

    let out = h
        .ok(
            "calculate_grade_scenarios",
            json!({ "course_identifier": "CS 101", "hypothetical_scores": { "30": 90 } }),
        )
        .await;
    assert!(out.contains("differs from Canvas's 85.00%"), "{out}");
    // Exams become 165/200 = 82.5% → 38 + 49.5 = 87.5.
    assert!(
        out.contains(
            "With what-ifs (\"Final exam\" = 90/100): current 87.50% (B), final 87.50% (B)"
        ),
        "{out}"
    );
}

#[tokio::test]
async fn scenarios_validate_inputs() {
    let h = Harness::new().await;
    scenario_course(&h, 83.0).await;
    for (args, expected) in [
        (
            json!({ "course_identifier": "CS 101", "target_letter": "A+" }),
            "isn't in this course's scheme (A, B, C, F)",
        ),
        (
            json!({ "course_identifier": "CS 101", "target_letter": "A", "target_percent": 90 }),
            "not both",
        ),
        (
            json!({ "course_identifier": "CS 101", "hypothetical_scores": { "999": 5 } }),
            "Assignment 999 doesn't count",
        ),
        (
            json!({ "course_identifier": "CS 101", "hypothetical_scores": { "30": -1 } }),
            "0 or more",
        ),
        (
            json!({ "course_identifier": "CS 101", "target_percent": 250 }),
            "between 0 and 200",
        ),
    ] {
        let (is_error, text) = h.call("calculate_grade_scenarios", args).await;
        assert!(is_error && text.contains(expected), "{expected}: {text}");
    }
}

#[tokio::test]
async fn scenarios_without_letter_grades_refuse_letter_targets() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/210",
        json!({ "id": 210, "restrict_quantitative_data": false, "apply_assignment_group_weights": false,
                "enrollments": [{ "type": "student", "computed_current_score": 50.0 }] }),
    )
    .await;
    h.get(
        "courses/210/assignment_groups",
        json!([{ "id": 1, "name": "All", "assignments": [
            { "id": 1, "name": "HW", "points_possible": 10.0, "submission": { "score": 5.0 } },
            { "id": 2, "name": "Project", "points_possible": 10.0 } ] }]),
    )
    .await;

    let out = h
        .ok(
            "calculate_grade_scenarios",
            json!({ "course_identifier": "CHEM 210", "target_percent": 70 }),
        )
        .await;
    assert!(
        out.contains("Letters: none") && out.contains("Recomputed: current 50.00%, final 25.00%"),
        "{out}"
    );
    assert!(
        out.contains("To reach 70%: about 90.00% on each of the 1 remaining"),
        "{out}"
    );
    let (is_error, text) = h
        .call(
            "calculate_grade_scenarios",
            json!({ "course_identifier": "CHEM 210", "target_letter": "B" }),
        )
        .await;
    assert!(
        is_error && text.contains("doesn't use letter grades"),
        "{text}"
    );
}

#[tokio::test]
async fn embedded_scheme_is_ignored_when_canvas_shows_no_letters() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/210",
        json!({ "id": 210, "restrict_quantitative_data": false, "apply_assignment_group_weights": false,
                "grading_scheme": [["A", 0.94], ["F", 0.0]],
                "enrollments": [{ "type": "student", "computed_current_score": 95.0 }] }),
    )
    .await;
    h.get(
        "courses/210/assignment_groups",
        json!([{ "id": 1, "name": "All", "assignments": [
            { "id": 1, "name": "HW", "points_possible": 20.0, "submission": { "score": 19.0 } } ] }]),
    )
    .await;
    let out = h
        .ok(
            "calculate_grade_scenarios",
            json!({ "course_identifier": "CHEM 210" }),
        )
        .await;
    assert!(
        out.contains("Letters: none") && out.contains("Canvas reports: current 95.00%"),
        "{out}"
    );
    assert!(
        out.contains("Recomputed: current 95.00%, final 95.00%"),
        "{out}"
    );
}
