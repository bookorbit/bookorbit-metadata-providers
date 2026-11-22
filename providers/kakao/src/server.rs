use std::collections::HashMap;
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
use crate::kakao::{Client, UpstreamError};
use crate::mapping::{is_valid_isbn, normalize_isbn, BookMetadata, KakaoDocument};
use crate::{Config, VERSION};

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_ID_LENGTH: usize = 13;

#[derive(Clone)]
struct AppState {
    cfg: Arc<Config>,
    kakao: Arc<Client>,
}

pub fn routes(cfg: Arc<Config>, kakao: Arc<Client>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(AppState { cfg, kakao })
}

async fn info(State(state): State<AppState>) -> Response {
    Json(json!({
        "name": "Kakao",
        "version": VERSION,
        "avatarUrl": "https://developers.kakao.com/favicon.ico",
        "specVersion": "1",
        "authenticationRequired": state.cfg.auth_token.is_some(),
        "key": "kakao",
        "mediaTypes": ["book"],
        "lookup": true,
        "supportedFields": ["title", "description", "cover", "authors", "publisher", "publishedYear", "publishedDate", "isbn"],
        "searchParams": ["isbn", "language", "limit"],
        "idMaxLength": MAX_ID_LENGTH,
        "coverShape": "portrait",
        "color": "#FEE500",
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
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Some(denied) = deny_unauthorized(&state, &headers) {
        return denied;
    }
    let param = |key: &str| q.get(key).map(|v| v.trim()).unwrap_or_default();
    let title_query = param("query").to_string();
    let isbn = param("isbn");
    let isbn_first = !isbn.is_empty();
    let normalized_isbn = normalize_isbn(isbn);
    let isbn_usable = isbn_first && is_valid_isbn(&normalized_isbn);
    let mut query = if isbn_usable {
        normalized_isbn.clone()
    } else {
        title_query.clone()
    };
    let target = if isbn_usable {
        "isbn".to_string()
    } else {
        param("target").to_string()
    };

    // Remove contributor role labels before sending a person search upstream.
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
        "" => None,
        "accuracy" => Some("accuracy"),
        "latest" => Some("latest"),
        _ => return error(StatusCode::BAD_REQUEST, "invalid sort"),
    };
    let page = q
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 50);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_LIMIT, |n| n.clamp(1, MAX_LIMIT));
    let size = limit.clamp(1, 50) as u32;
    let target = (!target.is_empty()).then_some(target.as_str());

    let result = tokio::time::timeout(
        REQUEST_TIMEOUT,
        state.kakao.search(&query, target, sort, page, size),
    )
    .await
    .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())));
    match result {
        Ok(response) => {
            let author_filter = param("author").to_lowercase();
            let mut matches: Vec<BookMetadata> = response
                .documents
                .into_iter()
                .filter_map(KakaoDocument::into_metadata)
                .filter(|book| {
                    !isbn_usable || book.isbn13 == normalized_isbn || book.isbn10 == normalized_isbn
                })
                .collect();
            if !author_filter.is_empty() {
                matches.sort_by_key(|book| {
                    !book
                        .authors
                        .iter()
                        .any(|author| author_names_match(author, &author_filter))
                });
            }
            if matches.is_empty() && isbn_usable && !title_query.is_empty() {
                let fallback = tokio::time::timeout(
                    REQUEST_TIMEOUT,
                    state.kakao.search(&title_query, None, sort, page, size),
                )
                .await
                .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())));
                match fallback {
                    Ok(response) => {
                        matches = response
                            .documents
                            .into_iter()
                            .filter_map(KakaoDocument::into_metadata)
                            .collect();
                    }
                    Err(err) => return upstream_error("search fallback", err),
                }
            }
            matches.truncate(limit);
            Json(SearchAnswer {
                matches,
                partial: false,
            })
            .into_response()
        }
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
    let normalized_id: String = id
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if !is_valid_isbn(&normalized_id) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let result = tokio::time::timeout(
        REQUEST_TIMEOUT,
        state
            .kakao
            .search(&normalized_id, Some("isbn"), None, 1, 20),
    )
    .await
    .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())));
    match result {
        Ok(response) => response
            .documents
            .into_iter()
            .filter_map(KakaoDocument::into_metadata)
            .find(|book| {
                book.provider_id == normalized_id
                    || book.isbn13 == normalized_id
                    || book.isbn10 == normalized_id
            })
            .map(|book| Json(book).into_response())
            .unwrap_or_else(|| error(StatusCode::NOT_FOUND, "Book not found")),
        Err(err) => upstream_error("lookup", err),
    }
}

