use std::cmp::Reverse;
use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use reqwest::Url;
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;

use crate::soundbooth::SITE;

static PRODUCT_CARDS: LazyLock<Selector> = LazyLock::new(|| selector("li.product"));
static PRODUCT_LINK: LazyLock<Selector> =
    LazyLock::new(|| selector("a.woocommerce-LoopProduct-link"));
static PRODUCT_TITLE: LazyLock<Selector> =
    LazyLock::new(|| selector(".woocommerce-loop-product__title"));
static DETAIL_TITLE: LazyLock<Selector> = LazyLock::new(|| selector("h1.product_title"));
static GALLERY_IMAGE: LazyLock<Selector> =
    LazyLock::new(|| selector(".woocommerce-product-gallery__image img"));
static SYNOPSIS: LazyLock<Selector> = LazyLock::new(|| selector(".synopsis.readmore"));
static DESCRIPTION_TAB: LazyLock<Selector> = LazyLock::new(|| selector("#tab-description"));
static SHORT_DESCRIPTION: LazyLock<Selector> =
    LazyLock::new(|| selector(".woocommerce-product-details__short-description"));
static PARAGRAPH: LazyLock<Selector> = LazyLock::new(|| selector("p"));
static AUTHOR_LINKS: LazyLock<Selector> = LazyLock::new(|| selector(".summary h3 a"));
static AUDIOBOOK_ITEMS: LazyLock<Selector> = LazyLock::new(|| selector("ul.audiobook-meta li"));
static LABEL: LazyLock<Selector> = LazyLock::new(|| selector("span"));
static LINKS: LazyLock<Selector> = LazyLock::new(|| selector("a"));
static OG_DESCRIPTION: LazyLock<Selector> =
    LazyLock::new(|| selector("meta[property='og:description']"));
static OG_TITLE: LazyLock<Selector> = LazyLock::new(|| selector("meta[property='og:title']"));
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

/// Words that say nothing about which book a title names.
const FILLER_WORDS: [&str; 7] = ["a", "an", "and", "the", "of", "book", "episode"];

fn selector(pattern: &str) -> Selector {
    Selector::parse(pattern).expect("valid CSS selector")
}

#[derive(Debug, Clone)]
pub struct ProductHit {
    pub provider_id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadata {
    pub media_type: String,
    pub provider_id: String,
    pub title: String,
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

pub fn parse_search_results(html: &str) -> Vec<ProductHit> {
    let document = Html::parse_fragment(html);
    document
        .select(&PRODUCT_CARDS)
        .filter_map(|product| {
            let href = product.select(&PRODUCT_LINK).next()?.value().attr("href")?;
            let (provider_id, url) = product_url(href)?;
            let title = product
                .select(&PRODUCT_TITLE)
                .next()
                .map(element_text)
                .filter(|title| !title.is_empty())?;
            Some(ProductHit {
                provider_id,
                url,
                title,
            })
        })
        .collect()
}

/// Soundbooth Theater lists matches newest first and also matches the query against descriptions,
/// so "Riftside, Book 1" comes after Books 4, 3 and 2, and a title the store does not sell finds
/// books whose blurbs mention it. Hits are ranked by their own titles instead: an exact title
/// first, whole or the name after "Series, Book 1:", then by the words and then the numbers they
/// share with it, then by the fewest words it lacks. A hit sharing no word with it is left out.
pub fn rank_hits(hits: Vec<ProductHit>, query: &str, limit: usize) -> Vec<ProductHit> {
    let wanted = TitleWords::of(query);
    let mut ranked: Vec<_> = hits
        .into_iter()
        .enumerate()
        .filter_map(|(index, hit)| {
            let found = TitleWords::of(&hit.title);
            let (name, _) = split_series_title(&hit.title);
            let exact = found.all == wanted.all || TitleWords::of(&name).all == wanted.all;
            let words = found.words.intersection(&wanted.words).count();
            let numbers = found.numbers.intersection(&wanted.numbers).count();
            let extra = found.words.len() - words;
            (exact || words > 0).then_some((
                Reverse(exact),
                Reverse(words),
                Reverse(numbers),
                extra,
                index,
                hit,
            ))
        })
        .collect();
    ranked.sort_by_key(|(exact, words, numbers, extra, index, _)| {
        (*exact, *words, *numbers, *extra, *index)
    });
    ranked
        .into_iter()
        .take(limit)
        .map(|(.., hit)| hit)
        .collect()
}

struct TitleWords {
    all: Vec<String>,
    words: HashSet<String>,
    numbers: HashSet<String>,
}

impl TitleWords {
    fn of(title: &str) -> Self {
        let all: Vec<String> = title
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect();
        let (numbers, words) = all
            .iter()
            .filter(|word| !FILLER_WORDS.contains(&word.as_str()))
            .cloned()
            .partition(|word| word.bytes().all(|byte| byte.is_ascii_digit()));
        Self {
            all,
            words,
            numbers,
        }
    }
}

/// Everything comes from the product page, so a search and a later lookup of the same id give the
/// same record.
pub fn map_product(provider_id: &str, url: &str, detail: &Html) -> BookMetadata {
    let raw_title = detail
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
    let (title, series) = split_series_title(&raw_title);
    let mut book = BookMetadata {
        media_type: "audiobook".into(),
        provider_id: provider_id.to_string(),
        title,
        series,
        url: url.to_string(),
        ..BookMetadata::default()
    };

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
                    let name = item_value(item);
                    if !name.is_empty() {
                        book.narrators.push(name);
                    }
                } else {
                    book.narrators.extend(linked);
                }
            }
            "length" => book.duration = parse_duration(&item_value(item)),
            // The page's own publication date is when the store listed it, often years after
            // the release, so the year comes only from the stated release date.
            "release date" => book.published_year = year_in(&item_value(item)),
            "imprint" => book.publisher = item_value(item),
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
    book.genres = unique(book.genres);

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
    let (description, audio_publisher) = split_copyright(&description);
    book.description = description;
    if book.publisher.is_empty() {
        book.publisher = audio_publisher;
    }

