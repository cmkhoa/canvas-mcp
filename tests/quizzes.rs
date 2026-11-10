mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{path, query_param},
};

#[tokio::test]
async fn list_quizzes_covers_classic_and_new_quizzes() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101/quizzes",
        json!([
            { "id": 12, "title": "Midterm", "quiz_type": "assignment", "assignment_id": 77, "time_limit": 60,
              "allowed_attempts": 2, "points_possible": 50.0, "due_at": "2026-11-20T05:59:00Z" },
            { "id": 13, "title": "Practice", "quiz_type": "practice_quiz", "allowed_attempts": -1 }
        ]),
    )
    .await;
    h.get(
        "courses/101/assignments",
        json!([
            { "id": 77, "name": "Midterm", "quiz_id": 12, "submission": { "submitted_at": "2026-11-01T00:00:00Z" } },
            { "id": 78, "name": "Quiz 3", "is_quiz_lti_assignment": true, "points_possible": 10.0,
              "allowed_attempts": 1, "submission": { "workflow_state": "unsubmitted" } }
        ]),
    )
    .await;

    let out = h
        .ok("list_quizzes", json!({ "course_identifier": "CS 101" }))
        .await;
    assert!(
        out.contains("- [quiz 12] \"Midterm\" · graded quiz · due")
            && out.contains("60 min · 2 attempts · 50 pts · submitted"),
        "{out}"
    );
    assert!(
        out.contains(
            "- [quiz 13] \"Practice\" · practice quiz · due no due date · unlimited attempts"
        ),
        "{out}"
    );
    assert!(
        out.contains("New Quizzes:\n- [assignment 78] \"Quiz 3\"")
            && out.contains("1 attempt · 10 pts · not submitted"),
        "{out}"
    );
}

#[tokio::test]
async fn classic_quiz_details_show_attempts_without_starting_one() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101/quizzes/12",
        json!({ "id": 12, "title": "Midterm", "quiz_type": "assignment", "allowed_attempts": 3,
                "scoring_policy": "keep_highest", "question_count": 20, "one_question_at_a_time": true,
                "cant_go_back": true, "description": "<p>Closed book.</p>" }),
    )
    .await;
    h.get(
        "courses/101/quizzes/12/submission",
        json!({ "quiz_submissions": [{ "attempt": 2, "score": 41.0, "kept_score": 44.0, "attempts_left": 1,
                                       "workflow_state": "complete", "finished_at": "2026-11-02T00:00:00Z" }] }),
    )
    .await;

    let out = h
        .ok(
            "get_quiz_details",
            json!({ "course_identifier": "CS 101", "quiz_id": 12 }),
        )
        .await;
    assert!(
        out.contains("Questions: 20") && out.contains("Score kept: highest"),
        "{out}"
    );
    assert!(
        out.contains("One question at a time, no going back"),
        "{out}"
    );
    assert!(
        out.contains("Your attempts: 2 used, 1 left") && out.contains("Kept score: 44"),
        "{out}"
    );
    assert!(
        out.contains("<untrusted source=\"quiz instructions\">\nClosed book."),
        "{out}"
    );
}

#[tokio::test]
async fn untaken_quiz_and_new_quiz_details() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "courses/101/quizzes/14",
        json!({ "id": 14, "title": "Quiz 4" }),
    )
    .await;
    Mock::given(path("/api/v1/courses/101/quizzes/14/submission"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&h.mock)
        .await;
    Mock::given(path("/api/v1/courses/101/assignments/78"))
        .and(query_param("include[]", "submission"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 78, "name": "Quiz 3", "is_quiz_lti_assignment": true, "allowed_attempts": 1,
            "submission": { "attempt": 1, "submitted_at": "2026-11-01T00:00:00Z", "score": 9.0 } })))
        .mount(&h.mock)
        .await;

    let untaken = h
        .ok(
            "get_quiz_details",
            json!({ "course_identifier": "CS 101", "quiz_id": 14 }),
        )
        .await;
    assert!(untaken.contains("Your attempts: none yet"), "{untaken}");
    let new_quiz = h
        .ok(
            "get_quiz_details",
            json!({ "course_identifier": "CS 101", "assignment_id": 78 }),
        )
        .await;
    assert!(
        new_quiz.contains("(New Quiz, assignment 78)")
            && new_quiz.contains("submitted · 1 attempts used"),
        "{new_quiz}"
    );
    assert!(
        new_quiz.contains("doesn't expose New Quiz settings"),
        "{new_quiz}"
    );

    let (is_error, text) = h
        .call(
            "get_quiz_details",
            json!({ "course_identifier": "CS 101", "quiz_id": 1, "assignment_id": 2 }),
        )
        .await;
    assert!(is_error && text.contains("exactly one"), "{text}");
}

#[tokio::test]
async fn peer_reviews_merge_planner_and_assignment_sources() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get("users/self", json!({ "id": 42, "name": "Me" })).await;
    Mock::given(path("/api/v1/planner/items"))
        .and(query_param("filter", "incomplete_items"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "course_id": 101, "plannable_type": "assessment_request", "plannable_id": 900,
              "plannable": { "id": 900, "title": "Essay 1", "workflow_state": "assigned" },
              "html_url": "/courses/101/assignments/55/submissions/7" },
            { "course_id": 101, "plannable_type": "assessment_request",
              "plannable": { "id": 901, "title": "Old", "workflow_state": "completed" } }
        ])))
        .mount(&h.mock)
        .await;
    h.get(
        "courses/101/assignments",
        json!([{ "id": 55, "name": "Essay 1", "peer_reviews": true }]),
    )
    .await;
    h.get(
        "courses/101/assignments/55/peer_reviews",
        json!([
            { "id": 900, "assessor_id": 42, "workflow_state": "assigned", "user": { "display_name": "Sam" } },
            { "id": 902, "assessor_id": 7, "workflow_state": "assigned" }
        ]),
    )
    .await;
    h.get(
        "courses/210/assignments",
        json!([{ "id": 60, "name": "Lab report", "peer_reviews": true }]),
    )
    .await;
    Mock::given(path("/api/v1/courses/210/assignments/60/peer_reviews"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&h.mock)
        .await;

    let out = h.ok("get_my_peer_reviews_todo", json!({})).await;
    assert!(
        out.contains("Peer reviews you need to complete (1):"),
        "{out}"
    );
    assert!(
        out.contains("- CS 101 · \"Essay 1\" (assignment 55) · reviewing \"Sam\""),
        "{out}"
    );
    assert!(!out.contains("Old"), "{out}");
    assert!(
        out.contains("Could not check: \"Lab report\" in CHEM 210"),
        "{out}"
    );
}
