use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    pub media_type: &'static str,
    pub provider_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub series_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub community_rating: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn: String,
    pub language: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_date: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_width: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_height: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Yes24Envelope {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub message: String,
    pub error_code: Option<String>,
    #[serde(default)]
    pub data: Yes24Data,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Yes24Data {
    #[serde(default)]
    pub items: Vec<Yes24Item>,
    #[serde(default)]
    pub current_page: u32,
    #[serde(default)]
    pub page_size: u32,
    #[serde(default)]
    pub total_count: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Yes24Item {
    #[serde(default)]
    pub item_id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub sub_title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub goods_type: String,
    #[serde(default)]
    pub goods_sort_nm: String,
    #[serde(default)]
    pub adult_yn: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub isbn10: String,
    #[serde(default)]
    pub isbn13: String,
    #[serde(default, deserialize_with = "deserialize_optional_u32")]
    pub pages: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_optional_f64")]
    pub star_score: Option<f64>,
    #[serde(default)]
    pub series_name: String,
    #[serde(default)]
    pub publish_date: String,
    #[serde(default)]
    pub cover: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub content_detail: Option<ContentDetail>,
    #[serde(default)]
    pub series: Vec<Yes24Series>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentDetail {
    pub book_introduction: Option<String>,
    pub book_summary: Option<String>,
    pub table_of_contents: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct Yes24Series {
    #[serde(default)]
    pub series_id: u64,
    #[serde(default)]
    pub series_name: String,
}

impl Yes24Item {
    pub fn provider_id(&self) -> String {
        // BookOrbit stores the provider's native ID, without a provider-name prefix.
        // YES24 exposes this stable numeric identifier as itemId.
        self.item_id.to_string()
    }

    pub fn into_metadata(self) -> BookMetadata {
        let provider_id = self.provider_id();
        let published_date = normalize_date(&self.publish_date);
        let published_year = published_date.get(..4).unwrap_or_default().to_string();
        let authors = split_authors(&self.author);
        let subtitle = self.sub_title.trim().to_string();
        let mut description_parts = Vec::new();
        if let Some(content) = self.content_detail {
            push_section(&mut description_parts, "책 소개", content.book_introduction);
            push_section(&mut description_parts, "줄거리", content.book_summary);
            push_section(&mut description_parts, "목차", content.table_of_contents);
        }
        let description = description_parts.join("\n\n");
        // BookOrbit metadata uses a single series entry for this provider.
        // Prefer the first non-empty series from YES24's series array.
        let first_series = self
            .series
            .into_iter()
            .find(|item| !item.series_name.trim().is_empty())
            .map(|item| item.series_name.trim().to_string())
            .or_else(|| {
                let name = self.series_name.trim();
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            });
        let series_name = first_series.unwrap_or_default();

        let genres = if self.goods_sort_nm.trim().is_empty() {
            Vec::new()
        } else {
            vec![self.goods_sort_nm.trim().to_string()]
        };

        let (isbn13, _isbn10) =
            pick_isbns(&[self.isbn13.clone(), self.isbn10.clone()], &self.isbn13);
        let isbn = if isbn13.is_empty() {
            normalize_isbn(&self.isbn10)
        } else {
            isbn13
        };

        BookMetadata {
            media_type: "book",
            provider_id,
            title: self.title.trim().to_string(),
            subtitle,
            authors,
            publisher: self.publisher.trim().to_string(),
            published_year,
            description,
            cover: self.cover,
            genres,
            series_name,
            page_count: self.pages,
            community_rating: self.star_score,
            isbn,
            // YES24's Goods API does not document a language field. The provider defaults to Korean,
            // matching the existing BookOrbit Kakao provider's behavior.
            language: "ko",
            published_date,
            cover_width: 0,
            cover_height: 0,
            url: if self.link.trim().is_empty() && self.item_id > 0 {
                format!("https://www.yes24.com/product/goods/{}", self.item_id)
            } else {
                self.link
            },
        }
    }
}

fn deserialize_optional_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        serde_json::Value::Number(number) => number.as_u64().and_then(|n| u32::try_from(n).ok()),
        serde_json::Value::String(text) => text.trim().parse::<u32>().ok(),
        _ => None,
    }))
}

fn deserialize_optional_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value
        .and_then(|value| match value {
            serde_json::Value::Number(number) => number.as_f64(),
            serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
            _ => None,
        })
        .filter(|score| score.is_finite()))
}

fn normalize_isbn(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == 'X' || *c == 'x')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn to_isbn13(isbn: &str) -> String {
    let isbn = normalize_isbn(isbn);
    if isbn.len() == 13 && isbn.chars().all(|c| c.is_ascii_digit()) {
        return isbn;
    }
    if isbn.len() != 10 || !isbn[..9].chars().all(|c| c.is_ascii_digit()) {
        return String::new();
    }
    let mut digits = format!("978{}", &isbn[..9]);
    let sum: u32 = digits
        .chars()
        .enumerate()
        .map(|(index, ch)| ch.to_digit(10).unwrap_or(0) * if index % 2 == 0 { 1 } else { 3 })
        .sum();
    digits.push(char::from_digit((10 - sum % 10) % 10, 10).unwrap());
    digits
}

