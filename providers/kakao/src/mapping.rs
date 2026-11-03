use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    pub media_type: &'static str,
    pub provider_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    pub publisher: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_year: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<SeriesEntry>,
    pub language_code: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_date: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_width: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_height: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeriesEntry {
    pub series: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sequence: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Deserialize)]
pub struct KakaoResponse {
    #[serde(default)]
    pub meta: KakaoMeta,
    #[serde(default)]
    pub documents: Vec<KakaoDocument>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct KakaoMeta {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub pageable_count: u64,
    #[serde(default)]
    pub is_end: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct KakaoDocument {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub contents: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub isbn: String,
    #[serde(default)]
    pub datetime: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub translators: Vec<String>,
    #[serde(default)]
    pub price: i64,
    #[serde(default)]
    pub sale_price: i64,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub status: String,
}

impl KakaoDocument {
    /// Stable for the same source record. Kakao does not return a dedicated volume ID.
    pub fn provider_id(&self) -> String {
        let identity = if !self.isbn.trim().is_empty() {
            format!(
                "isbn:{}",
                self.isbn.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        } else if !self.url.is_empty() {
            format!("url:{}", self.url)
        } else {
            format!(
                "title:{}|authors:{}|publisher:{}|date:{}",
                self.title,
                self.authors.join("|"),
                self.publisher,
                self.datetime
            )
        };
        let digest = Sha256::digest(identity.as_bytes());
        hex::encode(digest)
    }

    pub fn into_metadata(self) -> BookMetadata {
        let provider_id = self.provider_id();
        let published_date = self
            .datetime
            .get(..10)
            .filter(|date| {
                date.as_bytes().get(4) == Some(&b'-') && date.as_bytes().get(7) == Some(&b'-')
            })
            .unwrap_or_default()
            .to_string();
        let published_year = published_date.get(..4).unwrap_or_default().to_string();
        BookMetadata {
            media_type: "book",
            provider_id,
            title: self.title.trim().to_string(),
            authors: self
                .authors
                .into_iter()
                .filter(|v| !v.trim().is_empty())
                .collect(),
            publisher: self.publisher,
            published_year,
            description: self.contents,
            cover: self.thumbnail,
            genres: Vec::new(),
            series: Vec::new(),
            language_code: "ko",
            published_date,
            cover_width: 0,
            cover_height: 0,
            url: self.url,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_kakao_book_document() {
        let doc: KakaoDocument = serde_json::from_str(
            r#"{
            "title":"미움받을 용기", "contents":"소개", "url":"https://example.test/book",
            "isbn":"8996991341 9788996991342", "datetime":"2014-11-17T00:00:00.000+09:00",
            "authors":["기시미 이치로","고가 후미타케"], "publisher":"인플루엔셜",
            "translators":["전경아"], "price":14900, "sale_price":13410,
            "thumbnail":"https://example.test/cover", "status":"정상판매"
        }"#,
        )
        .unwrap();
        let book = doc.into_metadata();
        assert_eq!(book.title, "미움받을 용기");
        assert_eq!(book.authors, ["기시미 이치로", "고가 후미타케"]);
        assert_eq!(book.published_date, "2014-11-17");
        assert_eq!(book.published_year, "2014");
        assert_eq!(book.provider_id.len(), 64);
        assert!(book.provider_id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn identifier_is_stable_for_same_record() {
        let doc: KakaoDocument =
            serde_json::from_str(r#"{"title":"Book","isbn":"9781234567890"}"#).unwrap();
        assert_eq!(doc.provider_id(), doc.clone().provider_id());
    }
}
