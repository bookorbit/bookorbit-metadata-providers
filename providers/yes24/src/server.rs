use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::author_normalizer::{author_names_match, normalize_author_query};
use crate::mapping::{BookMetadata, Yes24Item};
use crate::yes24::{Client, UpstreamError};
use crate::{Config, VERSION};

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_ID_LENGTH: usize = 70;

#[derive(Clone)]
struct AppState {
    cfg: Arc<Config>,
    yes24: Arc<Client>,
}

pub fn routes(cfg: Arc<Config>, yes24: Arc<Client>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(AppState { cfg, yes24 })
}

async fn info(State(state): State<AppState>) -> Response {
    Json(json!({
        "name": "YES24",
        "version": VERSION,
        "avatarUrl": "https://www.yes24.com/favicon.ico",
        "specVersion": "1",
        "authenticationRequired": state.cfg.auth_token.is_some(),
        "key": "yes24",
        "mediaTypes": ["book"],
        "lookup": true,
        "supportedFields": ["title", "subtitle", "description", "authors", "publisher", "isbn", "publishedYear", "language", "pageCount", "seriesName", "genres", "communityRating"],
        "searchParams": ["limit", "author", "target", "sort", "page", "isbn"],
        "idMaxLength": MAX_ID_LENGTH,
        "coverShape": "portrait",
        "color": "#E60021",
        "rateLimit": { "maxConcurrency": state.cfg.max_inflight_requests, "minIntervalMs": if state.cfg.requests_per_second == 0 { 0 } else { (1000.0 / state.cfg.requests_per_second as f64).ceil() as u64 } }
    })).into_response()
}

#[derive(Serialize)]
struct SearchAnswer {
    matches: Vec<BookMetadata>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(denied) = deny_unauthorized(&state, &headers) {
        return denied;
    }
    let param = |key: &str| q.get(key).map(|v| v.trim()).unwrap_or_default();
    let mut query = param("query").to_string();
    let isbn = param("isbn");
    let mut target = param("target").to_string();
    if query.is_empty() && !isbn.is_empty() {
        query = isbn.to_string();
        target = "isbn".to_string();
    }

    // 인명 검색인 경우 API에 전달하기 전에 역할 표현 제거, 첫번째 이름만 남김
    if target == "person" {
        query = normalize_author_query(&query);
    }

    if query.is_empty() {
        return error(StatusCode::BAD_REQUEST, "query is required");
    }
    if !target.is_empty() && !["title", "isbn", "publisher", "person"].contains(&target.as_str()) {
        return error(StatusCode::BAD_REQUEST, "invalid target");
    }
    let sort = match param("sort") {
        "" | "accuracy" => "DEFAULT",
        "latest" => "RECENT",
        _ => return error(StatusCode::BAD_REQUEST, "invalid sort"),
    };
    let page = q
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 10_000);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_LIMIT, |n| n.clamp(1, MAX_LIMIT));
    let category = match param("category") {
        "" => "BOOK",
        "ALL" | "BOOK" | "FOREIGN" | "EBOOK" | "MUSIC" | "DVD" => param("category"),
        _ => return error(StatusCode::BAD_REQUEST, "invalid category"),
    };

    // YES24 has a dedicated ISBN13 lookup. ISBN10 and non-ISBN filters fall back to itemList.
    let isbn13 = query
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-')
        .then(|| {
            query
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|v| v.len() == 13);
    let fetched: Result<Vec<Yes24Item>, UpstreamError> = if target == "isbn" && isbn13.is_some() {
        let isbn = isbn13.as_deref().unwrap_or_default();
        tokio::time::timeout(REQUEST_TIMEOUT, state.yes24.detail_by_isbn13(isbn))
            .await
            .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())))
            .map(|item| item.into_iter().collect())
    } else {
        let page_size =
            if !param("author").is_empty() || target == "person" || target == "publisher" {
                100
            } else {
                limit.max(20) as u32
            };
        tokio::time::timeout(
            REQUEST_TIMEOUT,
            state.yes24.search(&query, category, sort, page, page_size),
        )
        .await
        .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())))
    };

    match fetched {
        Ok(items) => {
            let author_filter = param("author").to_lowercase();
            let publisher_filter = if target == "publisher" {
                query.to_lowercase()
            } else {
                String::new()
            };
            let person_filter = if target == "person" {
                query.to_lowercase()
            } else {
                String::new()
            };
            let mut matches: Vec<BookMetadata> = items
                .into_iter()
                .filter(|item| {
                    (author_filter.is_empty() || author_names_match(&item.author, &author_filter))
                        && (publisher_filter.is_empty()
                            || item.publisher.to_lowercase().contains(&publisher_filter))
                        && (person_filter.is_empty()
                            || author_names_match(&item.author, &person_filter))
                        && (target != "isbn" || {
                            let normalized_isbn: String = query
                                .chars()
                                .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
                                .collect();
                            item.isbn10.eq_ignore_ascii_case(&normalized_isbn)
                                || item.isbn13 == normalized_isbn
                        })
                })
                .map(Yes24Item::into_metadata)
                .collect();
            matches.truncate(limit);
            Json(SearchAnswer {
                matches,
                partial: false,
            })
            .into_response()
        }
        Err(UpstreamError::NotFound) => Json(SearchAnswer {
            matches: Vec::new(),
            partial: false,
        })
        .into_response(),
        Err(err) => upstream_error("search", err),
    }
}

