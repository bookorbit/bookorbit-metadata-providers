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
    pub language_code: &'static str,
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
            language_code: "ko",
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
            13 if normalized
                .chars()
                .all(|character| character.is_ascii_digit()) =>
            {
                isbn13 = normalized;
            }
            10 => isbn10 = normalized,
            _ => {}
        }
    }

    (isbn13, isbn10)
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
        assert_eq!(
            book.cover,
            "http://t1.daumcdn.net/lbook/image/1467038?timestamp=20230128141840"
        );
    }

    #[test]
    fn identifier_is_stable_for_same_record() {
        let doc: KakaoDocument =
            serde_json::from_str(r#"{"title":"Book","isbn":"9781234567890"}"#).unwrap();
        assert_eq!(doc.into_metadata().unwrap().provider_id, "9781234567890");
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