fn list_contains_isbn(valid: &[String], preferred: &str) -> bool {
    let preferred = normalize_isbn(preferred);
    valid
        .iter()
        .any(|candidate| candidate == &preferred || to_isbn13(candidate) == to_isbn13(&preferred))
}

/// Prefer the ISBN the client searched for when it identifies the same edition, while returning
/// the corresponding ISBN-13 and only its matching ISBN-10.
fn pick_isbns(list: &[String], preferred: &str) -> (String, String) {
    let valid: Vec<String> = list
        .iter()
        .map(|candidate| normalize_isbn(candidate))
        .filter(|isbn| {
            isbn.len() == 10 || (isbn.len() == 13 && isbn.chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    let preferred = normalize_isbn(preferred);
    let mut isbn13 = String::new();
    if !preferred.is_empty() && list_contains_isbn(&valid, &preferred) {
        isbn13 = to_isbn13(&preferred);
    }
    if isbn13.is_empty() {
        if let Some(first) = valid.iter().find(|isbn| isbn.len() == 13) {
            isbn13 = to_isbn13(first);
        } else if let Some(first) = valid.first() {
            isbn13 = to_isbn13(first);
        }
    }
    let isbn10 = valid
        .iter()
        .find(|isbn| isbn.len() == 10 && !isbn13.is_empty() && to_isbn13(isbn) == isbn13)
        .cloned()
        .unwrap_or_default();
    (isbn13, isbn10)
}

fn normalize_date(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 10 {
        let date = &value[..10];
        if date.as_bytes().get(4) == Some(&b'-') && date.as_bytes().get(7) == Some(&b'-') {
            return date.to_string();
        }
    }
    value.to_string()
}

fn split_authors(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    // YES24 combines contributors and their roles in one string, often separated by '/'.
    // Split explicit separators while retaining role labels such as '저', '그림', and '역'.
    value
        .split([',', ';', '|', '/', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn push_section(parts: &mut Vec<String>, label: &str, value: Option<String>) {
    if let Some(value) = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        parts.push(format!("{label}\n{value}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_extended_yes24_item() {
        let item: Yes24Item = serde_json::from_str(r#"{
          "itemId":12345678,"title":"클린 코드","subTitle":"애자일 소프트웨어 장인 정신",
          "author":"로버트 C. 마틴","publisher":"인사이트","isbn10":"8966260957","isbn13":"9788966260959",
          "pages":"584","starScore":9.2,"goodsSortNm":"컴퓨터/IT",
          "publishDate":"2013-12-24","cover":"https://image.yes24.com/goods/12345678/L",
          "link":"https://www.yes24.com/product/goods/12345678",
          "contentDetail":{"bookIntroduction":"소개","bookSummary":null,"tableOfContents":"1장"},
          "seriesName":"마틴 파울러 시리즈",
          "series":[
            {"seriesId":10001,"seriesName":"마틴 파울러 시리즈"},
            {"seriesId":10002,"seriesName":"다른 시리즈"}
          ]
        }"#).unwrap();
        let book = item.into_metadata();
        assert_eq!(book.provider_id, "12345678");
        assert_eq!(book.title, "클린 코드");
        assert_eq!(book.subtitle, "애자일 소프트웨어 장인 정신");
        assert_eq!(book.authors, ["로버트 C. 마틴"]);
        assert_eq!(book.published_date, "2013-12-24");
        assert_eq!(book.published_year, "2013");
        assert!(book.description.contains("책 소개\n소개"));
        assert_eq!(book.series_name, "마틴 파울러 시리즈");
        assert_eq!(book.page_count, Some(584));
        assert_eq!(book.community_rating, Some(9.2));
        assert_eq!(book.isbn, "9788966260959");
        assert_eq!(book.genres, ["컴퓨터/IT"]);
    }

    #[test]
    fn splits_yes24_contributors_separated_by_slashes() {
        let authors = split_authors("허버트 조지 웰즈 저/이태영 그림/채희석 역");
        assert_eq!(authors, ["허버트 조지 웰즈 저", "이태영 그림", "채희석 역"]);
    }

    #[test]
    fn prefers_isbn13_and_converts_matching_isbn10() {
        let (isbn13, isbn10) = pick_isbns(
            &["8966260957".to_string(), "9788966260959".to_string()],
            "8966260957",
        );
        assert_eq!(isbn13, "9788966260959");
        assert_eq!(isbn10, "8966260957");
    }

    #[test]
    fn handles_null_extended_fields_and_missing_item_id() {
        let item: Yes24Item = serde_json::from_str(
            r#"{"title":"테스트","author":"A, B","contentDetail":null,"series":[]}"#,
        )
        .unwrap();
        let book = item.into_metadata();
        assert_eq!(book.authors, ["A", "B"]);
        assert!(book.description.is_empty());
        assert_eq!(book.provider_id, "0");
    }
}
