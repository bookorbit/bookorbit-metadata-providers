use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::audiosilo::{AudioSilo, Query as SearchQuery, UpstreamError};
use crate::VERSION;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(9);

pub fn routes(source: Arc<AudioSilo>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(source)
}

async fn info() -> Json<serde_json::Value> {
    Json(json!({
        "name": "AudioSilo",
        "version": VERSION,
        "authenticationRequired": false,
        "key": "audiosilo",
        "mediaTypes": ["audiobook"],
        "lookup": true,
        "supportedFields": [
            "title", "subtitle", "description", "cover", "authors", "narrators", "publisher",
            "publishedYear", "language", "duration", "genres", "seriesName", "seriesIndex",
            "isbn", "isbn13", "isbn10"
        ],
        "searchParams": ["isbn", "limit"],
        "bookUrlTemplate": "https://meta.audiosilo.app/works/{id}",
        "rateLimit": { "maxConcurrency": 1, "minIntervalMs": 0 }
    }))
}

async fn search(
    State(source): State<Arc<AudioSilo>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let param = |name: &str| {
        params
            .get(name)
            .map(|value| value.trim())
            .unwrap_or_default()
    };
    let query = SearchQuery {
        title: param("query").to_string(),
        author: param("author").to_string(),
        isbn: param("isbn").to_string(),
        media_type: param("mediaType").to_string(),
        limit: param("limit")
            .parse::<usize>()
            .map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, MAX_LIMIT)),
    };
    if query.title.is_empty() && query.isbn.is_empty() {
        return error(StatusCode::BAD_REQUEST, "query is required");
    }
    match tokio::time::timeout(REQUEST_TIMEOUT, source.search(&query)).await {
        Ok(Ok(answer)) => Json(answer).into_response(),
        Ok(Err(err)) => upstream_error("search", err),
        Err(_) => upstream_error("search", UpstreamError::Failed("request timed out".into())),
    }
}

async fn lookup(State(source): State<Arc<AudioSilo>>, Path(id): Path<String>) -> Response {
    match tokio::time::timeout(REQUEST_TIMEOUT, source.lookup(&id)).await {
        Ok(Ok(Some(book))) => Json(book).into_response(),
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, "Not found"),
        Ok(Err(err)) => upstream_error("lookup", err),
        Err(_) => upstream_error("lookup", UpstreamError::Failed("request timed out".into())),
    }
}

fn upstream_error(operation: &str, upstream: UpstreamError) -> Response {
    match upstream {
        UpstreamError::NotFound => error(StatusCode::NOT_FOUND, "Not found"),
        UpstreamError::RateLimited {
            retry_after_seconds,
        } => {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "AudioSilo is temporarily unavailable",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            response
        }
        UpstreamError::Failed(message) => {
            eprintln!("[provider.{operation}] [fail] error={message:?} - AudioSilo request failed");
            error(StatusCode::BAD_GATEWAY, "AudioSilo is unavailable")
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}
