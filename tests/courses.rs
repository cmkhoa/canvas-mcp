mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param, query_param_is_missing},
};

#[tokio::test]
async fn list_courses_uses_cached_active_courses() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;

    let out = h.ok("list_courses", json!({})).await;
    assert!(
        out.contains("CS 101 — Intro to Computer Science (id 101) · Fall 2026 · you: student"),
        "{out}"
    );
    assert!(out.contains("CHEM 210"));
}

#[tokio::test]
async fn enrollments_drop_enrollment_state_for_concluded() {
    let h = Harness::new().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .and(query_param_is_missing("enrollment_state"))
        .and(query_param("state[]", "completed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 9, "course_code": "HIST 100", "name": "World History",
              "enrollments": [{ "type": "ta", "role": "TaEnrollment" }] }
        ])))
        .expect(1)
        .mount(&h.mock)
        .await;

    let out = h
        .ok("get_my_enrollments", json!({ "include_concluded": true }))
        .await;
    assert!(
        out.contains("HIST 100 — World History (id 9) · you: ta"),
        "{out}"
    );
    assert!(!out.contains("Active enrollments only"));
}

#[tokio::test]
async fn course_details_resolve_code_and_fence_syllabus_preview() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101",
        json!({
            "id": 101, "course_code": "CS 101", "name": "Intro to Computer Science",
            "start_at": "2026-08-24T05:00:00Z", "time_zone": "America/Chicago",
            "default_view": "modules",
            "syllabus_body": "<p>Office hours Tue.</p>",
            "enrollments": [{ "type": "student" }]
        }),
    )
    .await;

    let out = h
        .ok(
            "get_course_details",
            json!({ "course_identifier": "cs101" }),
        )
        .await;
    assert!(out.contains("Course time zone: America/Chicago"), "{out}");
    assert!(
        out.contains("<untrusted source=\"syllabus: CS 101\">\nOffice hours Tue.\n</untrusted>"),
        "{out}"
    );
}

#[tokio::test]
async fn syllabus_is_complete_unless_capped() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    let long = format!("<p>{}</p>", "word ".repeat(200));
    h.get(
        "courses/210",
        json!({ "id": 210, "course_code": "CHEM 210", "syllabus_body": long }),
    )
    .await;

    let full = h
        .ok("get_syllabus", json!({ "course_identifier": "CHEM 210" }))
        .await;
    assert!(!full.contains("truncated"), "{full}");

    let capped = h
        .ok(
            "get_syllabus",
            json!({ "course_identifier": "CHEM 210", "max_chars": 50, "output_format": "html" }),
        )
        .await;
    assert!(capped.contains("…[truncated: showing 50 of"), "{capped}");
    assert!(capped.contains("<p>word"), "{capped}");
}

#[tokio::test]
async fn unknown_course_is_a_tool_error_with_suggestions() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    let (is_error, text) = h
        .call("get_syllabus", json!({ "course_identifier": "chem 201" }))
        .await;
    assert!(is_error);
    assert!(text.contains("Did you mean: CHEM 210"), "{text}");
}

#[tokio::test]
async fn tools_are_registered() {
    let h = Harness::new().await;
    let names = h.tool_names().await;
    for tool in [
        "get_my_profile",
        "get_my_enrollments",
        "list_courses",
        "get_course_details",
        "get_syllabus",
    ] {
        assert!(
            names.iter().any(|n| n == tool),
            "{tool} missing from {names:?}"
        );
    }
}