async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = deny_unauthorized(&state, &headers) {
        return denied;
    }
    // New IDs are the numeric YES24 itemId only. Accept the old prefixed form as a
    // compatibility path for existing links/bookmarks.
    let raw_id = id.strip_prefix("YES24-").unwrap_or(&id);
    if raw_id.len() > MAX_ID_LENGTH
        || raw_id.is_empty()
        || !raw_id.bytes().all(|b| b.is_ascii_digit())
    {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let Ok(item_id) = raw_id.parse::<u64>() else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    if item_id == 0 {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let result = tokio::time::timeout(REQUEST_TIMEOUT, state.yes24.detail_by_item_id(item_id))
        .await
        .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())));
    match result {
        Ok(Some(item)) => Json(item.into_metadata()).into_response(),
        Ok(None) | Err(UpstreamError::NotFound) => error(StatusCode::NOT_FOUND, "Book not found"),
        Err(err) => upstream_error("lookup", err),
    }
}

fn upstream_error(op: &str, err: UpstreamError) -> Response {
    match err {
        UpstreamError::RateLimited {
            retry_after_seconds,
        } => {
            let mut res = error(StatusCode::TOO_MANY_REQUESTS, "rate limited by YES24");
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            res
        }
        UpstreamError::Unauthorized => {
            error(StatusCode::BAD_GATEWAY, "YES24 API authentication failed")
        }
        UpstreamError::NotFound => error(StatusCode::NOT_FOUND, "Not found"),
        UpstreamError::Failed(message) => {
            eprintln!("[provider.{op}] [fail] upstream request failed: {message:?}");
            error(StatusCode::BAD_GATEWAY, "YES24 API unavailable")
        }
    }
}

fn deny_unauthorized(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let token = state.cfg.auth_token.as_ref()?;
    let given = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    (!constant_time_eq(given, token.as_bytes()))
        .then(|| error(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

    const SEARCH_FIXTURE: &str = include_str!("../testdata/response-search.json");
    const DETAIL_FIXTURE: &str = include_str!("../testdata/response-detail-isbn13.json");

    async fn mock_yes24(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> String {
        let app = Router::new()
            .route(
                "/itemList",
                get({
                    let status = status.clone();
                    let calls = calls.clone();
                    move || {
                        let status = status.clone();
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            let code = status.load(Ordering::SeqCst);
                            if code != 0 {
                                return (
                                    StatusCode::from_u16(code).unwrap(),
                                    Json(json!({"success":false,"message":"mock failure"})),
                                )
                                    .into_response();
                            }
                            Json(serde_json::from_str::<serde_json::Value>(SEARCH_FIXTURE).unwrap())
                                .into_response()
                        }
                    }
                }),
            )
            .route(
                "/itemDetail",
                get({
                    let status = status.clone();
                    let calls = calls.clone();
                    move || {
                        let status = status.clone();
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            let code = status.load(Ordering::SeqCst);
                            if code != 0 {
                                return (
                                    StatusCode::from_u16(code).unwrap(),
                                    Json(json!({"success":false,"message":"mock failure"})),
                                )
                                    .into_response();
                            }
                            Json(serde_json::from_str::<serde_json::Value>(DETAIL_FIXTURE).unwrap())
                                .into_response()
                        }
                    }
                }),
            );
        serve(app).await
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn get_json(url: &str) -> (StatusCode, serde_json::Value) {
        let response = reqwest::Client::new().get(url).send().await.unwrap();
        let status = response.status();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    async fn test_provider(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> String {
        let upstream = mock_yes24(status, calls).await;
        let cfg = Arc::new(Config {
            auth_token: None,
            yes24_api_key: "test-key".into(),
            user_agent: "test-agent".into(),
            requests_per_second: 2,
            max_inflight_requests: 2,
        });
        let client = Arc::new(Client::with_base_url(
            "test-key",
            "test-agent",
            &upstream,
            0,
            2,
        ));
        serve(routes(cfg, client)).await
    }

    #[tokio::test]
    async fn search_uses_local_upstream_and_maps_response() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!("{base}/search?query=%EB%AF%B8%EC%9B%80")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["matches"].as_array().unwrap().is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "provider must call the local mock HTTP server"
        );
    }

    #[tokio::test]
    async fn isbn_detail_uses_local_upstream() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!("{base}/search?isbn=9791168340770")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["matches"].as_array().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn lookup_fetches_detail_from_local_upstream() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!("{base}/book/116599423")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["providerId"], "116599423");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "lookup must call the local mock HTTP server"
        );
    }

    #[tokio::test]
    async fn upstream_error_is_mapped_to_bad_gateway() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(500)), calls.clone()).await;
        let (status, _) = get_json(&format!("{base}/search?query=test")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_info_exposes_contract_and_rate_limit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!("{base}/provider/info")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["key"], "yes24");
        assert_eq!(body["rateLimit"]["minIntervalMs"], 500);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