fn upstream_error(op: &str, err: UpstreamError) -> Response {
    match err {
        UpstreamError::RateLimited {
            retry_after_seconds,
        } => {
            let mut res = error(StatusCode::TOO_MANY_REQUESTS, "rate limited by Kakao");
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            res
        }
        UpstreamError::Unauthorized => {
            error(StatusCode::BAD_GATEWAY, "Kakao API authentication failed")
        }
        UpstreamError::Failed(message) => {
            eprintln!("[provider.{op}] [fail] upstream request failed: {message:?}");
            error(StatusCode::BAD_GATEWAY, "Kakao API unavailable")
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
    use axum::extract::Query;
    use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

    const ISBN_SUCCESS: &str = include_str!("../testdata/response-isbn-success.json");
    const TITLE_SUCCESS: &str = include_str!("../testdata/response-title-success.json");

    async fn mock_kakao(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> String {
        let app = Router::new().route(
            "/",
            get(move |Query(_q): Query<HashMap<String, String>>| {
                let status = status.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let code = status.load(Ordering::SeqCst);
                    if code != 0 {
                        return (
                            StatusCode::from_u16(code).unwrap(),
                            Json(json!({"error":"mock failure"})),
                        )
                            .into_response();
                    }
                    Json(serde_json::from_str::<serde_json::Value>(TITLE_SUCCESS).unwrap())
                        .into_response()
                }
            }),
        );
        serve(app).await
    }

    async fn mock_kakao_fallback_error(fallback_status: u16) -> (String, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/",
            get({
                let calls = calls.clone();
                move |Query(_q): Query<HashMap<String, String>>| {
                    let calls = calls.clone();
                    async move {
                        let call = calls.fetch_add(1, Ordering::SeqCst);
                        if call == 0 {
                            return Json(
                                serde_json::from_str::<serde_json::Value>(TITLE_SUCCESS).unwrap(),
                            )
                            .into_response();
                        }
                        let mut response = (
                            StatusCode::from_u16(fallback_status).unwrap(),
                            Json(json!({"error":"mock fallback failure"})),
                        )
                            .into_response();
                        if fallback_status == 429 {
                            response
                                .headers_mut()
                                .insert(header::RETRY_AFTER, HeaderValue::from_static("37"));
                        }
                        response
                    }
                }
            }),
        );
        (serve(app).await, calls)
    }

    async fn test_provider_with_upstream(upstream: String) -> String {
        let cfg = Arc::new(Config {
            auth_token: None,
            kakao_rest_api_key: "test-key".into(),
            user_agent: "test-agent".into(),
            requests_per_second: 0,
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
        let upstream = mock_kakao(status, calls).await;
        let cfg = Arc::new(Config {
            auth_token: None,
            kakao_rest_api_key: "test-key".into(),
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
        let (status, body) = get_json(&format!(
            "{base}/search?query=%EB%AF%B8%EC%9B%80%EB%B0%9B%EC%9D%84%20%EC%9A%A9%EA%B8%B0&isbn=9788996991342"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["matches"].as_array().unwrap().is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "provider must call the local mock HTTP server"
        );
    }

    #[tokio::test]
    async fn lookup_resolves_isbn_through_local_upstream() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, book) = get_json(&format!("{base}/book/9788996991342")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(book["providerId"], "9788996991342");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "lookup must resolve ISBN through the local mock"
        );
    }

    #[tokio::test]
    async fn invalid_placeholder_isbn_uses_title_search() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) =
            get_json(&format!("{base}/search?isbn=0000000000&query=valid-title")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["matches"].as_array().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn isbn_title_fallback_propagates_rate_limit_and_retry_after() {
        let (upstream, calls) = mock_kakao_fallback_error(429).await;
        let base = test_provider_with_upstream(upstream).await;
        let response = reqwest::Client::new()
            .get(format!(
                "{base}/search?isbn=9780132350884&query=valid-title"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers().get(header::RETRY_AFTER).unwrap(), "37");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn isbn_title_fallback_maps_upstream_failure_to_bad_gateway() {
        let (upstream, calls) = mock_kakao_fallback_error(503).await;
        let base = test_provider_with_upstream(upstream).await;
        let response = reqwest::Client::new()
            .get(format!(
                "{base}/search?isbn=9780132350884&query=valid-title"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
        assert_eq!(body["key"], "kakao");
        assert_eq!(body["mediaTypes"], json!(["book"]));
        assert!(!body["supportedFields"]
            .as_array()
            .unwrap()
            .contains(&json!("language")));
        assert_eq!(body["rateLimit"]["minIntervalMs"], 500);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn captured_fixtures_remain_valid() {
        let isbn: crate::mapping::KakaoResponse = serde_json::from_str(ISBN_SUCCESS).unwrap();
        let title: crate::mapping::KakaoResponse = serde_json::from_str(TITLE_SUCCESS).unwrap();
        assert!(!isbn.documents.is_empty());
        assert!(!title.documents.is_empty());
    }
}
