use std::time::Duration;

use reqwest::{header::RETRY_AFTER, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::task::JoinSet;

pub const DEFAULT_API: &str = "https://meta.audiosilo.app";
const MAX_LIMIT: usize = 10;

#[derive(Debug)]
pub enum UpstreamError {
    NotFound,
    RateLimited { retry_after_seconds: u64 },
    Failed(String),
}

#[derive(Clone)]
pub struct AudioSilo {
    base: String,
    http: reqwest::Client,
}

impl AudioSilo {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent(format!("bookorbit-audiosilo-provider/{}", crate::VERSION))
            .build()
            .expect("could not build the HTTP client");
        Self {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub async fn search(&self, query: &Query) -> Result<SearchAnswer, UpstreamError> {
        if !query.media_type.is_empty() && query.media_type != "audiobook" {
            return Ok(SearchAnswer::default());
        }

        if !query.isbn.is_empty() {
            let Some(found) = self.lookup_isbn(&query.isbn).await? else {
                return Ok(SearchAnswer::default());
            };
            let work = self.work(&found.work.id).await?;
            return Ok(SearchAnswer {
                matches: vec![map_work(&work, None, Some(&found.recording_id))],
                partial: false,
            });
        }

        if query.title.is_empty() {
            return Ok(SearchAnswer::default());
        }

        let mut url = self.url("/api/v1/works/search")?;
        url.query_pairs_mut()
            .append_pair("q", &query.title)
            .append_pair("limit", &query.limit.clamp(1, MAX_LIMIT).to_string());
        let response: SearchResults = self.get_json(url).await?;
        let mut hits = response.results;
        let author = query.author.to_lowercase();
        if !author.is_empty() {
            hits.sort_by_key(|hit| {
                !hit.authors
                    .iter()
                    .any(|candidate| candidate.name.to_lowercase().contains(&author))
            });
        }
        let hit_count = hits.len();
        let mut tasks = JoinSet::new();
        for (index, hit) in hits.into_iter().enumerate() {
            let source = self.clone();
            tasks.spawn(async move {
                let result = source.work(&hit.id).await;
                (index, hit, result)
            });
        }

        let mut found: Vec<Option<(WorkCard, WorkDetail)>> = (0..hit_count).map(|_| None).collect();
        let mut partial = false;
        let mut failure = None;
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((index, hit, Ok(work))) => found[index] = Some((hit, work)),
                Ok((_, _, Err(UpstreamError::NotFound))) => partial = true,
                Ok((_, _, Err(error))) => {
                    partial = true;
                    failure.get_or_insert(error);
                }
                Err(error) => {
                    partial = true;
                    failure.get_or_insert(UpstreamError::Failed(error.to_string()));
                }
            }
        }

        let found: Vec<(WorkCard, WorkDetail)> = found.into_iter().flatten().collect();
        if found.is_empty() && partial {
            return Err(failure
                .unwrap_or_else(|| UpstreamError::Failed("no work details could be read".into())));
        }
        Ok(SearchAnswer {
            matches: found
                .into_iter()
                .map(|(hit, work)| map_work(&work, Some(&hit.narrators), None))
                .collect(),
            partial,
        })
    }

    pub async fn lookup(&self, id: &str) -> Result<Option<BookMetadata>, UpstreamError> {
        match self.work(id).await {
            Ok(work) => Ok(Some(map_work(&work, None, None))),
            Err(UpstreamError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn lookup_isbn(&self, isbn: &str) -> Result<Option<LookupResult>, UpstreamError> {
        let clean: String = isbn
            .chars()
            .filter(|character| !character.is_whitespace() && *character != '-')
            .collect();
        if clean.is_empty() {
            return Ok(None);
        }
        let mut url = self.url("/api/v1/lookup")?;
        url.query_pairs_mut().append_pair("isbn", &clean);
        match self.get_json(url).await {
            Ok(result) => Ok(Some(result)),
            Err(UpstreamError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn work(&self, id: &str) -> Result<WorkDetail, UpstreamError> {
        let mut url = self.url("/api/v1/works/")?;
        url.path_segments_mut()
            .map_err(|_| UpstreamError::Failed("invalid AudioSilo API URL".into()))?
            .pop_if_empty()
            .push(id);
        self.get_json(url).await
    }

    fn url(&self, path: &str) -> Result<Url, UpstreamError> {
        Url::parse(&format!("{}{}", self.base, path))
            .map_err(|error| UpstreamError::Failed(error.to_string()))
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T, UpstreamError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|error| UpstreamError::Failed(error.without_url().to_string()))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            return Err(UpstreamError::NotFound);
        }
        if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
            let seconds = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
                .unwrap_or(30);
            return Err(UpstreamError::RateLimited {
                retry_after_seconds: seconds,
            });
        }
        if !status.is_success() {
            return Err(UpstreamError::Failed(format!(
                "AudioSilo answered {status}"
            )));
        }
        response
            .json()
            .await
            .map_err(|error| UpstreamError::Failed(error.without_url().to_string()))
    }
}

#[derive(Debug, Default)]
pub struct Query {
    pub title: String,
    pub author: String,
    pub isbn: String,
    pub media_type: String,
    pub limit: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct SearchAnswer {
    pub matches: Vec<BookMetadata>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

#[derive(Debug, Default, Serialize)]
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn13: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub isbn10: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub duration: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<Series>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub language_code: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Series {
    pub series: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sequence: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Deserialize)]
struct SearchResults {
    #[serde(default)]
    results: Vec<WorkCard>,
}

#[derive(Debug, Default, Deserialize)]
struct WorkCard {
    #[serde(default)]
    id: String,
    #[serde(default)]
    authors: Vec<PersonRef>,
    #[serde(default)]
    narrators: Vec<PersonRef>,
}

#[derive(Debug, Deserialize)]
struct LookupResult {
    work: WorkCard,
    #[serde(default)]
    recording_id: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct PersonRef {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SeriesRef {
    name: String,
    position: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkXref {
    isbn: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CommunityDescription {
    text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Recording {
    id: String,
    narrators: Vec<PersonRef>,
    runtime_min: Option<u32>,
    release_date: String,
    publisher: String,
    isbn: Vec<String>,
    cover_url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkDetail {
    id: String,
    title: String,
    subtitle: String,
    authors: Vec<PersonRef>,
    language: String,
    first_published: String,
    description: String,
    genres: Vec<String>,
    series: Vec<SeriesRef>,
    xref: Option<WorkXref>,
    cover_url: Option<String>,
    recordings: Option<Vec<Recording>>,
    community_description: Option<CommunityDescription>,
}

fn map_work(
    work: &WorkDetail,
    search_narrators: Option<&[PersonRef]>,
    recording_id: Option<&str>,
) -> BookMetadata {
    let recordings = work.recordings.as_deref().unwrap_or_default();
    let first = recording_id
        .and_then(|id| recordings.iter().find(|recording| recording.id == id))
        .or(recordings.first());
    let recording_narrators: Vec<PersonRef> = recordings
        .iter()
        .flat_map(|recording| recording.narrators.iter().cloned())
        .collect();
    let narrator_names = unique_names(if recording_narrators.is_empty() {
        search_narrators.unwrap_or_default()
    } else {
        &recording_narrators
    });
    let isbn = first
        .and_then(|recording| recording.isbn.first())
        .or_else(|| work.xref.as_ref().and_then(|xref| xref.isbn.first()))
        .cloned()
        .unwrap_or_default();
    let release_date = first
        .map(|recording| recording.release_date.as_str())
        .filter(|date| !date.is_empty())
        .unwrap_or(&work.first_published);
    let description = match &work.community_description {
        Some(community) if !community.text.is_empty() => format!(
            "{}\n\n(Description CC BY-SA 4.0, AudioSilo Meta community)",
            community.text
        ),
        _ => work.description.clone(),
    };

    BookMetadata {
        media_type: "audiobook".into(),
        provider_id: work.id.clone(),
        title: work.title.clone(),
        subtitle: work.subtitle.clone(),
        authors: unique_names(&work.authors),
        narrators: narrator_names,
        publisher: first
            .map(|recording| recording.publisher.clone())
            .unwrap_or_default(),
        published_year: year(release_date),
        description,
        cover: first
            .map(|recording| recording.cover_url.clone())
            .filter(|cover| !cover.is_empty())
            .or_else(|| work.cover_url.clone())
            .unwrap_or_default(),
        isbn: isbn.clone(),
        isbn13: if isbn.len() == 13 {
            isbn.clone()
        } else {
            String::new()
        },
        isbn10: if isbn.len() == 10 {
            isbn
        } else {
            String::new()
        },
        duration: first
            .and_then(|recording| recording.runtime_min)
            .unwrap_or_default()
            .saturating_mul(60),
        genres: work.genres.iter().map(|genre| genre_label(genre)).collect(),
        series: work
            .series
            .iter()
            .map(|entry| Series {
                series: entry.name.clone(),
                sequence: entry.position.clone(),
            })
            .collect(),
        language_code: work.language.clone(),
        url: format!("{DEFAULT_API}/works/{}", work.id),
    }
}

fn unique_names(people: &[PersonRef]) -> Vec<String> {
    let mut names = Vec::new();
    for person in people {
        let name = person.name.trim();
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn year(date: &str) -> String {
    date.get(..4)
        .filter(|prefix| prefix.bytes().all(|byte| byte.is_ascii_digit()))
        .unwrap_or_default()
        .to_string()
}

fn genre_label(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_real_audiosilo_work_response() {
        let work: WorkDetail = serde_json::from_str(include_str!(
            "../testdata/a-killer-coffee-mystery-box-set.json"
        ))
        .unwrap();
        let mapped = map_work(&work, None, None);

        assert_eq!(
            mapped.provider_id,
            "a-killer-coffee-mystery-box-set-books-1-3"
        );
        assert_eq!(mapped.authors, ["Tonya Kappes"]);
        assert_eq!(mapped.narrators, ["Ariel Hicks", "Madeline Mrozek"]);
        assert_eq!(mapped.duration, 51_360);
        assert_eq!(mapped.published_year, "2023");
        assert_eq!(mapped.publisher, "Tonya Kappes Books");
        assert_eq!(
            mapped.genres,
            [
                "Cozy Mystery",
                "Food Cooking",
                "Mystery",
                "Nature",
                "Short Stories"
            ]
        );
        assert_eq!(mapped.series[0].sequence, "1-3");
        assert_eq!(
            mapped.cover,
            "https://m.media-amazon.com/images/I/81TI3RvP2NL.jpg"
        );
    }

    #[test]
    fn maps_work_metadata_and_attributes_community_description() {
        let work: WorkDetail = serde_json::from_value(serde_json::json!({
            "id": "the-example-book",
            "title": "The Example Book",
            "subtitle": "A test subtitle",
            "authors": [{"id": "author-one", "name": "A. Writer"}],
            "language": "en",
            "first_published": "2020",
            "description": "Public-domain description",
            "genres": ["hard-science-fiction"],
            "series": [{"id": "example-series", "name": "Example Series", "position": "2.5"}],
            "xref": {"isbn": ["9781234567890"]},
            "cover_url": "https://example.test/work-cover.jpg",
            "community_description": {"text": "A community description.", "license": "CC-BY-SA-4.0"},
            "recordings": [{
                "id": "other-recording",
                "narrators": [{"id": "other-narrator", "name": "Other Reader"}],
                "runtime_min": 30,
                "release_date": "2001-01-01",
                "publisher": "Other Audio",
                "isbn": ["0123456789"],
                "cover_url": "https://example.test/other-cover.jpg",
                "chapter_count": 4
            }, {
                "id": "reader-one",
                "narrators": [{"id": "narrator-one", "name": "N. Reader"}],
                "runtime_min": 321,
                "release_date": "2024-06-02",
                "publisher": "Example Audio",
                "isbn": ["9781234567890"],
                "asin": [{"region": "us", "asin": "B000000001"}],
                "cover_url": "https://example.test/audio-cover.jpg",
                "chapter_count": 12
            }]
        })).unwrap();

        let mapped = map_work(&work, None, Some("reader-one"));
        assert_eq!(mapped.provider_id, "the-example-book");
        assert_eq!(mapped.media_type, "audiobook");
        assert_eq!(mapped.authors, ["A. Writer"]);
        assert_eq!(mapped.narrators, ["Other Reader", "N. Reader"]);
        assert_eq!(mapped.duration, 19_260);
        assert_eq!(mapped.published_year, "2024");
        assert_eq!(mapped.isbn13, "9781234567890");
        assert_eq!(mapped.cover, "https://example.test/audio-cover.jpg");
        assert_eq!(mapped.genres, ["Hard Science Fiction"]);
        assert_eq!(mapped.series[0].sequence, "2.5");
        assert!(mapped
            .description
            .contains("(Description CC BY-SA 4.0, AudioSilo Meta community)"));
    }

    #[test]
    fn maps_recordless_work_and_uses_core_description() {
        let work: WorkDetail = serde_json::from_value(serde_json::json!({
            "id": "not-yet-recorded",
            "title": "Not Yet Recorded",
            "authors": [],
            "language": "en",
            "first_published": "1999-01-01",
            "description": "Core description",
            "recordings": null
        }))
        .unwrap();

        let mapped = map_work(&work, None, None);
        assert_eq!(mapped.published_year, "1999");
        assert_eq!(mapped.description, "Core description");
        assert_eq!(mapped.duration, 0);
    }
}
