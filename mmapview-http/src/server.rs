use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use mmapstruct::{DynValueType, SchemaCtx, Value};
use mmapview::json_conv::{compute_stats, result_to_json, value_type_to_json};
use serde::Deserialize;
use std::sync::Arc;

/// Upper bound for the `limit` query parameter.
const MAX_PAGE_LIMIT: usize = 1000;

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

/// Run a synchronous computation, converting a panic into `Err`.
///
/// The served files are treated as trusted-but-arbitrary data; a corrupt
/// file must produce a clean HTTP 500, not a dropped connection.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        format!("internal error while processing the file (panic): {detail}")
    })
}

fn error_json(status: StatusCode, msg: String) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
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
) -> Response {
    let start = std::time::Instant::now();
    let q = params.q.as_deref().unwrap_or("/");
    let page = params.page.unwrap_or(0);
    let limit = params.limit.unwrap_or(50);

    if limit == 0 || limit > MAX_PAGE_LIMIT {
        return error_json(
            StatusCode::BAD_REQUEST,
            format!("limit must be in 1..={MAX_PAGE_LIMIT}, got {limit}"),
        );
    }

    let ctx = SchemaCtx::new(&state.data);
    let root = Value::new(0, state.data.len() as u64, state.root_schema);

    let results = match guarded(|| mmapview::query_from(q, &ctx, &root)) {
        Err(p) => return error_json(StatusCode::INTERNAL_SERVER_ERROR, p),
        // DSL errors keep the old 200-with-`{"error": ...}` contract —
        // only panics become HTTP 500.
        Ok(Err(e)) => return Json(serde_json::json!({"error": e})).into_response(),
        Ok(Ok(v)) => v,
    };

    let total = results.len();
    let total_pages = total.div_ceil(limit);

    let slice: Vec<mmapview::EvalResult> = if total == 0 {
        vec![]
    } else {
        let start_idx = page.min(total_pages.saturating_sub(1)) * limit;
        let end_idx = (start_idx + limit).min(total);
        results[start_idx..end_idx].to_vec()
    };

    let data = match guarded(|| {
        slice
            .iter()
            .map(|v| result_to_json(&ctx, v))
            .collect::<Vec<_>>()
    }) {
        Err(p) => return error_json(StatusCode::INTERNAL_SERVER_ERROR, p),
        Ok(d) => d,
    };

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
    .into_response()
}

async fn schema_handler(State(state): State<Arc<AppState>>) -> Response {
    match guarded(|| value_type_to_json(state.root_schema)) {
        Err(p) => error_json(StatusCode::INTERNAL_SERVER_ERROR, p),
        Ok(v) => Json(v).into_response(),
    }
}

async fn stats_handler(State(state): State<Arc<AppState>>) -> Response {
    use serde_json::json;

    let ctx = SchemaCtx::new(&state.data);
    let root = Value::new(0, state.data.len() as u64, state.root_schema);

    let stats = match guarded(|| compute_stats(&ctx, &root)) {
        Err(p) => return error_json(StatusCode::INTERNAL_SERVER_ERROR, p),
        Ok(s) => s,
    };
    let file_size = state.data.len();

    let mut result = serde_json::Map::new();
    result.insert("file_size_bytes".into(), json!(file_size));
    result.insert("file_size_human".into(), json!(format_size(file_size)));
    if let Some(obj) = stats.as_object() {
        result.extend(obj.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    Json(serde_json::Value::Object(result)).into_response()
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
