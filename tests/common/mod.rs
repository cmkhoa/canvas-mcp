//! Test harness: a wiremock "Canvas" plus the real server, driven over an in-memory MCP
//! connection so tests exercise argument parsing and schemas, not just Rust methods.
#![allow(dead_code)]

use std::time::Duration;

use canvas_mcp::{CanvasClient, CanvasServer, Config, canvas::RetryPolicy};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use serde_json::Value;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

pub struct Harness {
    pub mock: MockServer,
    client: RunningService<RoleClient, ()>,
}

impl Harness {
    pub async fn new() -> Self {
        let mock = MockServer::start().await;
        let config = Config::new(&format!("{}/api/v1", mock.uri()), "test-token").unwrap();
        let client = CanvasClient::new(&config)
            .unwrap()
            .with_retry_policy(RetryPolicy {
                max_retries: 1,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(5),
            });
        let server = CanvasServer::new(client);

        let (server_io, client_io) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_io).await {
                let _ = running.waiting().await;
            }
        });
        let client = ().serve(client_io).await.expect("MCP handshake");
        Self { mock, client }
    }

    /// Calls a tool and returns the raw result (for non-text content).
    pub async fn call_raw(&self, tool: &str, args: Value) -> CallToolResult {
        let mut params = CallToolRequestParams::new(tool.to_owned());
        params.arguments = args.as_object().cloned();
        self.client.call_tool(params).await.expect("tool call")
    }

    /// Calls a tool; returns (is_error, text).
    pub async fn call(&self, tool: &str, args: Value) -> (bool, String) {
        let result = self.call_raw(tool, args).await;
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        (result.is_error.unwrap_or(false), text)
    }

    /// Calls a tool that must succeed and returns its text.
    pub async fn ok(&self, tool: &str, args: Value) -> String {
        let (is_error, text) = self.call(tool, args).await;
        assert!(!is_error, "{tool} failed: {text}");
        text
    }

    pub async fn tool_names(&self) -> Vec<String> {
        let tools = self.client.list_all_tools().await.expect("list tools");
        tools.into_iter().map(|t| t.name.to_string()).collect()
    }

    /// Serves `body` for `GET {api_path}` (path relative to /api/v1/).
    pub async fn get(&self, api_path: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/{api_path}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.mock)
            .await;
    }

    /// Serves the active-course list the course resolver reads.
    pub async fn active_courses(&self, body: Value) {
        Mock::given(method("GET"))
            .and(path("/api/v1/courses"))
            .and(query_param("enrollment_state", "active"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.mock)
            .await;
    }
}

/// Two typical courses for resolver lookups.
pub fn two_courses() -> Value {
    serde_json::json!([
        {
            "id": 101, "course_code": "CS 101", "name": "Intro to Computer Science",
            "term": { "name": "Fall 2026" },
            "enrollments": [{ "type": "student", "role": "StudentEnrollment" }]
        },
        {
            "id": 210, "course_code": "CHEM 210", "name": "Organic Chemistry I",
            "term": { "name": "Fall 2026" },
            "enrollments": [{ "type": "student", "role": "StudentEnrollment" }]
        }
    ])
}
