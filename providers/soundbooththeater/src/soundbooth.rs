use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::header::RETRY_AFTER;
use reqwest::{RequestBuilder, StatusCode, Url};
use scraper::Html;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};

use crate::mapping::{map_product, parse_search_results, rank_hits, BookMetadata, ProductHit};

pub const SITE: &str = "https://soundbooththeater.com";
const MAX_BODY_BYTES: usize = 4 << 20;
const MAX_RESULTS: usize = 3;
const SEARCH_DEADLINE: Duration = Duration::from_secs(8);

#[derive(Debug)]
pub enum UpstreamError {
    NotFound,
    RateLimited { retry_after_seconds: u64 },
    Failed(String),
}

#[derive(Clone)]
pub struct Soundbooth {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    site: String,
    http: reqwest::Client,
    slots: Semaphore,
    interval: Duration,
    next: Mutex<Instant>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchAnswer {
    pub matches: Vec<BookMetadata>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

#[derive(Debug, Deserialize)]
struct SearchEnvelope {
    success: Value,
    posts: Option<String>,
}

impl Soundbooth {
    pub fn new(site: &str, interval: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(format!(
                "bookorbit-soundbooththeater-provider/{}",
                crate::VERSION
            ))
            .build()
            .expect("could not build the HTTP client");
        Self {
            inner: Arc::new(ClientInner {
                site: site.trim_end_matches('/').to_string(),
                http,
                slots: Semaphore::new(1),
                interval,
                next: Mutex::new(Instant::now()),
            }),
        }
    }

    pub async fn search(
        &self,
        title: &str,
        author: &str,
        limit: usize,
    ) -> Result<SearchAnswer, UpstreamError> {
        let query = [title.trim(), author.trim()]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if query.is_empty() {
            return Ok(SearchAnswer::default());
        }

        let deadline = tokio::time::Instant::now() + SEARCH_DEADLINE;
        let envelope = match tokio::time::timeout_at(deadline, self.search_page(&query)).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(UpstreamError::NotFound)) => {
                return Err(UpstreamError::Failed(
                    "Soundbooth Theater AJAX search endpoint was not found".into(),
                ))
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(UpstreamError::Failed(
                    "Soundbooth Theater search timed out".into(),
                ))
            }
        };
        if !search_succeeded(&envelope.success) {
            return Err(UpstreamError::Failed(
                "Soundbooth Theater AJAX search returned success=false".into(),
            ));
        }
        let hits = search_hits(envelope.posts.as_deref().unwrap_or_default())?;
        let hits = rank_hits(hits, title, limit.clamp(1, MAX_RESULTS));
        let mut matches = Vec::new();
        let mut partial = false;
        let mut failure = None;
        for hit in hits {
            match tokio::time::timeout_at(deadline, self.product_page(&hit.provider_id)).await {
                Ok(Ok(html)) => matches.push(map_product(&hit.provider_id, &hit.url, &html)),
                Ok(Err(UpstreamError::NotFound)) => partial = true,
                Ok(Err(error)) => {
                    partial = true;
                    failure.get_or_insert(error);
                }
                Err(_) => {
                    partial = true;
                    break;
                }
            }
        }

        if matches.is_empty() && partial {
            return Err(failure.unwrap_or_else(|| {
                UpstreamError::Failed("Soundbooth Theater product pages timed out".into())
            }));
        }
        Ok(SearchAnswer { matches, partial })
    }

