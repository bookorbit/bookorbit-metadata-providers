use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use reqwest::Url;
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use serde_json::Value;

use crate::soundbooth::SITE;

static PRODUCT_CARDS: LazyLock<Selector> = LazyLock::new(|| selector("li.product"));
static PRODUCT_LINK: LazyLock<Selector> =
    LazyLock::new(|| selector("a.woocommerce-LoopProduct-link"));
static PRODUCT_TITLE: LazyLock<Selector> =
    LazyLock::new(|| selector(".woocommerce-loop-product__title"));
static PRODUCT_IMAGE: LazyLock<Selector> =
    LazyLock::new(|| selector("img.attachment-woocommerce_thumbnail"));
static DETAIL_TITLE: LazyLock<Selector> = LazyLock::new(|| selector("h1.product_title"));
static SYNOPSIS: LazyLock<Selector> = LazyLock::new(|| selector(".synopsis.readmore"));
static DESCRIPTION_TAB: LazyLock<Selector> = LazyLock::new(|| selector("#tab-description"));
static SHORT_DESCRIPTION: LazyLock<Selector> =
    LazyLock::new(|| selector(".woocommerce-product-details__short-description"));
static PARAGRAPH: LazyLock<Selector> = LazyLock::new(|| selector("p"));
static AUTHOR_LINKS: LazyLock<Selector> = LazyLock::new(|| selector(".summary h3 a"));
static AUDIOBOOK_ITEMS: LazyLock<Selector> = LazyLock::new(|| selector("ul.audiobook-meta li"));
static LABEL: LazyLock<Selector> = LazyLock::new(|| selector("span"));
static LINKS: LazyLock<Selector> = LazyLock::new(|| selector("a"));
static POSTED_GENRES: LazyLock<Selector> = LazyLock::new(|| selector(".posted_in a"));
static JSON_LD: LazyLock<Selector> = LazyLock::new(|| selector("script.yoast-schema-graph"));
static OG_DESCRIPTION: LazyLock<Selector> =
    LazyLock::new(|| selector("meta[property='og:description']"));
static OG_TITLE: LazyLock<Selector> = LazyLock::new(|| selector("meta[property='og:title']"));
static OG_IMAGE: LazyLock<Selector> = LazyLock::new(|| selector("meta[property='og:image']"));
static TITLE_SERIES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.*?),\s*(?:book|episode)\s+([0-9]+(?:\.[0-9]+)?)(?::\s*(.*))?$")
        .expect("valid series-title pattern")
});
static DURATION_PART: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([0-9]+)\s*(hours?|hrs?|h|minutes?|mins?|m|seconds?|secs?|s)\b")
        .expect("valid duration pattern")
});
static PRODUCT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9-]{0,127}$").expect("valid product id pattern"));

fn selector(pattern: &str) -> Selector {
    Selector::parse(pattern).expect("valid CSS selector")
}

#[derive(Debug, Clone)]
pub struct ProductHit {
    pub provider_id: String,
    pub url: String,
    pub title: String,
    pub subtitle: String,
    pub cover: String,
    pub cover_width: u32,
    pub cover_height: u32,
    pub series: Vec<Series>,
}

impl ProductHit {
    pub fn lookup(id: &str, url: &str) -> Self {
        Self {
            provider_id: id.to_string(),
            url: url.to_string(),
            title: String::new(),
            subtitle: String::new(),
            cover: String::new(),
            cover_width: 0,
            cover_height: 0,
            series: Vec::new(),
        }
    }
}

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
    pub publisher: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub published_year: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub cover: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_width: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub cover_height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
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

fn is_zero(value: &u32) -> bool {
    *value == 0
}

pub fn valid_product_id(raw: &str) -> bool {
    PRODUCT_ID.is_match(raw)
}

