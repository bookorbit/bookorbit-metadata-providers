use scraper::Html;
use serde::Serialize;
use url::Url;

use crate::graphicaudio::CatalogBook;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    pub media_type: String,
    pub provider_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub narrators: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn13: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn10: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub asin: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_year: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<Series>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub series: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sequence: String,
}

pub fn provider_id(book: &CatalogBook) -> Option<String> {
    if let Some(isbn) = book
        .isbn
        .as_deref()
        .map(normalize_identifier)
        .filter(|id| !id.is_empty())
    {
        return Some(isbn);
    }
    if let Some(asin) = book
        .asin
        .as_deref()
        .map(|id| id.trim().to_ascii_uppercase())
        .filter(|id| !id.is_empty())
    {
        return Some(asin);
    }
    product_slug(book.link.as_deref()?)
}

pub fn map_book(book: &CatalogBook) -> Option<BookMetadata> {
    let title = clean(book.title.as_deref().or(book.rawtitle.as_deref())?);
    let provider_id = provider_id(book)?;
    let isbn = book
        .isbn
        .as_deref()
        .map(str::trim)
        .filter(|isbn| !isbn.is_empty())
        .unwrap_or_default()
        .to_string();
    let normalized_isbn = normalize_identifier(&isbn);
    let (isbn10, isbn13) = match normalized_isbn.len() {
        10 => (normalized_isbn.clone(), String::new()),
        13 => (String::new(), normalized_isbn.clone()),
        _ => (String::new(), String::new()),
    };
    let series = if book
        .wayback_link
        .as_deref()
        .is_some_and(|link| !link.is_empty())
    {
        Vec::new()
    } else {
        book.series_name
            .as_deref()
            .map(clean)
            .filter(|series| !series.is_empty())
            .map(|series| {
                vec![Series {
                    series,
                    sequence: book
                        .episode_number
                        .as_ref()
                        .and_then(value_as_string)
                        .unwrap_or_default(),
                }]
            })
            .unwrap_or_default()
    };
    let narrators = book
        .cast
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|name| !name.trim().eq_ignore_ascii_case("narrator"))
        .take(10)
        .map(|name| clean(name))
        .filter(|name| !name.is_empty())
        .collect();
    let genres = book
        .genre
        .as_deref()
        .map(clean)
        .filter(|genre| !genre.is_empty())
        .into_iter()
        .collect();
    let description = book
        .description
        .as_deref()
        .map(plain_text)
        .unwrap_or_default();

    Some(BookMetadata {
        media_type: "audiobook".into(),
        provider_id,
        title,
        subtitle: book.subtitle.as_deref().map(clean).unwrap_or_default(),
        authors: book
            .author
            .as_deref()
            .map(clean)
            .filter(|author| !author.is_empty())
            .into_iter()
            .collect(),
        narrators,
        description,
        cover: book.cover.as_deref().map(clean).unwrap_or_default(),
        isbn,
        isbn13,
        isbn10,
        asin: book.asin.as_deref().map(clean).unwrap_or_default(),
        published_year: book
            .release_date
            .as_deref()
            .and_then(year_prefix)
            .unwrap_or_default(),
        genres,
        series,
        url: book.link.as_deref().map(clean).unwrap_or_default(),
    })
}

pub fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase())
        .collect()
}

fn product_slug(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let slug = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    valid_product_slug(slug).then(|| slug.to_ascii_lowercase())
}

fn valid_product_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 255
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte))
}

fn year_prefix(date: &str) -> Option<String> {
    let year = date.get(..4)?;
    year.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| year.to_string())
}

fn value_as_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::String(string) if !string.trim().is_empty() => {
            Some(string.trim().to_string())
        }
        _ => None,
    }
}

fn clean(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn plain_text(value: &str) -> String {
    let fragment = Html::parse_fragment(value);
    clean(&fragment.root_element().text().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURRENT: &str = include_str!("../testdata/results-revenge.json");
    const ARCHIVE: &str = include_str!("../testdata/wayback-stony-man.json");

    #[test]
    fn maps_current_catalog_metadata_and_identifier() {
        let books: Vec<CatalogBook> = serde_json::from_str(CURRENT).unwrap();
        let metadata = map_book(&books[0]).unwrap();
        assert_eq!(metadata.provider_id, "9781648797835");
        assert_eq!(metadata.media_type, "audiobook");
        assert_eq!(metadata.title, "Revenge");
        assert_eq!(metadata.authors, ["Peter Brandvold"]);
        assert_eq!(metadata.narrators[0], "Nathanial Perry");
        assert_eq!(metadata.isbn13, "9781648797835");
        assert_eq!(metadata.asin, "1648797830");
        assert_eq!(metadata.published_year, "2017");
        assert_eq!(metadata.genres, ["Western"]);
        assert_eq!(metadata.series[0].series, ".45-Caliber");
        assert_eq!(metadata.series[0].sequence, "1");
        assert!(metadata.url.ends_with("45-caliber-1-revenge.html"));
    }

    #[test]
    fn falls_back_to_a_catalog_link_for_old_records_without_isbn_or_asin() {
        let mut books: Vec<CatalogBook> = serde_json::from_str(ARCHIVE).unwrap();
        let book = &mut books[0];
        book.isbn = None;
        book.asin = None;
        assert_eq!(
            provider_id(book).as_deref(),
            Some("stony-man-1-stony-man-doctrine.html")
        );
        assert!(map_book(book).unwrap().series.is_empty());
    }

    #[test]
    fn strips_markup_from_catalog_descriptions() {
        assert_eq!(
            plain_text("<p>First &amp; second.</p><p>Third.</p>"),
            "First & second. Third."
        );
    }
}
