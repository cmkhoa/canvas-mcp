use std::time::Duration;

use canvas_mcp::{
    CanvasClient, Config,
    canvas::{Paged, RetryPolicy},
    error::CanvasError,
};
use serde::Deserialize;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param, query_param_is_missing},
};

#[derive(Debug, Deserialize)]
struct Item {
    id: u64,
}

const FAST_RETRY: RetryPolicy = RetryPolicy {
    max_retries: 2,
    base_delay: Duration::from_millis(1),
    max_delay: Duration::from_millis(5),
};

fn client_for(mock: &MockServer) -> CanvasClient {
    let config = Config::new(&format!("{}/api/v1", mock.uri()), "test-token").unwrap();
    CanvasClient::new(&config)
        .unwrap()
        .with_retry_policy(FAST_RETRY)
}

fn page(ids: std::ops::Range<u64>, next: Option<String>) -> ResponseTemplate {
    let body: Vec<_> = ids.map(|id| json!({ "id": id })).collect();
    let resp = ResponseTemplate::new(200).set_body_json(body);
    match next {
        Some(url) => resp.insert_header("link", format!(r#"<{url}>; rel="next""#).as_str()),
        None => resp,
    }
}

fn ids(paged: &Paged<Item>) -> Vec<u64> {
    paged.items.iter().map(|i| i.id).collect()
}

#[tokio::test]
async fn get_all_follows_next_links_across_pages() {
    let mock = MockServer::start().await;
    let base = format!("{}/api/v1/courses", mock.uri());
    Mock::given(path("/api/v1/courses"))
        .and(query_param_is_missing("page"))
        .and(query_param("per_page", "100"))
        .and(query_param("include[]", "term"))
        .respond_with(page(0..2, Some(format!("{base}?page=2&per_page=100"))))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(path("/api/v1/courses"))
        .and(query_param("page", "2"))
        .respond_with(page(2..4, Some(format!("{base}?page=3&per_page=100"))))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(path("/api/v1/courses"))
        .and(query_param("page", "3"))
        .respond_with(page(4..5, None))
        .expect(1)
        .mount(&mock)
        .await;

    let paged: Paged<Item> = client_for(&mock)
        .get_all("courses", &[("include[]", "term")], 500)
        .await
        .unwrap();

    assert_eq!(ids(&paged), vec![0, 1, 2, 3, 4]);
    assert!(!paged.truncated);
}

#[tokio::test]
async fn get_all_stops_at_limit_and_reports_truncation() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/courses"))
        .and(query_param("per_page", "3"))
        .respond_with(page(
            0..3,
            Some(format!("{}/api/v1/courses?page=2", mock.uri())),
        ))
        .expect(1)
        .mount(&mock)
        .await;

    let paged: Paged<Item> = client_for(&mock).get_all("courses", &[], 3).await.unwrap();

    assert_eq!(ids(&paged), vec![0, 1, 2]);
    assert!(paged.truncated);
}

#[tokio::test]
async fn get_all_refuses_next_link_to_another_host() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/courses"))
        .respond_with(page(0..1, Some("https://evil.example/steal?page=2".into())))
        .mount(&mock)
        .await;

    let err = client_for(&mock)
        .get_all::<Item>("courses", &[], 50)
        .await
        .unwrap_err();

    assert!(
        matches!(err, CanvasError::Decode(ref m) if m.contains("another host")),
        "{err}"
    );
}

#[tokio::test]
async fn throttled_request_is_retried_then_succeeds() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/users/self/profile"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string("403 Forbidden (Rate Limit Exceeded)"),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(path("/api/v1/users/self/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7 })))
        .expect(1)
        .mount(&mock)
        .await;

    let item: Item = client_for(&mock)
        .get("users/self/profile", &[])
        .await
        .unwrap();
    assert_eq!(item.id, 7);
}

#[tokio::test]
async fn throttling_gives_up_after_max_retries() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/users/self/profile"))
        .respond_with(ResponseTemplate::new(403).set_body_string("Rate Limit Exceeded"))
        .expect(1 + u64::from(FAST_RETRY.max_retries))
        .mount(&mock)
        .await;

    let err = client_for(&mock)
        .get::<Item>("users/self/profile", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, CanvasError::Throttled), "{err}");
}

#[tokio::test]
async fn transient_5xx_is_retried_for_get_but_not_post() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 1 })))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&mock)
        .await;

    let client = client_for(&mock);
    let item: Item = client.get("courses/1", &[]).await.unwrap();
    assert_eq!(item.id, 1);

    let err = client
        .post::<_, Item>("courses/1/discussion_topics", &json!({ "title": "x" }))
        .await
        .unwrap_err();
    assert!(
        matches!(err, CanvasError::Http { status: 503, .. }),
        "{err}"
    );
}

#[tokio::test]
async fn validation_errors_are_not_retried() {
    let mock = MockServer::start().await;
    Mock::given(method("PUT"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_json(json!({ "errors": { "name": ["is too long"] } })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let err = client_for(&mock)
        .put::<_, Item>("courses/1", &json!({ "name": "x" }))
        .await
        .unwrap_err();
    assert!(
        matches!(err, CanvasError::Validation(ref b) if b.contains("too long")),
        "{err}"
    );
}

#[tokio::test]
async fn malformed_json_is_a_decode_error() {
    let mock = MockServer::start().await;
    Mock::given(path("/api/v1/courses/1"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
        .mount(&mock)
        .await;

    let err = client_for(&mock)
        .get::<Item>("courses/1", &[])
        .await
        .unwrap_err();
    assert!(matches!(err, CanvasError::Decode(_)), "{err}");
}
