mod common;

use common::{Harness, two_courses};
use rmcp::model::{ContentBlock, ResourceContents};
use serde_json::json;
use wiremock::{
    Mock, Request, ResponseTemplate,
    matchers::{path, query_param},
};

/// Serves file metadata whose signed link points back at the mock, and the bytes at that
/// link only when no token is sent.
async fn serve_file(h: &Harness, id: u64, name: &str, mime: &str, bytes: &[u8]) {
    h.get(
        &format!("courses/101/files/{id}"),
        json!({ "id": id, "display_name": name, "filename": name, "content-type": mime,
                "size": bytes.len(), "url": format!("{}/files/{id}/download?verifier=abc", h.mock.uri()) }),
    )
    .await;
    Mock::given(path(format!("/files/{id}/download")))
        .and(query_param("verifier", "abc"))
        .and(|req: &Request| !req.headers.contains_key("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
        .expect(1..)
        .mount(&h.mock)
        .await;
}

#[tokio::test]
async fn list_files_validates_search_and_formats_sizes() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    Mock::given(path("/api/v1/courses/101/files"))
        .and(query_param("search_term", "lab"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 900, "display_name": "lab3.pdf", "content-type": "application/pdf", "size": 2_516_582 }
        ])))
        .mount(&h.mock)
        .await;

    let out = h
        .ok(
            "list_course_files",
            json!({ "course_identifier": "CS 101", "search_term": "lab" }),
        )
        .await;
    assert!(
        out.contains("- [file 900] \"lab3.pdf\" · 2.4 MB · application/pdf"),
        "{out}"
    );
    let (is_error, text) = h
        .call(
            "list_course_files",
            json!({ "course_identifier": "CS 101", "search_term": "l" }),
        )
        .await;
    assert!(is_error && text.contains("at least 2 characters"), "{text}");
}

#[tokio::test]
async fn read_text_file_is_fenced_and_fetched_without_token() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    serve_file(
        &h,
        901,
        "notes.md",
        "text/markdown",
        b"# Week 3\nRead ch. 4",
    )
    .await;

    let out = h
        .ok(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 901 }),
        )
        .await;
    assert!(
        out.contains("\"notes.md\" (text/markdown, 19 B) from CS 101"),
        "{out}"
    );
    assert!(
        out.contains("<untrusted source=\"file: notes.md\">\n# Week 3\nRead ch. 4"),
        "{out}"
    );
}

#[tokio::test]
async fn read_pdf_and_image_return_embedded_content() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    serve_file(&h, 902, "slides.pdf", "application/pdf", b"%PDF-1.7 fake").await;
    serve_file(&h, 903, "diagram.png", "image/png", b"\x89PNG fake").await;

    let pdf = h
        .call_raw(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 902 }),
        )
        .await;
    match &pdf.content[1] {
        ContentBlock::Resource(r) => match &r.resource {
            ResourceContents::BlobResourceContents {
                mime_type, blob, ..
            } => {
                assert_eq!(mime_type.as_deref(), Some("application/pdf"));
                assert_eq!(blob, "JVBERi0xLjcgZmFrZQ==");
            }
            other => panic!("expected blob, got {other:?}"),
        },
        other => panic!("expected resource, got {other:?}"),
    }
    let png = h
        .call_raw(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 903 }),
        )
        .await;
    assert!(matches!(&png.content[1], ContentBlock::Image(img) if img.mime_type == "image/png"));
}

#[tokio::test]
async fn read_refuses_oversized_and_locked_files() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get("courses/101/files/904", json!({ "id": 904, "display_name": "video.mp4", "size": 80_000_000u64, "url": "https://x/f" })).await;
    h.get(
        "courses/101/files/905",
        json!({ "id": 905, "display_name": "answers.pdf", "locked_for_user": true,
                                          "lock_explanation": "Unlocks after the exam." }),
    )
    .await;

    let (is_error, text) = h
        .call(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 904 }),
        )
        .await;
    assert!(is_error && text.contains("over the 25 MB limit"), "{text}");
    let (is_error, text) = h
        .call(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 905 }),
        )
        .await;
    assert!(
        is_error && text.contains("is locked for you \"Unlocks after the exam.\""),
        "{text}"
    );
    let (is_error, text) = h
        .call(
            "read_course_file",
            json!({ "course_identifier": "CS 101", "file_id": 904, "max_size_mb": 99 }),
        )
        .await;
    assert!(is_error && text.contains("at most 50"), "{text}");
}

#[tokio::test]
async fn download_saves_once_and_refuses_to_overwrite() {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    serve_file(&h, 906, "../sneaky.txt", "text/plain", b"hello").await;
    let dir = tempfile::tempdir().unwrap();
    let args =
        json!({ "course_identifier": "CS 101", "file_id": 906, "save_directory": dir.path() });

    let out = h.ok("download_course_file", args.clone()).await;
    let saved = dir.path().canonicalize().unwrap().join("_sneaky.txt");
    assert!(out.contains(&saved.display().to_string()), "{out}");
    assert_eq!(std::fs::read(&saved).unwrap(), b"hello");

    let (is_error, text) = h.call("download_course_file", args).await;
    assert!(
        is_error && text.contains("already exists; nothing was overwritten"),
        "{text}"
    );
}
