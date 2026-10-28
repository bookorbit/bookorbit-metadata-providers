use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::header::RETRY_AFTER;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::mapping::{map_book, provider_id, BookMetadata};

const CATALOG_URLS: [&str; 2] = [
    "https://raw.githubusercontent.com/binyaminyblatt/graphicaudio_scraper/refs/heads/main/results.json",
    "https://raw.githubusercontent.com/binyaminyblatt/graphicaudio_scraper/refs/heads/main/wayback_results.json",
];
const MAX_CATALOG_BYTES: usize = 8 << 20;
const CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const CATALOG_REQUEST_GAP: Duration = Duration::from_millis(250);
const DEFAULT_RETRY_AFTER_SECONDS: u64 = 60;
pub const DEFAULT_LIMIT: usize = 5;
pub const MAX_LIMIT: usize = 20;

#[derive(Debug)]
pub enum UpstreamError {
    NotFound,
    RateLimited { retry_after_seconds: u64 },
    Failed(String),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CatalogBook {
    pub link: Option<String>,
    #[serde(rename = "wayback_link")]
    pub wayback_link: Option<String>,
    pub cover: Option<String>,
    pub series_name: Option<String>,
    pub title: Option<String>,
    pub rawtitle: Option<String>,
    pub episode_number: Option<serde_json::Value>,
    pub episode_part: Option<String>,
    pub episode_code: Option<String>,
    pub total_parts: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub release_date: Option<String>,
    pub isbn: Option<String>,
    pub genre: Option<String>,
    pub description: Option<String>,
    pub cast: Option<Vec<String>>,
    pub asin: Option<String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct SearchAnswer {
    pub matches: Vec<BookMetadata>,
    pub partial: bool,
}

#[derive(Clone)]
pub struct GraphicAudio {
    inner: Arc<Inner>,
}

struct Inner {
    http: Client,
    cache: Mutex<Option<CatalogCache>>,
    next_request: Mutex<Instant>,
}

#[derive(Clone)]
struct CatalogCache {
    loaded_at: Instant,
    books: Arc<Vec<CatalogBook>>,
    partial: bool,
}

impl GraphicAudio {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(format!(
                "bookorbit-graphicaudio-provider/{}",
                crate::VERSION
            ))
            .build()
            .expect("could not build the HTTP client");
        Self {
            inner: Arc::new(Inner {
                http,
                cache: Mutex::new(None),
                next_request: Mutex::new(Instant::now()),
            }),
        }
    }

    pub async fn search(
        &self,
        title: &str,
        author: &str,
        isbn: &str,
        limit: usize,
    ) -> Result<SearchAnswer, UpstreamError> {
        let catalog = self.catalog().await?;
        let clean_isbn = normalize_identifier(isbn);
        if !clean_isbn.is_empty() {
            let found = catalog.books.iter().find(|book| {
                normalize_identifier(book.isbn.as_deref().unwrap_or_default()) == clean_isbn
            });
            return Ok(SearchAnswer {
                matches: found.and_then(map_book).into_iter().collect(),
                partial: catalog.partial,
            });
        }

        let title = normalize_search(title);
        if title.is_empty() {
            return Ok(SearchAnswer {
                matches: Vec::new(),
                partial: catalog.partial,
            });
        }
        let author = normalize_search(author);
        let mut matches: Vec<(usize, &CatalogBook)> = catalog
            .books
            .iter()
            .enumerate()
            .filter(|(_, book)| title_matches(book, &title))
            .collect();
        matches.sort_by_key(|(index, book)| {
            (
                Reverse(
                    book.title
                        .as_deref()
                        .is_some_and(|value| normalize_search(value) == title),
                ),
                Reverse(
                    author.is_empty()
                        || book
                            .author
                            .as_deref()
                            .is_some_and(|value| normalize_search(value).contains(&author)),
                ),
                *index,
            )
        });

        Ok(SearchAnswer {
            matches: matches
                .into_iter()
                .take(limit.clamp(1, MAX_LIMIT))
                .filter_map(|(_, book)| map_book(book))
                .collect(),
            partial: catalog.partial,
        })
    }

    pub async fn lookup(&self, id: &str) -> Result<Option<BookMetadata>, UpstreamError> {
        if !valid_lookup_id(id) {
            return Ok(None);
        }
        let catalog = self.catalog().await?;
        let id = id.trim();
        Ok(catalog
            .books
            .iter()
            .find(|book| {
                let id_lower = id.to_ascii_lowercase();
                normalize_identifier(book.isbn.as_deref().unwrap_or_default())
                    == normalize_identifier(id)
                    || book
                        .asin
                        .as_deref()
                        .is_some_and(|asin| asin.eq_ignore_ascii_case(id))
                    || provider_id(book).as_deref() == Some(id_lower.as_str())
            })
            .and_then(map_book))
    }

    async fn catalog(&self) -> Result<CatalogCache, UpstreamError> {
        let mut cache = self.inner.cache.lock().await;
        if let Some(cached) = cache.as_ref() {
            if cached.loaded_at.elapsed() < CATALOG_TTL {
                return Ok(cached.clone());
            }
        }
        let stale = cache.clone();
        let mut books = Vec::new();
        let mut seen = HashSet::new();
        let mut successful = 0;
        let mut last_error = None;

        for url in CATALOG_URLS {
            match self.fetch_catalog(url).await {
                Ok(items) => {
                    successful += 1;
                    for book in items {
                        let Some(id) = provider_id(&book) else {
                            continue;
                        };
                        if seen.insert(id) {
                            books.push(book);
                        }
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }

        if successful == 0 {
            if let Some(mut stale) = stale {
                stale.partial = true;
                return Ok(stale);
            }
            return Err(last_error.unwrap_or_else(|| {
                UpstreamError::Failed("GraphicAudio catalogs were unavailable".into())
            }));
        }

        let refreshed = CatalogCache {
            loaded_at: Instant::now(),
            books: Arc::new(books),
            partial: successful < CATALOG_URLS.len(),
        };
        *cache = Some(refreshed.clone());
        Ok(refreshed)
    }

    async fn fetch_catalog(&self, url: &str) -> Result<Vec<CatalogBook>, UpstreamError> {
        self.pace().await;
        let mut response = self
            .inner
            .http
            .get(url)
            .header("Accept", "application/json")
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
                .unwrap_or(DEFAULT_RETRY_AFTER_SECONDS);
            return Err(UpstreamError::RateLimited {
                retry_after_seconds,
            });
        }
        if !status.is_success() {
            return Err(UpstreamError::Failed(format!(
                "GraphicAudio catalog answered {status}"
            )));
        }

        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| UpstreamError::Failed(error.without_url().to_string()))?
        {
            if body.len() + chunk.len() > MAX_CATALOG_BYTES {
                return Err(UpstreamError::Failed(format!(
                    "GraphicAudio catalog exceeded {MAX_CATALOG_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|error| {
            UpstreamError::Failed(format!("invalid GraphicAudio catalog JSON: {error}"))
        })
    }

    async fn pace(&self) {
        let start = {
            let mut next = self.inner.next_request.lock().await;
            let start = (*next).max(Instant::now());
            *next = start + CATALOG_REQUEST_GAP;
            start
        };
        tokio::time::sleep_until(tokio::time::Instant::from_std(start)).await;
    }
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn normalize_search(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn title_matches(book: &CatalogBook, query: &str) -> bool {
    [
        book.title.as_deref(),
        book.rawtitle.as_deref(),
        book.series_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| normalize_search(value).contains(query))
}

fn valid_lookup_id(id: &str) -> bool {
    let id = id.trim();
    (1..=512).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = include_str!("../testdata/results-revenge.json");
    const ARCHIVE: &str = include_str!("../testdata/wayback-stony-man.json");

    #[test]
    fn reads_current_and_archived_catalog_entries() {
        let current: Vec<CatalogBook> = serde_json::from_str(CURRENT).unwrap();
        let archive: Vec<CatalogBook> = serde_json::from_str(ARCHIVE).unwrap();
        assert_eq!(current[0].isbn.as_deref(), Some("9781648797835"));
        assert_eq!(archive[0].asin, None);
        assert_eq!(provider_id(&archive[0]).as_deref(), Some("9781599504018"));
    }

    #[test]
    fn identifiers_normalize_and_lookup_ids_are_bounded() {
        assert_eq!(normalize_identifier("978-1-64879-783-5"), "9781648797835");
        assert!(valid_lookup_id("9781648797835"));
        assert!(!valid_lookup_id("../etc/passwd"));
        assert!(!valid_lookup_id(&"x".repeat(513)));
    }

    #[test]
    fn searches_title_raw_title_and_series() {
        let books: Vec<CatalogBook> = serde_json::from_str(CURRENT).unwrap();
        let book = &books[0];
        assert!(title_matches(book, "revenge"));
        assert!(title_matches(book, "episode number 1"));
        assert!(title_matches(book, ".45-caliber"));
        assert!(!title_matches(book, "not present"));
    }
}
