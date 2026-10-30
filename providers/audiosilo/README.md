# AudioSilo metadata provider

BookOrbit audiobook metadata from [AudioSilo Meta](https://meta.audiosilo.app), the public community
catalogue of works, recordings, narrators and series. It runs beside BookOrbit as a small container
and adapts AudioSilo's read-only API to BookOrbit's external provider contract.

Rust: axum, tokio, reqwest with rustls and serde. The image is a static musl binary on `scratch`.

## Run

Beside BookOrbit, on the same Docker network:

```yaml
services:
  audiosilo:
    image: ghcr.io/bookorbit/audiosilo-provider:0.1.0
    restart: unless-stopped
```

Then add `http://audiosilo:8080` in BookOrbit under **Settings > Metadata > Providers > External
providers**. To build it yourself: `docker build -t audiosilo-provider .`, or with a Rust toolchain,
`cargo run --release`.

| Variable | Default | |
|---|---|---|
| `PORT` | `8080` | |
| `AUTH_TOKEN` | unset | When set, `/search` and `/book/{id}` require this exact `Authorization` header |
| `AUDIOSILO_API` | `https://meta.audiosilo.app` | For a compatible deployment or a local test server |

AudioSilo needs no API key and publishes no rate limit. The provider keeps at most four requests to
it in flight, across all callers. AudioSilo answers `503` with `Retry-After` while it loads a new
data release; that and a `429` reach BookOrbit as `429` with `Retry-After`.

The provider sends AudioSilo the title and the ISBN. The author only reorders AudioSilo's answer
inside the provider and is never sent. It stores nothing.

## Endpoints

| Route | |
|---|---|
| `GET /provider/info` | Key `audiosilo`, media `audiobook`, lookup, ISBN search, a URL template for record links |
| `GET /search?query=&author=&isbn=&mediaType=&limit=` | `limit` defaults to 5, at most 10 |
| `GET /book/{id}` | An AudioSilo work slug |

Status codes: `400` no query or ISBN, `401` bad token, `404` unknown id, `429` with `Retry-After`
when AudioSilo throttles or is loading a release, `502` when AudioSilo fails or takes longer than
9 seconds.

## Mapping

- **Medium:** every record is an `audiobook`, whatever `mediaType` asks for. BookOrbit asks with
  `book` for a book that has both an ebook and audio, and ranks the records by medium itself.
- **Ids** are AudioSilo's work slugs, which AudioSilo keeps stable. A slug retired by a merge
  redirects to the work that replaced it, so a stored id answers with the work it now lives in.
  Records carry the work's page as `url`.
- **Search** is AudioSilo's work search on the title, up to `limit`, then each hit's detail. An
  `author` moves that author's works first and filters nothing out. An `isbn` is resolved exactly
  through AudioSilo's lookup and answers the recording it names.
- **One recording per record.** A work can have several narrations; the record is the one an ISBN
  named, else the first. Narrators, publisher, release year, runtime (in seconds), ISBN and cover
  all come from that recording, so they describe the same audiobook. A work not yet recorded keeps
  its title, authors, description, series and genres but has no year or ISBN, since the work's first
  publication and cross-referenced ISBNs belong to the print book.
- **Description** is the community's own-words paragraph when there is one, ending with
  `(Description CC BY-SA 4.0, AudioSilo Meta community)` as its share-alike licence requires, the
  same line AudioSilo's own Audiobookshelf endpoint adds. Otherwise it is the public-domain record's
  description.
- **Genres** read as AudioSilo's own labels ("hard-science-fiction" is "Hard Science Fiction").
  **Series** positions pass through as text, so "2.5" and "1-3" stay distinct. **Language** is the
  work's BCP 47 tag.

Known gaps: no ratings, page counts or chapters. A work's other recordings are not offered as
records of their own. Abridgement is not passed on, since BookOrbit's external contract has no field
for it.

## Test

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Or in a container, with no local toolchain:

```bash
docker run --rm -v "$PWD":/src -w /src rust:1.90-alpine sh -c 'apk add --no-cache musl-dev && cargo test'
```

The fixture in `testdata/` is a real AudioSilo work answer, cut down to the fields the provider
reads.
