use mmapstruct::{write_to_buf_raw, GetSchema, MmapStruct};
use std::sync::Arc;

#[derive(MmapStruct, Debug)]
struct TestFile {
    id: u32,
    name: String,
    tags: Vec<u32>,
    #[mmap(byref)]
    comment: String,
}

fn make_test_file() -> Vec<u8> {
    write_to_buf_raw(&TestFile {
        id: 42,
        name: "hello".into(),
        tags: vec![1, 2, 3],
        comment: "a comment".into(),
    })
}

async fn spawn_server(data: Vec<u8>) -> (tokio::task::JoinHandle<()>, String) {
    let state = Arc::new(mmapview_http::AppState {
        data: Arc::from(data),
        root_schema: TestFile::root_value_type(),
    });
    let app = mmapview_http::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Brief wait for server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (handle, format!("http://{}", addr))
}

#[tokio::test]
async fn test_query_root_fields() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    // Navigate to /id to verify struct reading works end-to-end
    let resp = reqwest::get(format!("{}/api/query?q=/id", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(json["data"], serde_json::json!([42]));
    assert!(json["elapsed_ms"].as_f64().unwrap() >= 0.0);
    assert!(json["pagination"]["total"].as_u64().unwrap() >= 1);

    // Navigate to /name to verify string reading
    let resp = reqwest::get(format!("{}/api/query?q=/name", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!(["hello"]));

    // Navigate to /tags to verify vec of pod reading
    let resp = reqwest::get(format!("{}/api/query?q=/tags", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([[1, 2, 3]]));
}

#[tokio::test]
async fn test_query_by_field() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    let resp = reqwest::get(format!("{}/api/query?q=/id", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([42]));
}

#[tokio::test]
async fn test_query_tags() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    let resp = reqwest::get(format!("{}/api/query?q=/tags", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([[1, 2, 3]]));
}

#[tokio::test]
async fn test_query_pagination() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    // /tags returns [[1,2,3]] — one result, so pagination shows total=1
    let resp = reqwest::get(format!("{}/api/query?q=/tags&page=0&limit=1", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();

    let data_arr = json["data"].as_array().unwrap();
    assert_eq!(data_arr.len(), 1);
    assert_eq!(json["pagination"]["total"], 1);
    assert_eq!(json["pagination"]["page"], 0);
    assert_eq!(json["pagination"]["has_prev"], false);
    assert_eq!(json["pagination"]["has_next"], false);
}

#[tokio::test]
async fn test_schema_endpoint() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    let resp = reqwest::get(format!("{}/api/schema", base)).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(json["name"], "TestFile");
    let fields = json["fields"].as_array().unwrap();
    assert!(fields.iter().any(|f| f["name"] == "id"));
    assert!(fields.iter().any(|f| f["name"] == "name"));
    assert!(fields.iter().any(|f| f["name"] == "tags"));
}

#[tokio::test]
async fn test_stats_endpoint() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    let resp = reqwest::get(format!("{}/api/stats", base)).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();

    assert!(json["file_size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(json["root_type"], "TestFile");
    assert_eq!(json["top_level"]["id"]["type"], "u32");
    assert!(json["top_level"]["id"].get("present").is_none());
    assert_eq!(json["top_level"]["tags"]["type"], "array");
    assert_eq!(json["top_level"]["tags"]["count"], 3);
}

#[tokio::test]
async fn test_query_error_handling() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    // Malformed query
    let resp = reqwest::get(format!("{}/api/query?q=///", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json["error"].as_str().is_some());
}

#[tokio::test]
async fn test_limit_validation_returns_400() {
    let data = make_test_file();
    let (_server, base) = spawn_server(data).await;

    // limit = 0 → 400
    let resp = reqwest::get(format!("{}/api/query?q=/id&limit=0", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json["error"].as_str().unwrap().contains("limit"));

    // limit > MAX_PAGE_LIMIT (1000) → 400
    let resp = reqwest::get(format!("{}/api/query?q=/id&limit=1001", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Boundary: limit = 1000 → OK
    let resp = reqwest::get(format!("{}/api/query?q=/id&limit=1000", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Corrupt the data so rendering "/" slices out of bounds → the handler
/// must return a clean HTTP 500 (and the server must survive).
#[tokio::test]
async fn test_corrupt_data_panic_returns_500() {
    let mut data = make_test_file();
    // TestFile layout: fixed section = id (4 bytes) + ByRefRange placeholder
    // for `comment` (8 bytes) at [4..12]; offset table for name/tags/sentinel
    // at [12..36].  Overwrite the ByRefRange with begin = 0xFFFFFFFF → the
    // byref resolution of `comment` produces an out-of-bounds Value →
    // `read_str` panics while rendering "/".
    data[4..12].copy_from_slice(&0x0000_FFFF_FFFF_FFFFu64.to_le_bytes());
    let (_server, base) = spawn_server(data).await;

    let resp = reqwest::get(format!("{}/api/query?q=/", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert!(json["error"].as_str().unwrap().contains("panic"));

    // The server survived the panic: a benign query still answers 200.
    let resp = reqwest::get(format!("{}/api/query?q=/id&limit=10", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([42]));
}