    pub async fn lookup(&self, id: &str) -> Result<Option<BookMetadata>, UpstreamError> {
        if !crate::mapping::valid_product_id(id) {
            return Ok(None);
        }
        match self.product_page(id).await {
            Ok(html) => {
                let url = self.product_url(id)?;
                Ok(Some(map_product(id, url.as_str(), &html)))
            }
            Err(UpstreamError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn search_page(&self, query: &str) -> Result<SearchEnvelope, UpstreamError> {
        let request = self
            .inner
            .http
            .post(format!("{}/wp-admin/admin-ajax.php", self.inner.site))
            .header("Accept", "application/json")
            .form(&[
                ("action", "loadmore"),
                ("page", "0"),
                ("first_page_load", "true"),
                ("search", query),
            ]);
        let body = self.send(request).await?;
        serde_json::from_slice(&body).map_err(|error| {
            UpstreamError::Failed(format!("invalid Soundbooth search JSON: {error}"))
        })
    }

    async fn product_page(&self, id: &str) -> Result<Html, UpstreamError> {
        if !crate::mapping::valid_product_id(id) {
            return Err(UpstreamError::NotFound);
        }
        let url = self.product_url(id)?;
        let request = self.inner.http.get(url).header("Accept", "text/html");
        let body = self.send(request).await?;
        let html = String::from_utf8(body)
            .map_err(|error| UpstreamError::Failed(format!("invalid Soundbooth HTML: {error}")))?;
        Ok(Html::parse_document(&html))
    }

    fn product_url(&self, id: &str) -> Result<Url, UpstreamError> {
        Url::parse(&format!("{}/shop/audiobooks/{id}/", self.inner.site))
            .map_err(|error| UpstreamError::Failed(error.to_string()))
    }

    async fn send(&self, request: RequestBuilder) -> Result<Vec<u8>, UpstreamError> {
        let _permit: SemaphorePermit<'_> = self
            .inner
            .slots
            .acquire()
            .await
            .map_err(|error| UpstreamError::Failed(error.to_string()))?;
        self.pace().await;
        let mut response = request
            .send()
            .await
            .map_err(|error| UpstreamError::Failed(error.without_url().to_string()))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(UpstreamError::NotFound);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_seconds = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|seconds| *seconds > 0)
                .unwrap_or(60);
            return Err(UpstreamError::RateLimited {
                retry_after_seconds,
            });
        }
        if !status.is_success() {
            return Err(UpstreamError::Failed(format!(
                "Soundbooth Theater answered {status}"
            )));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| UpstreamError::Failed(error.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_BODY_BYTES {
                return Err(UpstreamError::Failed(format!(
                    "Soundbooth Theater response exceeded {MAX_BODY_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn pace(&self) {
        let start = {
            let mut next = self.inner.next.lock().await;
            let start = (*next).max(Instant::now());
            *next = start + self.inner.interval;
            start
        };
        tokio::time::sleep_until(tokio::time::Instant::from_std(start)).await;
    }
}

fn search_succeeded(value: &Value) -> bool {
    match value {
        Value::Bool(success) => *success,
        Value::Number(number) => number.as_i64() == Some(1) || number.as_u64() == Some(1),
        Value::String(success) => success == "1" || success.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// No match is an empty `posts`. Products that can no longer be read mean the store's markup
/// changed, which has to fail rather than tell BookOrbit the book is not there.
fn search_hits(posts: &str) -> Result<Vec<ProductHit>, UpstreamError> {
    if posts.trim().is_empty() {
        return Ok(Vec::new());
    }
    let hits = parse_search_results(posts);
    if hits.is_empty() {
        return Err(UpstreamError::Failed(
            "Soundbooth Theater search results could not be read".into(),
        ));
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posts(raw: &str) -> String {
        let response: SearchEnvelope = serde_json::from_str(raw).unwrap();
        assert!(search_succeeded(&response.success));
        response.posts.unwrap_or_default()
    }

    #[test]
    fn reads_real_search_answers() {
        let none = posts(include_str!("../testdata/search-none.json"));
        assert!(search_hits(&none).unwrap().is_empty());
        let riftside = posts(include_str!("../testdata/search-riftside.json"));
        assert_eq!(search_hits(&riftside).unwrap().len(), 4);
    }

    #[test]
    fn unreadable_results_are_a_failure_not_an_empty_answer() {
        assert!(matches!(
            search_hits("<div class=\"redesigned\">Riftside, Book 1</div>"),
            Err(UpstreamError::Failed(_))
        ));
    }

    #[test]
    fn accepts_wordpress_success_values() {
        for raw in [
            r#"{"success":true,"posts":""}"#,
            r#"{"success":1,"posts":""}"#,
            r#"{"success":"1","posts":""}"#,
        ] {
            let response: SearchEnvelope = serde_json::from_str(raw).unwrap();
            assert!(search_succeeded(&response.success));
        }
        let response: SearchEnvelope = serde_json::from_str(r#"{"success":0,"posts":""}"#).unwrap();
        assert!(!search_succeeded(&response.success));
    }
}
