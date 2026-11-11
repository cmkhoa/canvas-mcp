mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{path, query_param},
};

async fn harness() -> Harness {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get("users/self/profile", json!({ "id": 42, "name": "Me" }))
        .await;
    h.get(
        "users/self/groups",
        json!([{ "id": 7, "name": "Team", "course_id": 101 }]),
    )
    .await;
    h
}

#[tokio::test]
async fn calendar_merges_events_and_assignments_across_contexts() {
    let h = harness().await;
    Mock::given(path("/api/v1/calendar_events"))
        .and(query_param("type", "event"))
        .and(query_param("context_codes[]", "user_42"))
        .and(query_param("context_codes[]", "course_210"))
        .and(query_param("context_codes[]", "group_7"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 900, "title": "Office hours", "context_name": "Organic Chemistry I", "context_code": "course_210",
              "start_at": "2026-11-12T20:00:00Z", "end_at": "2026-11-12T21:00:00Z", "location_name": "SEL 4250",
              "description": "<p>Bring questions.</p>" },
            { "id": 901, "title": "Thanksgiving", "context_name": "Me", "context_code": "user_42", "all_day": true,
              "start_at": "2026-11-26T06:00:00Z" }
        ])))
        .mount(&h.mock)
        .await;
    Mock::given(path("/api/v1/calendar_events"))
        .and(query_param("type", "assignment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": "assignment_55", "title": "Lab 3", "context_name": "Intro to Computer Science",
              "context_code": "course_101", "all_day": true, "start_at": "2026-11-11T05:59:00Z",
              "assignment": { "id": 55, "due_at": "2026-11-11T05:59:00Z" } }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "list_calendar_events",
            json!({ "start_date": "2026-11-10", "days": 30, "include_descriptions": true }),
        )
        .await;
    let lab = out.find("\"Lab 3\" (assignment 55)").expect(&out);
    let hours = out.find("\"Office hours\"").expect(&out);
    let holiday = out.find("\"Thanksgiving\"").expect(&out);
    assert!(lab < hours && hours < holiday, "{out}");
    assert!(
        out.contains("Wed Nov 11, 5:59 AM UTC")
            && out.contains("· CS 101 · \"Lab 3\" (assignment 55)"),
        "{out}"
    );
    assert!(out.contains("· CHEM 210 · \"Office hours\""), "{out}");
    assert!(
        out.contains("· at \"SEL 4250\" · event 900\n  Bring questions."),
        "{out}"
    );
    assert!(
        out.contains("Thu Nov 26 (all day) · personal · \"Thanksgiving\""),
        "{out}"
    );
    assert!(out.contains("<untrusted source=\"calendar\">"), "{out}");
}

#[tokio::test]
async fn calendar_validates_windows() {
    let h = harness().await;
    let (is_error, text) = h.call("list_calendar_events", json!({ "days": 0 })).await;
    assert!(
        is_error && text.contains("days must be between 1 and 366"),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "list_calendar_events",
            json!({ "start_date": "2026-01-01", "end_date": "2027-06-01" }),
        )
        .await;
    assert!(is_error && text.contains("at most 366 days"), "{text}");
}

#[tokio::test]
async fn single_event_and_planner_notes() {
    let h = harness().await;
    h.get(
        "calendar_events/900",
        json!({ "id": 900, "title": "Office hours", "context_name": "CHEM 210", "start_at": "2026-11-12T20:00:00Z",
                "location_address": "845 W Taylor St", "description": "<p>Room may change.</p>" }),
    )
    .await;
    Mock::given(path("/api/v1/planner_notes"))
        .and(query_param("context_codes[]", "course_101"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 3, "title": "Study ch. 5", "todo_date": "2026-11-13T18:00:00Z", "course_id": 101, "details": "with Sam" }
        ])))
        .mount(&h.mock)
        .await;

    let event = h.ok("get_calendar_event", json!({ "event_id": 900 })).await;
    assert!(
        event.contains("Address: \"845 W Taylor St\"") && event.contains("Room may change."),
        "{event}"
    );

    let notes = h
        .ok(
            "list_planner_notes",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        notes.contains("- [note 3] ") && notes.contains("\"Study ch. 5\" · CS 101\n  with Sam"),
        "{notes}"
    );
}
