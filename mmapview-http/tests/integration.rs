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

// ---------------------------------------------------------------------------
// List<T> (linked-node container) over the compile-time schema interpreter
// ---------------------------------------------------------------------------

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
#[repr(C)]
struct Probe {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
struct ListFile {
    id: u32,
    tags: mmapstruct::List<String>,
    events: mmapstruct::List<Probe>,
}

fn make_list_file() -> Vec<u8> {
    write_to_buf_raw(&ListFile {
        id: 1,
        tags: mmapstruct::List::from_vec(vec!["alpha".into(), "beta".into()]),
        events: mmapstruct::List::from_vec(vec![
            Probe { id: 10, score: 1.5 },
            Probe {
                id: 20,
                score: -2.5,
            },
            Probe {
                id: 30,
                score: 3.25,
            },
        ]),
    })
}

#[tokio::test]
async fn test_query_list_fields() {
    let data = make_list_file();
    let (_server, base) = spawn_server_with::<ListFile>(data).await;

    // List of strings.
    let resp = reqwest::get(format!("{}/api/query?q=/tags", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([["alpha", "beta"]]));

    // List of pod structs: full array and index access.
    let resp = reqwest::get(format!("{}/api/query?q=/events", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"][0].as_array().unwrap().len(), 3);
    assert_eq!(json["data"][0][1]["score"], -2.5);

    let resp = reqwest::get(format!("{}/api/query?q=/events[1]", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([{"id": 20, "score": -2.5}]));

    // Filter and aggregate over a List.
    let resp = reqwest::get(format!("{}/api/query?q=/events | select(.score > 0)", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    let arr = json["data"].as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], 10);
    assert_eq!(arr[1]["id"], 30);

    let resp = reqwest::get(format!("{}/api/query?q=/events | avg(.score)", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(json["data"], serde_json::json!([0.75]));
}

#[tokio::test]
async fn test_list_schema_endpoint() {
    let data = make_list_file();
    let (_server, base) = spawn_server_with::<ListFile>(data).await;

    let resp = reqwest::get(format!("{}/api/schema", base)).await.unwrap();
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("list"),
        "schema should label List layout: {text}"
    );
}

async fn spawn_server_with<T: GetSchema>(data: Vec<u8>) -> (tokio::task::JoinHandle<()>, String) {
    let state = Arc::new(mmapview_http::AppState {
        data: Arc::from(data),
        root_schema: T::root_value_type(),
    });
    let app = mmapview_http::build_router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    (handle, format!("http://{}", addr))
}

// ---------------------------------------------------------------------------
// Option fields through the compile-time SchemaCtx interpreter: Some reads
// as the inner value, None renders as null, filters match Some only.
// ---------------------------------------------------------------------------

#[derive(MmapStruct, Debug)]
struct OptHttpFile {
    id: u32,
    maybe: Option<String>,
    n: Option<u64>,
    items: Vec<Option<HttpRec>>,
}

#[derive(MmapStruct, Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct HttpRec {
    test_field: u32,
}

fn make_opt_http_file() -> Vec<u8> {
    write_to_buf_raw(&OptHttpFile {
        id: 9,
        maybe: Some("present".into()),
        n: None,
        items: vec![
            Some(HttpRec { test_field: 5 }),
            None,
            Some(HttpRec { test_field: 0 }),
            Some(HttpRec { test_field: 7 }),
        ],
    })
}

#[tokio::test]
async fn test_query_option_fields() {
    let data = make_opt_http_file();
    let (_server, base) = spawn_server_with::<OptHttpFile>(data).await;

    // Some<String> reads as the string.
    let resp = reqwest::get(format!("{}/api/query?q=/maybe", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        json["data"],
        serde_json::json!(["present"]),
        "Some field: {json}"
    );

    // None renders as null.
    let resp = reqwest::get(format!("{}/api/query?q=/n", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        json["data"],
        serde_json::json!([null]),
        "None field: {json}"
    );

    // Filters over Option fields: Some matches, None does not.
    let resp = reqwest::get(format!(
        "{}/api/query?q=/items[?test_field>0]&limit=10",
        base
    ))
    .await
    .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    let arr = json["data"].as_array().unwrap();
    assert_eq!(arr.len(), 2, "None and test_field=0 must not match: {json}");
    assert_eq!(arr[0]["test_field"], 5);
    assert_eq!(arr[1]["test_field"], 7);

    // Vec<Option<Rec>> full read: None elements render as null.
    let resp = reqwest::get(format!("{}/api/query?q=/items&limit=10", base))
        .await
        .unwrap();
    let json: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        json["data"],
        serde_json::json!([[
            {"test_field": 5},
            null,
            {"test_field": 0},
            {"test_field": 7}
        ]]),
        "full vec of options: {json}"
    );
}
