mod common;

use common::{Harness, two_courses};
use serde_json::json;
use wiremock::{
    Mock, ResponseTemplate,
    matchers::{path, query_param},
};

async fn harness() -> Harness {
    let h = Harness::new().await;
    h.active_courses(two_courses()).await;
    h.get(
        "users/self/groups",
        json!([
            { "id": 7, "name": "Team Rocket", "course_id": 101, "group_category_id": 3, "members_count": 2 },
            { "id": 8, "name": "Chem Pod", "course_id": 210 },
            { "id": 9, "name": "Chess Club", "context_type": "Account" }
        ]),
    )
    .await;
    h
}

#[tokio::test]
async fn my_groups_list_and_filter_by_course() {
    let h = harness().await;
    let all = h.ok("list_my_groups", json!({})).await;
    assert!(
        all.contains("- [group 7] \"Team Rocket\" · CS 101 (course 101) · set 3 · 2 members"),
        "{all}"
    );
    assert!(
        all.contains("- [group 9] \"Chess Club\" · Account group"),
        "{all}"
    );

    let chem = h
        .ok("list_my_groups", json!({ "course_identifier": "CHEM 210" }))
        .await;
    assert!(
        chem.contains("Chem Pod") && !chem.contains("Team Rocket"),
        "{chem}"
    );
}

#[tokio::test]
async fn members_are_only_readable_for_your_groups() {
    let h = harness().await;
    h.get(
        "groups/7/users",
        json!([{ "id": 1, "name": "Ana", "email": "ana@school.edu" }, { "id": 2, "name": "Sam" }]),
    )
    .await;

    let out = h.ok("get_group_members", json!({ "group_id": 7 })).await;
    assert!(
        out.contains("Members of \"Team Rocket\" (2):\n- \"Ana\" (user id 1)"),
        "{out}"
    );
    assert!(!out.contains("ana@school.edu"), "{out}");

    let (is_error, text) = h.call("get_group_members", json!({ "group_id": 99 })).await;
    assert!(
        is_error && text.contains("not a member of group 99"),
        "{text}"
    );
}

#[tokio::test]
async fn group_files_pass_sort_and_search() {
    let h = harness().await;
    Mock::given(path("/api/v1/groups/7/files"))
        .and(query_param("sort", "updated_at"))
        .and(query_param("search_term", "draft"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 31, "display_name": "draft.docx", "size": 20480 }
        ])))
        .mount(&h.mock)
        .await;
    let out = h
        .ok(
            "list_group_files",
            json!({ "group_id": 7, "sort": "updated_at", "search_term": "draft" }),
        )
        .await;
    assert!(out.contains("- [file 31] \"draft.docx\" · 20 KB"), "{out}");
}
