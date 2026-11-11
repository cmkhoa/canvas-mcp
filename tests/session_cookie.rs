use std::time::Duration;

use canvas_mcp::{CanvasClient, Config, canvas::RetryPolicy, error::CanvasError};
use serde::Deserialize;
use serde_json::json;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{header, method, path},
};

#[derive(Debug, Deserialize)]
struct Item {
    id: u64,
}

const COOKIE: &str = "canvas_session=s3cret; _csrf_token=ab%2Bcd%3D%3D; log_session_id=z";

fn cookie_client(mock: &MockServer) -> CanvasClient {
    let config = Config::with_session_cookie(&format!("{}/api/v1", mock.uri()), COOKIE).unwrap();
    CanvasClient::new(&config)
        .unwrap()
        .with_retry_policy(RetryPolicy {
            max_retries: 0,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        })
}

#[tokio::test]
async fn sends_cookie_not_bearer_and_strips_json_prefix() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/users/self/profile"))
        .and(header("cookie", COOKIE))
        .and(|req: &Request| !req.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_string("while(1);{\"id\": 42}"))
        .expect(1)
        .mount(&mock)
        .await;

    let me: Item = cookie_client(&mock)
        .get("users/self/profile", &[])
        .await
        .unwrap();
    assert_eq!(me.id, 42);
}

#[tokio::test]
async fn writes_carry_the_decoded_csrf_token_and_reads_do_not() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("x-csrf-token", "ab+cd=="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 1 })))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(|req: &Request| !req.headers.contains_key("x-csrf-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 2 })))
        .expect(1)
        .mount(&mock)
        .await;

    let client = cookie_client(&mock);
    let posted: Item = client
        .post("planner_notes", &json!({ "title": "x" }))
        .await
        .unwrap();
    let read: Item = client.get("planner_notes/1", &[]).await.unwrap();
    assert_eq!((posted.id, read.id), (1, 2));
}

#[tokio::test]
async fn logged_out_sessions_say_so() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/courses/1"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthenticated"))
        .mount(&mock)
        .await;
    Mock::given(path("/api/v1/courses/2"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/login/saml"))
        .mount(&mock)
        .await;

    let client = cookie_client(&mock);
    for course in ["courses/1", "courses/2"] {
        let err = client.get::<Item>(course, &[]).await.unwrap_err();
        assert!(
            matches!(err, CanvasError::SessionExpired),
            "{course}: {err}"
        );
        assert!(err.to_string().contains("CANVAS_SESSION_COOKIE"));
    }
}
