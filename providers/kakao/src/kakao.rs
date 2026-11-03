use std::time::Duration;

use reqwest::{header, StatusCode};
use serde::de::DeserializeOwned;

use crate::mapping::KakaoResponse;

const API_URL: &str = "https://dapi.kakao.com/v3/search/book";
const MAX_BODY_BYTES: usize = 8 << 20;

#[derive(Debug)]
pub enum UpstreamError {
    RateLimited { retry_after_seconds: u64 },
    Unauthorized,
    Failed(String),
}

pub struct Client {
    key: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(key: &str, user_agent: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(user_agent)
            .build()
            .expect("could not build HTTP client");
        Self {
            key: key.to_string(),
            http,
        }
    }

    pub async fn search(
        &self,
        query: &str,
        target: Option<&str>,
        sort: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<KakaoResponse, UpstreamError> {
        let page = page.to_string();
        let size = size.to_string();
        let mut request = self
            .http
            .get(API_URL)
            .header(header::AUTHORIZATION, format!("KakaoAK {}", self.key))
            .query(&[
                ("query", query),
                ("page", page.as_str()),
                ("size", size.as_str()),
            ]);
        if let Some(target) = target {
            request = request.query(&[("target", target)]);
        }
        if let Some(sort) = sort {
            request = request.query(&[("sort", sort)]);
        }
        let response = request
            .send()
            .await
            .map_err(|e| UpstreamError::Failed(e.to_string()))?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(UpstreamError::Unauthorized);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_seconds = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(60);
            return Err(UpstreamError::RateLimited {
                retry_after_seconds,
            });
        }
        if !status.is_success() {
            return Err(UpstreamError::Failed(format!(
                "Kakao API answered {status}"
            )));
        }
        decode_limited(response).await
    }
}

async fn decode_limited<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, UpstreamError> {
    let mut body = Vec::new();
    let mut stream = response;
    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| UpstreamError::Failed(e.to_string()))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(UpstreamError::Failed("Kakao response too large".into()));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|e| UpstreamError::Failed(format!("Kakao response unreadable: {e}")))
}
