# Soundbooth Theater metadata provider

BookOrbit audiobook metadata from [Soundbooth Theater](https://soundbooththeater.com). The provider
uses the store's WordPress product search and product pages; it has no official public metadata API.

## Run

```yaml
services:
  soundbooththeater:
    image: ghcr.io/bookorbit/soundbooththeater-provider:0.1.0
    restart: unless-stopped
```

Add `http://soundbooththeater:8080` in BookOrbit under **Settings > Metadata > Providers > External
providers**. To build it yourself, run `docker build -t soundbooththeater-provider .` from this
directory or `cargo run --release` with a Rust toolchain.

| Variable | Default | |
|---|---|---|
| `PORT` | `8080` | Provider listen port |
| `AUTH_TOKEN` | unset | When set, `/search` and `/book/{id}` require this exact `Authorization` header |

## Metadata

Search sends the title and author together to Soundbooth Theater's AJAX product search. It reads a
small number of audiobook product pages for authors, narrators, series, duration, publisher,
description and release year. Runtime is returned in seconds. Product IDs are stable product slugs,
so BookOrbit can refresh a result through `/book/{id}`.

This provider depends on Soundbooth Theater's WordPress storefront rather than a documented metadata
API. Its markup and AJAX behavior may change; a source failure is reported as an error rather than
being cached as an empty search. The provider paces requests and does not store a local catalogue or
cache product details.

The provider sends the title and author supplied by BookOrbit to Soundbooth Theater. It sends no
other library or user data. Set `AUTH_TOKEN` if this service is reachable beyond a private network,
and enter the same value in BookOrbit's Authorization header field.