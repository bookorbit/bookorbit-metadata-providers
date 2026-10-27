# AudioSilo metadata provider

BookOrbit audiobook metadata from [AudioSilo Meta](https://meta.audiosilo.app), the public community
catalogue. This container adapts AudioSilo's read-only API to BookOrbit's external provider contract.

## Run

```yaml
services:
  audiosilo:
    image: ghcr.io/bookorbit/audiosilo-provider:0.1.0
    restart: unless-stopped
```

Add `http://audiosilo:8080` in BookOrbit under **Settings > Metadata > Providers > External
providers**. To build it yourself, run `docker build -t audiosilo-provider .` from this directory or
`cargo run --release` with a Rust toolchain.

| Variable | Default | |
|---|---|---|
| `PORT` | `8080` | Provider listen port |
| `AUDIOSILO_API` | `https://meta.audiosilo.app` | AudioSilo API origin, useful for a compatible deployment or local test server |

## Metadata

Search uses AudioSilo's work search and reads each matching work's detail. The title searches the
catalogue; matching authors are boosted without filtering out other candidates. Provider IDs are
stable AudioSilo work slugs, so BookOrbit can look a record up again without repeating a search.
ISBN searches resolve exactly through AudioSilo's lookup endpoint. Results are audiobooks; book-only
requests return no matches.

Authors, narrators, the first recording's publisher, release year, runtime, cover and ISBNs map from
the work and its recordings. Runtime is returned in seconds. AudioSilo genre slugs are rendered as
human-readable labels, and series positions pass through as strings. If a work has a community
description, the response appends `(Description CC BY-SA 4.0, AudioSilo Meta community)` as required
for attribution; otherwise it uses the public-domain core description.

AudioSilo's API is public and read-only; this provider needs no credentials. It does not store a
local catalogue or cache metadata.