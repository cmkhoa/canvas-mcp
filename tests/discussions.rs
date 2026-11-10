mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path, query_param},
};

async fn harness_with_groups() -> Harness {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "users/self/groups",
        json!([
            { "id": 7, "name": "Team Rocket", "course_id": 101, "group_category_id": 3, "members_count": 4 },
            { "id": 8, "name": "Chem Pod", "course_id": 210, "group_category_id": 4 }
        ]),
    )
    .await;
    h
}

#[tokio::test]
async fn topics_show_graded_and_post_first_flags() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/101/discussion_topics"))
        .and(query_param("include[]", "assignment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 40, "title": "Week 3 debate", "user_name": "Prof. Lee", "posted_at": "2026-11-01T12:00:00Z",
              "discussion_subentry_count": 12, "unread_count": 3, "require_initial_post": true,
              "assignment": { "id": 99, "due_at": "2026-11-20T05:59:00Z" } }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "list_discussion_topics",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(
        out.contains("- [id 40] \"Week 3 debate\" · by \"Prof. Lee\""),
        "{out}"
    );
    assert!(
        out.contains("12 posts (3 unread)") && out.contains("(assignment 99)"),
        "{out}"
    );
    assert!(out.contains("post first to see others"), "{out}");
}

#[tokio::test]
async fn group_ids_must_be_yours_and_in_the_course() {
    let h = harness_with_groups().await;
    h.get(
        "groups/7/discussion_topics",
        json!([{ "id": 1, "title": "Plan" }]),
    )
    .await;

    let out = h
        .ok(
            "list_discussion_topics",
            json!({ "course_identifier": "CS 101", "group_id": 7 }),
        )
        .await;
    assert!(
        out.contains("Discussions in CS 101 › group \"Team Rocket\"") && out.contains("\"Plan\""),
        "{out}"
    );

    let (is_error, text) = h
        .call(
            "list_discussion_topics",
            json!({ "course_identifier": "CS 101", "group_id": 8 }),
        )
        .await;
    assert!(
        is_error && text.contains("doesn't belong to CS 101"),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "list_discussion_topics",
            json!({ "course_identifier": "CS 101", "group_id": 9 }),
        )
        .await;
    assert!(
        is_error && text.contains("not a member of group 9"),
        "{text}"
    );
}

#[tokio::test]
async fn post_first_topics_explain_the_403() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/discussion_topics/40/entries"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_string("{\"errors\":[{\"message\":\"must post first\"}]}"),
        )
        .mount(&h.mock)
        .await;
    h.get(
        "courses/101/discussion_topics/40",
        json!({ "id": 40, "title": "Debate", "require_initial_post": true }),
    )
    .await;

    let (is_error, text) = h
        .call(
            "list_discussion_entries",
            json!({ "course_identifier": "CS 101", "topic_id": 40 }),
        )
        .await;
    assert!(
        is_error && text.contains("requires you to post before you can see"),
        "{text}"
    );
}

#[tokio::test]
async fn entries_preview_by_default_and_fetch_replies_oldest_first() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    let long = format!("<p>{}</p>", "a".repeat(400));
    h.get(
        "courses/101/discussion_topics/40/entries",
        json!([{ "id": 500, "user_name": "Sam", "message": long, "created_at": "2026-11-02T12:00:00Z",
                 "recent_replies": [{ "id": 502 }], "has_more_replies": true }]),
    )
    .await;
    h.get(
        "courses/101/discussion_topics/40/entries/500/replies",
        json!([
            { "id": 502, "user_name": "Kim", "message": "<p>second</p>", "created_at": "2026-11-03T12:00:00Z" },
            { "id": 501, "user_name": "Ana", "message": "<p>first</p>", "created_at": "2026-11-02T13:00:00Z" }
        ]),
    )
    .await;

    let preview = h
        .ok(
            "list_discussion_entries",
            json!({ "course_identifier": "CS 101", "topic_id": 40 }),
        )
        .await;
    assert!(
        preview.contains("…[truncated: showing 300 of 400 characters]"),
        "{preview}"
    );
    assert!(
        preview.contains("(1+ replies; pass include_replies=true"),
        "{preview}"
    );

    let full = h
        .ok(
            "list_discussion_entries",
            json!({ "course_identifier": "CS 101", "topic_id": 40, "include_full_content": true, "include_replies": true }),
        )
        .await;
    assert!(!full.contains("truncated"), "{full}");
    let first = full.find("[reply 501] \"Ana\"").expect(&full);
    let second = full.find("[reply 502] \"Kim\"").expect(&full);
    assert!(
        first < second
            && full.contains("<untrusted source=\"discussion posts by course participants\">"),
        "{full}"
    );
}

#[tokio::test]
async fn entry_details_use_entry_list() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/discussion_topics/40/entry_list"))
        .and(query_param("ids[]", "501"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 501, "user_name": "Ana", "message": "<p>first</p>", "parent_id": 500 }
        ])))
        .mount(&h.mock)
        .await;
    h.get(
        "courses/101/discussion_topics/40/entries/501/replies",
        json!([]),
    )
    .await;

    let out = h
        .ok(
            "get_discussion_entry_details",
            json!({ "course_identifier": "CS 101", "topic_id": 40, "entry_id": 501 }),
        )
        .await;
    assert!(
        out.contains("Entry 501 in topic 40 (CS 101) (a reply to entry 500)"),
        "{out}"
    );
    assert!(
        out.contains("[entry 501] \"Ana\"") && out.contains("first"),
        "{out}"
    );
}

#[tokio::test]
async fn topic_details_and_full_discussion() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101/discussion_topics/40",
        json!({ "id": 40, "title": "Debate", "message": "<p>Argue a side.</p>", "discussion_type": "threaded" }),
    )
    .await;
    h.get(
        "courses/101/discussion_topics/40/entries",
        json!([{ "id": 500, "user_name": "Sam", "message": "<p>Pro.</p>" }]),
    )
    .await;

    let details = h
        .ok(
            "get_discussion_topic_details",
            json!({ "course_identifier": "CS 101", "topic_id": 40 }),
        )
        .await;
    assert!(
        details.contains("Format: threaded")
            && details.contains("discussion prompt: Debate\">\nArgue a side."),
        "{details}"
    );

    let full = h
        .ok(
            "get_discussion_with_replies",
            json!({ "course_identifier": "CS 101", "topic_id": 40 }),
        )
        .await;
    assert!(
        full.starts_with("Discussion \"Debate\" in CS 101 (1 top-level posts)")
            && full.contains("Pro."),
        "{full}"
    );
}

#[tokio::test]
async fn group_topics_mark_copies_and_group_started_topics() {
    let h = harness_with_groups().await;
    h.get(
        "groups/7/discussion_topics",
        json!([{ "id": 70, "title": "Debate", "root_topic_id": 40 }, { "id": 71, "title": "Our plan" }]),
    )
    .await;

    let out = h
        .ok(
            "list_group_discussion_topics",
            json!({ "course_identifier": "CS 101" }),
        )
        .await;
    assert!(out.contains("== \"Team Rocket\" (group 7, set 3)"), "{out}");
    assert!(
        out.contains("\"Debate\"") && out.contains("group copy of course topic 40"),
        "{out}"
    );
    assert!(
        out.contains("\"Our plan\"") && out.contains("started in this group"),
        "{out}"
    );
    assert!(!out.contains("Chem Pod"), "{out}");
}
