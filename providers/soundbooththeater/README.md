# Soundbooth Theater metadata provider

BookOrbit audiobook metadata from [Soundbooth Theater](https://soundbooththeater.com), the audiobook
studio, read from its own store. It runs beside BookOrbit as a small container.

Rust: axum, tokio, reqwest with rustls, serde, scraper and regex. The image is a static musl binary
on `scratch`.

## Run

Beside BookOrbit, on the same Docker network:

```yaml
services:
  soundbooththeater:
    image: ghcr.io/bookorbit/soundbooththeater-provider:0.1.0
    restart: unless-stopped
```

Then add `http://soundbooththeater:8080` in BookOrbit under **Settings > Metadata > Providers >
External providers**. To build it yourself: `docker build -t soundbooththeater-provider .`, or with
a Rust toolchain, `cargo run --release`.

| Variable | Default | |
|---|---|---|
| `PORT` | `8080` | |
| `AUTH_TOKEN` | unset | When set, `/search` and `/book/{id}` require this exact `Authorization` header |

Soundbooth Theater has no metadata API, so the provider reads the store's product search and product
pages. It sends one request at a time, at least a second apart, which a search of three matches
takes about four seconds to get through. It sends Soundbooth Theater the title and author, and
stores nothing.

## Endpoints

| Route | |
|---|---|
| `GET /provider/info` | Key `soundbooththeater`, media `audiobook`, lookup, a 1-second `rateLimit`, a URL template for record links, square covers |
| `GET /search?query=&author=&mediaType=&limit=` | `limit` defaults to 3, at most 3 |
| `GET /book/{id}` | A product slug, such as `rickshaw-riot-book-1` |

Status codes: `400` no query, `401` bad token, `404` unknown id, `429` with `Retry-After` when
Soundbooth Theater throttles, `502` when it fails, takes longer than 9 seconds, or answers in a
shape the provider cannot read.

## Mapping

- **Medium:** every record is an `audiobook`, whatever `mediaType` asks for. BookOrbit asks with
  `book` for a book that has both an ebook and audio, and ranks the records by medium itself.
- **Matching:** the store lists matches newest first and also matches descriptions, so its answer
  is reordered by title: the exact title first, then the titles sharing the most words and then
  numbers with the query. "Riftside, Book 1" finds Book 1 ahead of Books 4, 3 and 2. A product
  whose title shares no word with the query is left out.
- **Ids** are the product's slug in `/shop/audiobooks/<slug>/`. Every record is read from its
  product page, so a search and a later lookup of the same id give the same record.
- **Title and series:** "Riftside, Book 1" is the title, with series Riftside at position 1;
  "Series, Book 2: Name" is titled "Name". The page's series link is kept too.
- **Narrators** are the narration and featuring credits, so a full cast is listed in full.
- **Published** is the year of the stated release date. The page's own date is when the store
  listed it, often years later, so it is never used.
- **Publisher** is the stated imprint, else the recording's ℗ holder from the closing credits. The
  © holder published the book, not the audiobook.
- **Description** is the synopsis as plain text, without the closing credits line.
- **Cover** is the product image's full-size original, with its width and height. The page's
  social image is a wide banner, so it is not used.
- **Runtime** is in seconds; **genres** are the page's genre links.

Known gaps: no ISBN, rating or language, which the store does not show. A query matching more than
one page of the store's results is ranked within the first page only.

## Test

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Or in a container, with no local toolchain:

```bash
docker run --rm -v "$PWD":/src -w /src rust:1.90-alpine sh -c 'apk add --no-cache musl-dev && cargo test'
```

The fixtures in `testdata/` are real Soundbooth Theater answers captured on 2026-10-30: the search
answers in full, and the product page cut down to the parts the provider reads.
