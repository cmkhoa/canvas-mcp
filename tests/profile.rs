use canvas_mcp::{CanvasClient, CanvasServer, Config};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

async fn server_for(mock: &MockServer) -> CanvasServer {
    let config = Config::new(&format!("{}/api/v1", mock.uri()), "test-token").unwrap();
    CanvasServer::new(CanvasClient::new(&config).unwrap())
}

#[tokio::test]
async fn get_my_profile_formats_profile_without_email() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self/profile"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42,
            "name": "Ada Lovelace",
            "short_name": "Ada",
            "login_id": "ada",
            "primary_email": "ada@example.edu",
            "time_zone": "America/Chicago"
        })))
        .expect(1)
        .mount(&mock)
        .await;

    let out = server_for(&mock).await.get_my_profile().await.unwrap();

    assert!(out.contains("Ada Lovelace"));
    assert!(out.contains("42"));
    assert!(out.contains("America/Chicago"));
    assert!(!out.contains("ada@example.edu"));
}

#[tokio::test]
async fn expired_token_becomes_readable_tool_error() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"errors": [{"message": "Invalid access token."}]})),
        )
        .mount(&mock)
        .await;

    let err = server_for(&mock).await.get_my_profile().await.unwrap_err();
    assert!(err.contains("401"), "{err}");
}

#[tokio::test]
async fn throttling_403_is_reported_as_rate_limit() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string("403 Forbidden (Rate Limit Exceeded)"),
        )
        .mount(&mock)
        .await;

    let err = server_for(&mock).await.get_my_profile().await.unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
}