pub fn parse_search_results(html: &str, limit: usize) -> Vec<ProductHit> {
    let document = Html::parse_fragment(html);
    let mut hits = Vec::new();
    for product in document.select(&PRODUCT_CARDS) {
        let Some(link) = product.select(&PRODUCT_LINK).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Some((provider_id, url)) = product_url(href) else {
            continue;
        };
        let Some(title) = product
            .select(&PRODUCT_TITLE)
            .next()
            .map(element_text)
            .filter(|title| !title.is_empty())
        else {
            continue;
        };
        let cover = product.select(&PRODUCT_IMAGE).next().and_then(parse_cover);
        let (title, subtitle, series) = split_series_title(&title);
        hits.push(ProductHit {
            provider_id,
            url,
            title,
            subtitle,
            cover: cover
                .as_ref()
                .map_or_else(String::new, |cover| cover.url.clone()),
            cover_width: cover.as_ref().map_or(0, |cover| cover.width),
            cover_height: cover.as_ref().map_or(0, |cover| cover.height),
            series,
        });
        if hits.len() == limit {
            break;
        }
    }
    hits
}

pub fn map_product(hit: &ProductHit, detail: &Html) -> BookMetadata {
    let mut book = BookMetadata {
        media_type: "audiobook".into(),
        provider_id: hit.provider_id.clone(),
        title: hit.title.clone(),
        subtitle: hit.subtitle.clone(),
        cover: hit.cover.clone(),
        cover_width: hit.cover_width,
        cover_height: hit.cover_height,
        series: hit.series.clone(),
        url: hit.url.clone(),
        ..BookMetadata::default()
    };

    if book.title.is_empty() {
        book.title = detail
            .select(&DETAIL_TITLE)
            .next()
            .map(element_text)
            .filter(|title| !title.is_empty())
            .or_else(|| {
                detail
                    .select(&OG_TITLE)
                    .next()
                    .and_then(|element| element.value().attr("content"))
                    .map(|title| {
                        title
                            .split(" - Soundbooth Theater")
                            .next()
                            .unwrap_or(title)
                            .trim()
                            .to_string()
                    })
                    .filter(|title| !title.is_empty())
            })
            .unwrap_or_default();
    }

    book.authors = unique(detail.select(&AUTHOR_LINKS).map(element_text));
    for item in detail.select(&AUDIOBOOK_ITEMS) {
        let label = item
            .select(&LABEL)
            .next()
            .map(element_text)
            .unwrap_or_default()
            .trim_end_matches(':')
            .trim()
            .to_ascii_lowercase();
        match label.as_str() {
            "narration" | "featuring" => {
                let linked: Vec<String> = item.select(&LINKS).map(element_text).collect();
                if linked.is_empty() {
                    let text = element_text(item);
                    let name = text
                        .split_once(':')
                        .map_or(text.as_str(), |(_, value)| value)
                        .trim();
                    if !name.is_empty() {
                        book.narrators.push(name.to_string());
                    }
                } else {
                    book.narrators.extend(linked);
                }
            }
            "length" => {
                let text = element_text(item);
                book.duration =
                    parse_duration(text.split_once(':').map_or(text.as_str(), |(_, v)| v));
            }
            "genres" => book.genres.extend(item.select(&LINKS).map(element_text)),
            "series" => {
                for name in item.select(&LINKS).map(element_text) {
                    add_series(&mut book.series, name, String::new());
                }
            }
            _ => {}
        }
    }
    book.narrators = unique(book.narrators);
    book.genres
        .extend(detail.select(&POSTED_GENRES).map(element_text));
    book.genres = unique(book.genres);
    book.published_year = published_year(detail);

    let description = detail
        .select(&SYNOPSIS)
        .next()
        .or_else(|| detail.select(&DESCRIPTION_TAB).next())
        .or_else(|| detail.select(&SHORT_DESCRIPTION).next())
        .map(paragraph_text)
        .filter(|text| text.len() >= 50)
        .or_else(|| {
            detail
                .select(&OG_DESCRIPTION)
                .next()
                .and_then(|element| element.value().attr("content"))
                .map(str::to_string)
        })
        .unwrap_or_default();
    if let Some((description, publisher)) = split_copyright(&description) {
        book.description = description;
        book.publisher = publisher;
    } else {
        book.description = description;
    }

    if book.cover.is_empty() {
        if let Some(cover) = detail
            .select(&OG_IMAGE)
            .next()
            .and_then(|element| element.value().attr("content"))
        {
            book.cover = cover.to_string();
        }
    }
    book
}

