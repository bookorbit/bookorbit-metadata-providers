use serde::{Deserialize, Serialize};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<&'static str>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn13: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn10: String,
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
    pub documents: Vec<KakaoDocument>,
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
    pub fn into_metadata(self) -> Option<BookMetadata> {
        let (isbn13, isbn10) = split_isbns(&self.isbn);
        let provider_id = if !isbn13.is_empty() {
            isbn13.clone()
        } else if !isbn10.is_empty() {
            isbn10.clone()
        } else {
            return None;
        };

        let published_date = self
            .datetime
            .get(..10)
            .filter(|date| {
                date.as_bytes().get(4) == Some(&b'-') && date.as_bytes().get(7) == Some(&b'-')
            })
            .unwrap_or_default()
            .to_string();
        let published_year = published_date.get(..4).unwrap_or_default().to_string();
        let cover = self
            .thumbnail
            .parse::<reqwest::Url>()
            .ok()
            .and_then(|url| {
                url.query_pairs()
                    .find(|(key, _)| key == "fname")
                    .map(|(_, value)| value.into_owned())
            })
            .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
            .unwrap_or(self.thumbnail);

        Some(BookMetadata {
            media_type: "book",
            provider_id,
            title: self.title.trim().to_string(),
            authors: self
                .authors
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .collect(),
            publisher: self.publisher,
            published_year,
            description: self.contents,
            cover,
            genres: Vec::new(),
            series: Vec::new(),
            language_code: None,
            isbn13,
            isbn10,
            published_date,
            cover_width: 0,
            cover_height: 0,
            url: self.url,
        })
    }
}

fn split_isbns(value: &str) -> (String, String) {
    let mut isbn13 = String::new();
    let mut isbn10 = String::new();

    for candidate in value.split_whitespace() {
        let normalized: String = candidate
            .chars()
            .filter(|character| {
                character.is_ascii_digit() || *character == 'X' || *character == 'x'
            })
            .map(|character| character.to_ascii_uppercase())
            .collect();
        match normalized.len() {
            13 if is_valid_isbn(&normalized) => isbn13 = normalized,
            10 if is_valid_isbn(&normalized) => isbn10 = normalized,
            _ => {}
        }
    }

    (isbn13, isbn10)
}

pub fn normalize_isbn(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

pub fn is_valid_isbn(value: &str) -> bool {
    let normalized = normalize_isbn(value);
    if is_placeholder_isbn(&normalized) {
        return false;
    }
    match normalized.len() {
        10 => is_valid_isbn10(&normalized),
        13 => is_valid_isbn13(&normalized),
        _ => false,
    }
}

fn is_placeholder_isbn(value: &str) -> bool {
    if value.len() == 10 {
        value[..9].bytes().all(|byte| byte == b'0')
    } else if value.len() == 13 {
        matches!(&value[..3], "978" | "979") && value[3..12].bytes().all(|byte| byte == b'0')
    } else {
        false
    }
}

fn is_valid_isbn10(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || !bytes[..9].iter().all(|byte| byte.is_ascii_digit())
        || !(bytes[9].is_ascii_digit() || bytes[9] == b'X')
    {
        return false;
    }
    let sum: u32 = bytes
        .iter()
        .enumerate()
        .map(|(i, byte)| {
            let digit = if *byte == b'X' {
                10
            } else {
                (*byte - b'0') as u32
            };
            digit * (10 - i as u32)
        })
        .sum();
    sum.is_multiple_of(11)
}

fn is_valid_isbn13(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 13
        || !matches!(&value[..3], "978" | "979")
        || !bytes.iter().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    let sum: u32 = bytes[..12]
        .iter()
        .enumerate()
        .map(|(i, byte)| (*byte - b'0') as u32 * if i % 2 == 0 { 1 } else { 3 })
        .sum();
    (10 - sum % 10) % 10 == (bytes[12] - b'0') as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_kakao_book_document() {
        let doc: KakaoDocument = serde_json::from_str(
            r#"{
            "title":"미움받을 용기", "contents":"소개", "url":"https://search.daum.net/search?w=bookpage&bookId=6252918&q=%EB%AF%B8%EC%9B%80%EB%B0%9B%EC%9D%84+%EC%9A%A9%EA%B8%B0+2%28200%EB%A7%8C+%EB%B6%80+%EA%B8%B0%EB%85%90+%EC%8A%A4%ED%8E%98%EC%85%9C+%EC%97%90%EB%94%94%EC%85%98%29",
            "isbn":"8996991341 9788996991342", "datetime":"2014-11-17T00:00:00.000+09:00",
            "authors":["기시미 이치로","고가 후미타케"], "publisher":"인플루엔셜",
            "translators":["전경아"], "price":14900, "sale_price":13410,
            "thumbnail":"http://t1.daumcdn.net/lbook/image/1467038?timestamp=20230128141840", "status":"정상판매"
        }"#,
        )
        .unwrap();
        let book = doc.into_metadata().unwrap();
        assert_eq!(book.title, "미움받을 용기");
        assert_eq!(book.authors, ["기시미 이치로", "고가 후미타케"]);
        assert_eq!(book.published_date, "2014-11-17");
        assert_eq!(book.published_year, "2014");
        assert_eq!(book.provider_id, "9788996991342");
        assert_eq!(book.isbn10, "8996991341");
        assert_eq!(book.isbn13, "9788996991342");
        assert_eq!(book.language_code, None);
        assert!(serde_json::to_value(&book)
            .unwrap()
            .get("languageCode")
            .is_none());
        assert_eq!(
            book.cover,
            "http://t1.daumcdn.net/lbook/image/1467038?timestamp=20230128141840"
        );
    }

    #[test]
    fn rejects_all_zero_fixture_documents() {
        let response: KakaoResponse =
            serde_json::from_str(include_str!("../testdata/response-isbn-all0s.json")).unwrap();
        assert_eq!(
            response
                .documents
                .into_iter()
                .filter_map(KakaoDocument::into_metadata)
                .count(),
            0
        );
    }

    #[test]
    fn rejects_placeholder_and_invalid_isbns() {
        assert!(!is_valid_isbn("0000000000"));
        assert!(!is_valid_isbn("9780000000002"));
        assert!(!is_valid_isbn("9781234567890"));
        assert!(!is_valid_isbn("9778996991342"));
        assert!(is_valid_isbn("9788996991342"));
        assert_eq!(
            split_isbns("0000000000 9781234567890"),
            (String::new(), String::new())
        );
        let invalid: KakaoDocument =
            serde_json::from_str(r#"{"title":"Invalid","isbn":"9781234567890"}"#).unwrap();
        assert!(invalid.into_metadata().is_none());
    }

    #[test]
    fn identifier_is_stable_for_same_record() {
        let doc: KakaoDocument =
            serde_json::from_str(r#"{"title":"Book","isbn":"9788996991342"}"#).unwrap();
        assert_eq!(doc.into_metadata().unwrap().provider_id, "9788996991342");
    }
}

#[cfg(test)]
mod isbn_tests {
    use super::*;
    #[test]
    fn omits_documents_without_isbn() {
        let doc: KakaoDocument = serde_json::from_str(r#"{"title":"No ISBN"}"#).unwrap();
        assert!(doc.into_metadata().is_none());
    }
}
