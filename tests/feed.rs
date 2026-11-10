mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[tokio::test]
async fn course_announcements_are_fenced_newest_first() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/discussion_topics"))
        .and(query_param("only_announcements", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 2, "title": "Exam moved", "posted_at": "2026-11-09T15:00:00Z",
              "message": "<p>Now on <b>Friday</b>.</p>", "author": { "display_name": "Prof. Lee" } }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "list_announcements",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        out.contains("<untrusted source=\"announcements: CS 101\">"),
        "{out}"
    );
    assert!(
        out.contains("[id 2] ")
            && out.contains("· CS 101 · \"Exam moved\" · by \"Prof. Lee\"\nNow on **Friday**."),
        "{out}"
    );
}

#[tokio::test]
async fn my_announcements_retry_per_course_when_a_chunk_fails() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    // The batched request (both courses) fails because of one course.
    Mock::given(path("/api/v1/announcements"))
        .and(query_param("context_codes[]", "course_101"))
        .and(query_param("context_codes[]", "course_210"))
        .respond_with(ResponseTemplate::new(403).set_body_string("not authorized"))
        .with_priority(1)
        .expect(1)
        .mount(&h.mock)
        .await;
    Mock::given(path("/api/v1/announcements"))
        .and(query_param("context_codes[]", "course_101"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 5, "title": "Lab open", "context_code": "course_101", "posted_at": "2026-11-08T12:00:00Z",
              "message": "<p>Lab is open.</p>" }
        ])))
        .mount(&h.mock)
        .await;
    Mock::given(path("/api/v1/announcements"))
        .and(query_param("context_codes[]", "course_210"))
        .respond_with(ResponseTemplate::new(403).set_body_string("not authorized"))
        .mount(&h.mock)
        .await;

    let out = h
        .ok("list_my_announcements", json!({ "preview_chars": 0 }))
        .await;
    assert!(
        out.contains("[id 5]") && out.contains("CS 101 · \"Lab open\""),
        "{out}"
    );
    assert!(
        !out.contains("Lab is open."),
        "preview_chars=0 shows titles only: {out}"
    );
    assert!(
        out.contains("Could not read: CHEM 210 (Canvas denied access (403)"),
        "{out}"
    );
}

#[tokio::test]
async fn my_announcements_validate_inputs() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    let (is_error, text) = h.call("list_my_announcements", json!({ "limit": 0 })).await;
    assert!(
        is_error && text.contains("limit must be between 1 and 200"),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "list_my_announcements",
            json!({ "start_date": "2026-11-10", "end_date": "2026-11-01" }),
        )
        .await;
    assert!(
        is_error && text.contains("start_date is after end_date"),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "list_my_announcements",
            json!({ "start_date": "last week" }),
        )
        .await;
    assert!(is_error && text.contains("Unreadable date"), "{text}");
}

#[tokio::test]
async fn activity_stream_groups_filters_and_summarizes() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/users/self/activity_stream"))
        .and(query_param("only_active_courses", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "type": "Submission", "title": "Lab 3", "course_id": 101, "grade": "A",
              "updated_at": "2026-11-09T12:00:00Z", "read_state": false },
            { "type": "Announcement", "title": "Exam moved", "course_id": 210,
              "message": "<p>Friday</p>", "updated_at": "2026-11-08T12:00:00Z", "read_state": true },
            { "type": "Collaboration", "title": "Shared doc", "updated_at": "2026-11-07T12:00:00Z" }
        ])))
        .mount(&h.mock)
        .await;
    h.get(
        "users/self/activity_stream/summary",
        json!([
            { "type": "Submission", "count": 4, "unread_count": 1 },
            { "type": "DiscussionTopic", "count": 2, "unread_count": 0 },
            { "type": "DiscussionEntry", "count": 3, "unread_count": 2 }
        ]),
    )
    .await;

    let out = h.ok("get_my_activity_stream", json!({})).await;
    assert!(
        out.contains("Grades & submission comments: 4 (1 unread)"),
        "{out}"
    );
    assert!(out.contains("Discussions: 5 (2 unread)"), "{out}");
    assert!(
        out.contains("== Announcements (1)\n- ")
            && out.contains("CHEM 210 · \"Exam moved\"\n  Friday"),
        "{out}"
    );
    assert!(
        out.contains("CS 101 · \"Lab 3\" · unread · grade \"A\""),
        "{out}"
    );
    assert!(out.contains("== Other activity (1)"), "{out}");

    let only = h
        .ok(
            "get_my_activity_stream",
            json!({ "item_type": "submissions", "include_summary": false }),
        )
        .await;
    assert!(
        only.contains("Lab 3") && !only.contains("Exam moved") && !only.contains("summary"),
        "{only}"
    );
}
