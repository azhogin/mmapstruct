use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use mmapstruct::{DynValueType, SchemaCtx, Value};
use mmapview::json_conv::{compute_stats, result_to_json, value_type_to_json};
use serde::Deserialize;
use std::sync::Arc;

/// Shared application state.
pub struct AppState {
    pub data: Arc<[u8]>,
    pub root_schema: &'static DynValueType,
}

#[derive(Deserialize)]
pub struct QueryParams {
    pub q: Option<String>,
    pub page: Option<usize>,
    pub limit: Option<usize>,
}

/// Build the axum router with all handlers and static file routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index_handler))
        .route("/static/viewer.js", get(js_handler))
        .route("/static/viewer.css", get(css_handler))
        .route("/api/query", get(query_handler))
        .route("/api/schema", get(schema_handler))
        .route("/api/stats", get(stats_handler))
        .with_state(state)
}

const INDEX_HTML: &str = include_str!("../static/index.html");
const VIEWER_JS: &str = include_str!("../static/viewer.js");
const VIEWER_CSS: &str = include_str!("../static/viewer.css");

async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn js_handler() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        Html(VIEWER_JS),
    )
}

async fn css_handler() -> impl IntoResponse {
    ([(axum::http::header::CONTENT_TYPE, "text/css")], VIEWER_CSS)
}

async fn query_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<QueryParams>,
) -> Json<serde_json::Value> {
    let start = std::time::Instant::now();
    let q = params.q.as_deref().unwrap_or("/");
    let page = params.page.unwrap_or(0);
    let limit = params.limit.unwrap_or(50);

    let ctx = SchemaCtx::new(&state.data);
    let root = Value::new(0, state.data.len() as u64, state.root_schema);

    let results = match mmapview::query_from(q, &ctx, &root) {
        Ok(v) => v,
        Err(e) => return Json(serde_json::json!({"error": e})),
    };

    let total = results.len();
    let total_pages = if limit > 0 { total.div_ceil(limit) } else { 1 };

    let slice: Vec<mmapview::EvalResult> = if total == 0 {
        vec![]
    } else {
        let start_idx = page.min(total_pages.saturating_sub(1)) * limit;
        let end_idx = (start_idx + limit).min(total);
        results[start_idx..end_idx].to_vec()
    };

    let data: Vec<serde_json::Value> = slice.iter().map(|v| result_to_json(&ctx, v)).collect();
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    Json(serde_json::json!({
        "data": data,
        "pagination": {
            "page": page,
            "limit": limit,
            "total": total,
            "pages": total_pages,
            "has_next": page + 1 < total_pages,
            "has_prev": page > 0,
        },
        "query": q,
        "elapsed_ms": elapsed,
    }))
}

async fn schema_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(value_type_to_json(state.root_schema))
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    use serde_json::json;

    let ctx = SchemaCtx::new(&state.data);
    let root = Value::new(0, state.data.len() as u64, state.root_schema);

    let stats = compute_stats(&ctx, &root);
    let file_size = state.data.len();

    let mut result = serde_json::Map::new();
    result.insert("file_size_bytes".into(), json!(file_size));
    result.insert("file_size_human".into(), json!(format_size(file_size)));
    if let Some(obj) = stats.as_object() {
        result.extend(obj.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    Json(serde_json::Value::Object(result))
}

fn format_size(bytes: usize) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}