    if let Some((cover, width, height)) = gallery_cover(detail) {
        book.cover = cover;
        book.cover_width = width;
        book.cover_height = height;
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

/// The product image's full-size original. The page's `og:image` is a wide social banner, not the
/// cover.
fn gallery_cover(detail: &Html) -> Option<(String, u32, u32)> {
    let image = detail.select(&GALLERY_IMAGE).next()?;
    let attr = |name: &str| image.value().attr(name).filter(|value| !value.is_empty());
    let number = |name: &str| {
        attr(name)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default()
    };
    match attr("data-large_image") {
        Some(url) => Some((
            url.to_string(),
            number("data-large_image_width"),
            number("data-large_image_height"),
        )),
        None => attr("src").map(|url| (url.to_string(), number("width"), number("height"))),
    }
}

/// "Series, Book 2" keeps its title; "Series, Book 2: Name" is titled "Name". Either way the
/// series and its position come out of the title.
fn split_series_title(raw: &str) -> (String, Vec<Series>) {
    let raw = raw.trim();
    let Some(captures) = TITLE_SERIES.captures(raw) else {
        return (raw.to_string(), Vec::new());
    };
    let series = captures.get(1).map_or("", |value| value.as_str()).trim();
    let sequence = captures.get(2).map_or("", |value| value.as_str()).trim();
    let name = captures.get(3).map_or("", |value| value.as_str()).trim();
    let title = if name.is_empty() { raw } else { name };
    let series = if series.is_empty() {
        Vec::new()
    } else {
        vec![Series {
            series: series.to_string(),
            sequence: sequence.to_string(),
        }]
    };
    (title.to_string(), series)
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

/// Splits the closing credits line ("© 2026 Book Publisher ℗ 2026 Soundbooth Theater") off a
/// description. The ℗ holder made the recording, so it is the audiobook's publisher; the ©
/// holder published the book.
fn split_copyright(raw: &str) -> (String, String) {
    let Some((description, credits)) = raw.split_once('\u{00a9}') else {
        return (raw.to_string(), String::new());
    };
    let recording = credits
        .split_once('\u{2117}')
        .map_or("", |(_, holder)| holder)
        .split_whitespace()
        .collect::<Vec<_>>();
    let recording = match recording.split_first() {
        Some((year, rest)) if year.len() == 4 && year.bytes().all(|byte| byte.is_ascii_digit()) => {
            rest.join(" ")
        }
        _ => recording.join(" "),
    };
    (description.trim().to_string(), recording)
}

/// The four-digit year in a date the store writes as `07/14/2026`.
fn year_in(date: &str) -> String {
    date.split(|character: char| !character.is_ascii_digit())
        .find(|part| part.len() == 4)
        .unwrap_or_default()
        .to_string()
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

/// What follows the label in `<li><span>Imprint: </span>Soundbooth Theater</li>`.
fn item_value(item: ElementRef<'_>) -> String {
    let text = element_text(item);
    text.split_once(':')
        .map_or(text.as_str(), |(_, value)| value)
        .trim()
        .to_string()
}

/// Text pieces are joined as they stand, so "<i>Carl</i>, <i>Dan</i>" reads "Carl, Dan"; a `<br>`
/// still keeps the words on either side apart.
fn element_text(element: ElementRef<'_>) -> String {
    let mut text = String::new();
    for node in element.descendants() {
        if let Some(fragment) = node.value().as_text() {
            text.push_str(fragment);
        } else if node
            .value()
            .as_element()
            .is_some_and(|tag| tag.name() == "br")
        {
            text.push(' ');
        }
    }
    normalize(&text)
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

    const DETAIL: &str = include_str!("../testdata/detail-rickshaw-riot.html");

    fn hits(raw: &str) -> Vec<ProductHit> {
        let answer: serde_json::Value = serde_json::from_str(raw).unwrap();
        parse_search_results(answer["posts"].as_str().unwrap())
    }

    fn riftside_hits() -> Vec<ProductHit> {
        hits(include_str!("../testdata/search-riftside.json"))
    }

    #[test]
    fn parses_every_card_of_a_real_search_answer() {
        let hits = riftside_hits();
        let titles: Vec<&str> = hits.iter().map(|hit| hit.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Riftside, Book 4",
                "Riftside, Book 3",
                "Riftside, Book 2",
                "Riftside, Book 1"
            ]
        );
        assert_eq!(hits[3].provider_id, "riftside-book-1");
        assert_eq!(
            hits[3].url,
            "https://soundbooththeater.com/shop/audiobooks/riftside-book-1/"
        );
    }

    #[test]
    fn ranks_the_named_volume_first_within_the_limit() {
        let ranked = rank_hits(riftside_hits(), "Riftside, Book 1", 3);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].provider_id, "riftside-book-1");
        let ranked = rank_hits(riftside_hits(), "Riftside 2", 3);
        assert_eq!(ranked[0].provider_id, "riftside-book-2");
    }

    #[test]
    fn a_volume_named_like_the_query_beats_newer_volumes() {
        let life_reset = hits(include_str!("../testdata/search-life-reset.json"));
        assert_eq!(life_reset.len(), 6);
        let ranked = rank_hits(life_reset, "Life Reset", 3);
        assert_eq!(ranked[0].provider_id, "life-reset-a-litrpg-novel");
    }

    #[test]
    fn leaves_out_hits_sharing_no_title_word() {
        assert!(rank_hits(riftside_hits(), "He Who Fights with Monsters", 3).is_empty());
    }

    #[test]
    fn maps_a_real_product_page() {
        let detail = Html::parse_document(DETAIL);
        let book = map_product(
            "rickshaw-riot-book-1",
            "https://soundbooththeater.com/shop/audiobooks/rickshaw-riot-book-1/",
            &detail,
        );
        assert_eq!(book.media_type, "audiobook");
        assert_eq!(book.title, "Rickshaw Riot, Book 1");
        assert_eq!(
            book.series,
            [Series {
                series: "Rickshaw Riot".into(),
                sequence: "1".into()
            }]
        );
        assert_eq!(book.authors, ["Ben Wolf", "Luke Messa"]);
        assert_eq!(
            book.narrators,
            [
                "Ryan H. Reid",
                "Gary Furlong",
                "Jeff Hays",
                "Annie Ellicott",
                "Rebecca Brokaw-Sands",
                "Johnathan McClain"
            ]
        );
        assert_eq!(book.duration, Some(35_760));
        assert_eq!(book.publisher, "Soundbooth Theater");
        assert_eq!(book.published_year, "2026");
        assert_eq!(book.genres, ["Humor", "LitRPG", "Science Fiction"]);
        assert!(book.cover.ends_with("/RickshawRiot_Cover.jpg"));
        assert_eq!((book.cover_width, book.cover_height), (1024, 1024));
        assert!(book.description.starts_with("Video game mogul, Erik Shaw,"));
        assert!(book
            .description
            .contains("fans of Dungeon Crawler Carl, Discount Dan, and He Who Fights"));
        assert!(!book.description.contains('\u{00a9}'));
    }

    #[test]
    fn a_named_volume_is_titled_by_its_name() {
        let (title, series) = split_series_title("The Feedback Loop, Book 3: High Fantasy");
        assert_eq!(title, "High Fantasy");
        assert_eq!(series[0].series, "The Feedback Loop");
        assert_eq!(series[0].sequence, "3");
    }

    #[test]
    fn the_recording_holder_is_the_fallback_publisher() {
        let (description, publisher) = split_copyright(
            "A blurb long enough to keep.\n\n\u{00a9} 2017 A Book House \u{2117} 2020 Soundbooth Theater",
        );
        assert_eq!(description, "A blurb long enough to keep.");
        assert_eq!(publisher, "Soundbooth Theater");
    }

    #[test]
    fn a_line_break_keeps_words_apart() {
        let fragment = Html::parse_fragment("<p>First line<br>second <i>line</i>, done</p>");
        let paragraph = fragment.select(&PARAGRAPH).next().unwrap();
        assert_eq!(element_text(paragraph), "First line second line, done");
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
