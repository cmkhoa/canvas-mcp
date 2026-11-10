mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param},
};

fn modules_fixture() -> serde_json::Value {
    json!([
        { "id": 1, "name": "Week 1", "state": "completed", "items_count": 3, "items": [
            { "id": 11, "type": "SubHeader", "title": "Readings" },
            { "id": 12, "type": "Page", "title": "Welcome", "page_url": "welcome", "indent": 1,
              "completion_requirement": { "type": "must_view", "completed": true } },
            { "id": 13, "type": "Assignment", "title": "HW 1", "content_id": 55,
              "content_details": { "points_possible": 10.0, "due_at": "2026-09-01T05:59:00Z" },
              "completion_requirement": { "type": "min_score", "min_score": 7.0, "completed": false } }
        ] },
        { "id": 2, "name": "Week 2", "state": "locked", "prerequisite_module_ids": [1],
          "require_sequential_progress": true }
    ])
}

#[tokio::test]
async fn modules_show_progress_items_and_requirements() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/modules"))
        .and(query_param("include[]", "items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(modules_fixture()))
        .mount(&h.mock)
        .await;
    // Week 2 came back without items, so they're fetched separately.
    h.get(
        "courses/101/modules/2/items",
        json!([{ "id": 21, "type": "Quiz", "title": "Quiz 2", "content_id": 77 }]),
    )
    .await;

    let out = h
        .ok(
            "list_modules",
            json!({ "course_identifier": "CS 101", "include_items": true }),
        )
        .await;
    assert!(
        out.contains("[module 1] \"Week 1\" · completed · 3 items"),
        "{out}"
    );
    assert!(out.contains("  — \"Readings\" —"), "{out}");
    assert!(
        out.contains("    - Page: \"Welcome\" (page welcome) [item 12] · to complete: view (done)"),
        "{out}"
    );
    assert!(
        out.contains("- Assignment: \"HW 1\" (assignment id 55) [item 13] · 10 pts · due"),
        "{out}"
    );
    assert!(
        out.contains("to complete: score at least 7 (not done)"),
        "{out}"
    );
    assert!(
        out.contains(
            "[module 2] \"Week 2\" · locked · complete items in order · requires module 1"
        ),
        "{out}"
    );
    assert!(
        out.contains("- Quiz: \"Quiz 2\" (quiz id 77) [item 21]"),
        "{out}"
    );
}

#[tokio::test]
async fn hidden_tabs_are_explained() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/pages"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string("{\"message\":\"That page has been disabled\"}"),
        )
        .mount(&h.mock)
        .await;
    let (is_error, text) = h
        .call("list_pages", json!({ "course_identifier": "CS 101" }))
        .await;
    assert!(
        is_error && text.contains("Pages tab may be hidden"),
        "{text}"
    );
}

#[tokio::test]
async fn pages_list_and_read_with_encoded_slugs() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/pages"))
        .and(query_param("sort", "updated_at"))
        .and(query_param("order", "desc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "url": "week-3", "title": "Week 3", "updated_at": "2026-11-01T00:00:00Z", "front_page": true }
        ])))
        .mount(&h.mock)
        .await;
    h.get(
        "courses/101/pages/notes%20%26%20tips",
        json!({ "url": "notes-tips", "title": "Notes & tips", "body": "<p>Bring a calculator.</p>",
                "last_edited_by": { "display_name": "Prof. Lee" }, "html_url": "https://x/p" }),
    )
    .await;

    let list = h
        .ok(
            "list_pages",
            json!({ "course_identifier": "CS 101", "sort": "updated_at", "order": "desc" }),
        )
        .await;
    assert!(
        list.contains("- \"Week 3\" (page week-3) · front page · updated"),
        "{list}"
    );

    let content = h
        .ok(
            "get_page_content",
            json!({ "course_identifier": "CS 101", "page_url_or_id": "notes & tips" }),
        )
        .await;
    assert!(content.contains("by \"Prof. Lee\""), "{content}");
    assert!(
        content.contains("<untrusted source=\"page: Notes & tips\">\nBring a calculator."),
        "{content}"
    );

    let details = h
        .ok(
            "get_page_details",
            json!({ "course_identifier": "CS 101", "page_url_or_id": "notes & tips" }),
        )
        .await;
    assert!(
        details.contains("Link: https://x/p") && details.contains("Preview"),
        "{details}"
    );
}

#[tokio::test]
async fn front_page_missing_is_explained() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/front_page"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&h.mock)
        .await;
    let (is_error, text) = h
        .call("get_front_page", json!({ "course_identifier": "CS 101" }))
        .await;
    assert!(is_error && text.contains("has no front page"), "{text}");
}

#[tokio::test]
async fn overview_reports_hidden_sections_without_failing() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101",
        json!({ "id": 101, "syllabus_body": "<p>Grading: 50% exams.</p>" }),
    )
    .await;
    Mock::given(path("/api/v1/courses/101/pages"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&h.mock)
        .await;
    h.get("courses/101/modules", modules_fixture()).await;
    h.get("courses/101/modules/2/items", json!([])).await;

    let out = h
        .ok(
            "get_course_content_overview",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        out.contains("== Syllabus\n<untrusted source=\"syllabus: CS 101\">\nGrading: 50% exams."),
        "{out}"
    );
    assert!(
        out.contains("== Pages\n(unavailable:") && out.contains("Pages tab may be hidden"),
        "{out}"
    );
    assert!(out.contains("== Modules\n[module 1]"), "{out}");

    let structure = h
        .ok(
            "get_course_structure",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        structure.starts_with("Structure of CS 101 (2 modules, 3 items)"),
        "{structure}"
    );
}

#[tokio::test]
async fn module_items_include_module_header() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101/modules/1",
        json!({ "id": 1, "name": "Week 1", "state": "started" }),
    )
    .await;
    Mock::given(path("/api/v1/courses/101/modules/1/items"))
        .and(query_param("include[]", "content_details"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 14, "type": "ExternalUrl", "title": "Docs", "external_url": "https://docs.rs" }
        ])))
        .mount(&h.mock)
        .await;
    let out = h
        .ok(
            "list_module_items",
            json!({ "course_identifier": "CS 101", "module_id": 1 }),
        )
        .await;
    assert!(
        out.starts_with("[module 1] \"Week 1\" · started in CS 101:"),
        "{out}"
    );
    assert!(
        out.contains("- ExternalUrl: \"Docs\" (https://docs.rs) [item 14]"),
        "{out}"
    );
}
