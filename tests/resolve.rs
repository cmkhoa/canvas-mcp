use std::sync::Arc;

use canvas_mcp::{CanvasClient, Config, resolve::CourseDirectory};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

fn directory_for(mock: &MockServer) -> CourseDirectory {
    let config = Config::new(&format!("{}/api/v1", mock.uri()), "test-token").unwrap();
    CourseDirectory::new(Arc::new(CanvasClient::new(&config).unwrap()))
}

#[tokio::test]
async fn resolves_names_from_one_cached_course_fetch() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/courses"))
        .and(query_param("enrollment_state", "active"))
        .and(query_param("include[]", "term"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 101, "course_code": "CS 101", "name": "Intro to CS" },
            { "id": 210, "course_code": "CHEM 210", "name": "Organic Chemistry I" }
        ])))
        .expect(1)
        .mount(&mock)
        .await;

    let dir = directory_for(&mock);
    assert_eq!(dir.resolve("cs101").await.unwrap().path_id, "101");
    assert_eq!(dir.resolve("organic").await.unwrap().path_id, "210");
    let err = dir.resolve("biology").await.unwrap_err().to_string();
    assert!(err.contains("Did you mean"), "{err}");
}

#[tokio::test]
async fn numeric_ids_skip_the_course_lookup() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;

    let resolved = directory_for(&mock).resolve(" 4242 ").await.unwrap();
    assert_eq!(resolved.path_id, "4242");
}