fn product_url(raw: &str) -> Option<(String, String)> {
    let base = Url::parse(SITE).ok()?;
    let url = base.join(raw).ok()?;
    if url.host_str()? != "soundbooththeater.com" || !url.path().starts_with("/shop/audiobooks/") {
        return None;
    }
    let id = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .next_back()?;
    valid_product_id(id).then(|| (id.to_string(), url.to_string()))
}

struct Cover {
    url: String,
    width: u32,
    height: u32,
}

fn parse_cover(image: ElementRef<'_>) -> Option<Cover> {
    let source_width = image
        .value()
        .attr("width")
        .and_then(|v| v.parse::<u32>().ok());
    let source_height = image
        .value()
        .attr("height")
        .and_then(|v| v.parse::<u32>().ok());
    let mut candidates = image
        .value()
        .attr("srcset")
        .unwrap_or_default()
        .split(',')
        .filter_map(|entry| {
            let mut pieces = entry.split_whitespace();
            let url = pieces.next()?.trim();
            let descriptor = pieces.next().unwrap_or_default();
            let width = descriptor.strip_suffix('w')?.parse::<u32>().ok()?;
            (!url.is_empty()).then(|| (width, url.to_string()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(width, _)| *width);
    let (width, url) = candidates.pop().or_else(|| {
        image
            .value()
            .attr("src")
            .filter(|src| !src.is_empty())
            .map(|src| (source_width.unwrap_or_default(), src.to_string()))
    })?;
    let height = match (source_width, source_height) {
        (Some(source_width), Some(source_height)) if source_width > 0 => {
            width.saturating_mul(source_height) / source_width
        }
        _ => 0,
    };
    Some(Cover { url, width, height })
}

fn split_series_title(raw: &str) -> (String, String, Vec<Series>) {
    let Some(captures) = TITLE_SERIES.captures(raw.trim()) else {
        return (raw.trim().to_string(), String::new(), Vec::new());
    };
    let series = captures.get(1).map_or("", |value| value.as_str()).trim();
    let sequence = captures.get(2).map_or("", |value| value.as_str()).trim();
    let subtitle = captures.get(3).map_or("", |value| value.as_str()).trim();
    let title = if subtitle.is_empty() {
        raw.trim().to_string()
    } else {
        subtitle.to_string()
    };
    let subtitle = if subtitle.is_empty() {
        String::new()
    } else {
        raw.trim().to_string()
    };
    let series = if series.is_empty() {
        Vec::new()
    } else {
        vec![Series {
            series: series.to_string(),
            sequence: sequence.to_string(),
        }]
    };
    (title, subtitle, series)
}

fn add_series(series: &mut Vec<Series>, name: String, sequence: String) {
    let Some(existing) = series
        .iter_mut()
        .find(|entry| entry.series.eq_ignore_ascii_case(&name))
    else {
        if !name.trim().is_empty() {
            series.push(Series {
                series: name,
                sequence,
            });
        }
        return;
    };
    if existing.sequence.is_empty() {
        existing.sequence = sequence;
    }
}

fn paragraph_text(root: ElementRef<'_>) -> String {
    let paragraphs: Vec<String> = root
        .select(&PARAGRAPH)
        .map(element_text)
        .filter(|text| !text.is_empty())
        .collect();
    if paragraphs.is_empty() {
        element_text(root)
    } else {
        paragraphs.join("\n\n")
    }
}

fn split_copyright(raw: &str) -> Option<(String, String)> {
    let (description, credits) = raw.split_once('\u{00a9}')?;
    let publisher = credits
        .split('\u{2117}')
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    let publisher = if publisher
        .first()
        .is_some_and(|year| year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()))
    {
        publisher[1..].join(" ")
    } else {
        publisher.join(" ")
    };
    Some((description.trim().to_string(), publisher))
}

fn published_year(document: &Html) -> String {
    for script in document.select(&JSON_LD) {
        let raw = script.text().collect::<String>();
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let graph: Vec<&Value> = value
            .get("@graph")
            .and_then(Value::as_array)
            .map(|nodes| nodes.iter().collect())
            .unwrap_or_else(|| vec![&value]);
        if let Some(year) = graph
            .iter()
            .filter_map(|node| node.get("datePublished").and_then(Value::as_str))
            .find_map(year_prefix)
        {
            return year;
        }
    }
    String::new()
}

fn year_prefix(date: &str) -> Option<String> {
    let year = date.get(..4)?;
    year.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| year.to_string())
}

fn parse_duration(raw: &str) -> Option<u32> {
    let mut seconds = 0u64;
    for captures in DURATION_PART.captures_iter(raw) {
        let amount = captures.get(1)?.as_str().parse::<u64>().ok()?;
        let unit = captures.get(2)?.as_str().to_ascii_lowercase();
        let multiplier = match unit.as_str() {
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            _ => 0,
        };
        seconds = seconds.saturating_add(amount.saturating_mul(multiplier));
    }
    (seconds > 0).then(|| seconds.min(u64::from(u32::MAX)) as u32)
}

fn element_text(element: ElementRef<'_>) -> String {
    normalize(&element.text().collect::<Vec<_>>().join(" "))
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unique(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| normalize(&value))
        .filter(|value| !value.is_empty() && seen.insert(value.to_ascii_lowercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: &str = include_str!("../testdata/search-rickshaw-riot.html");
    const DETAIL: &str = include_str!("../testdata/detail-rickshaw-riot.html");

    #[test]
    fn parses_a_real_product_card_and_chooses_largest_cover() {
        let hits = parse_search_results(SEARCH, 3);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provider_id, "rickshaw-riot-book-1");
        assert_eq!(hits[0].title, "Rickshaw Riot, Book 1");
        assert_eq!(hits[0].cover_width, 1024);
        assert_eq!(hits[0].cover_height, 1024);
        assert!(hits[0].cover.ends_with("RickshawRiot_Cover.jpg"));
        assert_eq!(hits[0].series[0].series, "Rickshaw Riot");
        assert_eq!(hits[0].series[0].sequence, "1");
    }

    #[test]
    fn maps_product_details_to_bookorbit_metadata() {
        let hit = parse_search_results(SEARCH, 1).remove(0);
        let detail = Html::parse_document(DETAIL);
        let book = map_product(&hit, &detail);
        assert_eq!(book.media_type, "audiobook");
        assert_eq!(book.title, "Rickshaw Riot, Book 1");
        assert_eq!(book.authors, ["Ben Wolf", "Luke Messa"]);
        assert_eq!(
            book.narrators,
            ["Ryan H. Reid", "Gary Furlong", "Jeff Hays"]
        );
        assert_eq!(book.duration, Some(35_760));
        assert_eq!(book.publisher, "Splickety Publishing Group");
        assert_eq!(book.published_year, "2026");
        assert_eq!(book.genres, ["Fantasy", "LitRPG"]);
        assert_eq!(book.series[0].sequence, "1");
        assert_eq!(book.description, "A short test synopsis.");
    }

    #[test]
    fn lookup_populates_title_from_the_product_page() {
        let detail = Html::parse_document(DETAIL);
        let hit = ProductHit::lookup(
            "rickshaw-riot-book-1",
            "https://soundbooththeater.com/shop/audiobooks/rickshaw-riot-book-1/",
        );
        assert_eq!(map_product(&hit, &detail).title, "Rickshaw Riot, Book 1");
    }

    #[test]
    fn duration_reads_hours_minutes_and_seconds() {
        assert_eq!(parse_duration("9 hours and 56 minutes"), Some(35_760));
        assert_eq!(parse_duration("1 hr 2 min 3 sec"), Some(3_723));
        assert_eq!(parse_duration("unknown"), None);
    }

    #[test]
    fn rejects_non_audiobook_urls_and_malformed_ids() {
        assert!(product_url("https://soundbooththeater.com/shop/audiobooks/test-title/").is_some());
        assert!(product_url("https://soundbooththeater.com/shop/merch/test-title/").is_none());
        assert!(!valid_product_id("../other-site"));
    }
}
