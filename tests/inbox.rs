mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{path, query_param, query_param_is_missing},
};

#[tokio::test]
async fn conversations_list_with_scope_and_filters() {
    let h = Harness::new().await;
    Mock::given(path("/api/v1/conversations"))
        .and(query_param("scope", "unread"))
        .and(query_param("filter[]", "course_101"))
        .and(query_param_is_missing("filter_mode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 12, "subject": "Lab question", "workflow_state": "unread", "message_count": 3,
              "last_message_at": "2026-11-09T15:00:00Z", "context_name": "CS 101",
              "last_message": "Can we use numpy?", "participants": [{ "id": 1, "name": "Ana" }, { "id": 42, "name": "Me" }] }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "list_conversations",
            json!({ "scope": "unread", "filter_ids": ["course_101"] }),
        )
        .await;
    assert!(out.contains("[conversation 12] ") && out.contains("\"Lab question\" · unread · 3 messages · \"CS 101\" · with \"Ana\", \"Me\"\n  Can we use numpy?"), "{out}");

    let (is_error, text) = h
        .call(
            "list_conversations",
            json!({ "filter_ids": ["course_1&x=y"] }),
        )
        .await;
    assert!(
        is_error && text.contains("filter_ids entries look like"),
        "{text}"
    );
}

#[tokio::test]
async fn conversation_details_never_mark_read_and_order_messages() {
    let h = Harness::new().await;
    h.get("users/self/profile", json!({ "id": 42, "name": "Me" }))
        .await;
    Mock::given(path("/api/v1/conversations/12"))
        .and(query_param("auto_mark_as_read", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 12, "subject": "Lab question",
            "participants": [{ "id": 1, "name": "Ana" }, { "id": 42, "name": "Me" }],
            "messages": [
                { "id": 3, "author_id": 1, "created_at": "2026-11-09T15:00:00Z", "body": "Yes, numpy is fine." },
                { "id": 2, "author_id": 42, "created_at": "2026-11-09T14:00:00Z", "body": "Can we use numpy?",
                  "attachments": [{ "id": 77, "display_name": "plot.png" }] }
            ]
        })))
        .expect(1)
        .mount(&h.mock)
        .await;

    let out = h
        .ok("get_conversation_details", json!({ "conversation_id": 12 }))
        .await;
    let mine = out.find("[message 2] \"Me\" (you)").expect(&out);
    let reply = out.find("[message 3] \"Ana\"").expect(&out);
    assert!(mine < reply, "{out}");
    assert!(
        out.contains("attachment: \"plot.png\" (file id 77)"),
        "{out}"
    );
}

#[tokio::test]
async fn unread_count_accepts_canvas_string() {
    let h = Harness::new().await;
    h.get("conversations/unread_count", json!({ "unread_count": "4" }))
        .await;
    assert_eq!(
        h.ok("get_unread_count", json!({})).await,
        "Unread conversations: 4"
    );
}

#[tokio::test]
async fn recipients_are_individuals_with_roles() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/search/recipients"))
        .and(query_param("context", "course_101_teachers"))
        .and(query_param("type", "user"))
        .and(query_param("search", "lee"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 5, "name": "Prof. Lee", "type": "user", "common_courses": { "101": ["TeacherEnrollment"] } },
            { "id": "course_101_teachers", "name": "Teachers", "type": "context" }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "find_message_recipients",
            json!({ "course_identifier": "CS 101", "role": "teacher", "search": "lee" }),
        )
        .await;
    assert!(
        out.contains("- \"Prof. Lee\" (user id 5) · teacher"),
        "{out}"
    );
    assert!(!out.contains("Teachers"), "{out}");
}
