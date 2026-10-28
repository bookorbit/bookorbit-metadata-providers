# Graphic Audio metadata provider

BookOrbit audiobook metadata from the Graphic Audio catalog. This provider reads the public JSON
catalogs published by [graphicaudio_scraper](https://github.com/binyaminyblatt/graphicaudio_scraper)
and does not scrape GraphicAudio product pages.

## Run

```yaml
services:
  graphicaudio:
    image: ghcr.io/bookorbit/graphicaudio-provider:0.1.0
    restart: unless-stopped
```

Add `http://graphicaudio:8080` in BookOrbit under **Settings > Metadata > Providers > External
providers**. To build it yourself, run `docker build -t graphicaudio-provider .` from this directory
or `cargo run --release` with a Rust toolchain.

| Variable | Default | |
|---|---|---|
| `PORT` | `8080` | Provider listen port |
| `AUTH_TOKEN` | unset | When set, `/search` and `/book/{id}` require this exact `Authorization` header |

## Metadata

The provider downloads the current and archive catalogs, merges them by stable record ID, and keeps
the result in memory for 24 hours. If a refresh fails, it serves the last in-memory catalog when
available; if neither feed can be loaded on a cold start, it reports an upstream error rather than
pretending there are no matches.

Search matches title, raw title, or series name; matching authors are ranked first without hiding
other title matches. ISBN lookups are exact. Provider IDs prefer ISBN, then ASIN, then the catalog
product slug so older records without identifiers can still be refreshed. Cast members are returned
as narrators (up to 10), and runtimes are not provided because the catalog does not contain them.
The archive feed concatenates unrelated series names, so series metadata is omitted for those older
records rather than returning a misleading series.

This provider sends title, author, and ISBN search values only to its public catalog source. It does
not store a local catalog file or send other library or user data. Set `AUTH_TOKEN` if this service is
reachable beyond a private network, and enter the same value in BookOrbit's Authorization header
field.