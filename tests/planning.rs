mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn planning_snapshot_returns_structured_contract() {
    let harness = Harness::new().await;
    harness.active_courses(two_courses()).await;

    let assignments = json!([
        {
            "id": 9001,
            "name": "Problem Set 4",
            "due_at": "2026-11-13T05:59:00Z",
            "points_possible": 50.0,
            "submission_types": ["online_upload"],
            "submission": {
                "workflow_state": "unsubmitted",
                "missing": true,
                "submitted_at": null
            }
        },
        {
            "id": 9002,
            "name": "Lab 5",
            "due_at": null,
            "points_possible": 100,
            "submission_types": ["online_upload"],
            "submission": {
                "workflow_state": "unsubmitted",
                "submitted_at": null
            }
        }
    ]);
    for course_id in [101, 210] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/courses/{course_id}/assignments")))
            .respond_with(ResponseTemplate::new(200).set_body_json(assignments.clone()))
            .mount(&harness.mock)
            .await;
    }

    let result = harness.call_raw("get_planning_snapshot", json!({})).await;
    assert_eq!(result.is_error, Some(false));
    let structured = result
        .structured_content
        .expect("structured content is present");
    assert_eq!(structured["schema_version"], "1");
    assert_eq!(structured["courses"].as_array().unwrap().len(), 2);
    let first = &structured["assignments"][0];
    assert_eq!(first["assignment_id"], 9001);
    assert_eq!(first["due_at"], "2026-11-13T05:59:00Z");
    assert_eq!(first["points_possible"], 50.0);
    assert_eq!(first["submission"]["missing"], true);
    assert_eq!(structured["warnings"].as_array().unwrap().len(), 0);
}
