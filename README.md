# BookOrbit metadata providers

Extra metadata sources for [BookOrbit](https://github.com/bookorbit/bookorbit), each a small container
that runs beside it, and everything you need to write your own.

BookOrbit calls a provider over HTTP and never runs its code, so a new provider needs no BookOrbit
release.

## Providers

| Provider | What it covers | Image |
|---|---|---|
| [Douban](providers/douban) | Chinese books from [book.douban.com](https://book.douban.com): titles, authors, publishers, ratings, ISBN lookups. No covers, which Douban serves only to its own pages | `ghcr.io/bookorbit/douban-provider` |
| [FantLab](providers/fantlab) | Russian-language science fiction and fantasy from [fantlab.ru](https://fantlab.ru): works, editions by ISBN, series, genres, ratings | `ghcr.io/bookorbit/fantlab-provider` |
| [Graphic Audio](providers/graphicaudio) | Audiobook productions from Graphic Audio: full casts, descriptions, series, release years and ISBNs | `ghcr.io/bookorbit/graphicaudio-provider` |
| [J-Novel Club](providers/jnovelclub) | Light novels and manga published in English by [J-Novel Club](https://j-novel.club), with the volume number taken from the file's title | `ghcr.io/bookorbit/jnovelclub-provider` |
| [Libris](providers/libris) | Swedish books from [Libris](https://libris.kb.se), the National Library of Sweden's catalogue | `ghcr.io/bookorbit/libris-provider` |
| [LubimyCzytac](providers/lubimyczytac) | Polish books from [lubimyczytac.pl](https://lubimyczytac.pl): titles, authors, series, genres | `ghcr.io/bookorbit/lubimyczytac-provider` |
| [MangaBaka](providers/mangabaka) | Manga, manhwa, manhua and light novels from [mangabaka.org](https://mangabaka.org), with the volume number taken from the file's title | `ghcr.io/bookorbit/mangabaka-provider` |
| [Romance.io](providers/romanceio) | Romance books from [Romance.io](https://www.romance.io): community topics, ratings and steam/spice levels | `ghcr.io/bookorbit/romanceio-provider` |
| [Yandex Books](providers/yandexbooks) | Russian books, audiobooks and comics from [Yandex Books](https://books.yandex.ru): descriptions, publishers, series, genres, narrators and running times. No ISBNs, which the site does not show | `ghcr.io/bookorbit/yandexbooks-provider` |

LubimyCzytac used to be built into BookOrbit. It keeps its old key, so the ids already on your books
and the field rules that name it carry on once the provider is added.

Run them on the same Docker network as BookOrbit:

```yaml
services:
  fantlab:
    image: ghcr.io/bookorbit/fantlab-provider:0.1.0
    restart: unless-stopped
  mangabaka:
    image: ghcr.io/bookorbit/mangabaka-provider:0.1.0
    restart: unless-stopped
```

Every provider listens on port 8080. Then in BookOrbit, **Settings > Metadata > Providers >
External providers > Add provider**, with the service name and port as the address, such as
`http://fantlab:8080`. The
[BookOrbit guide](https://github.com/bookorbit/bookorbit/blob/main/docs/METADATA_PROVIDERS.md) walks
through it.

## Write your own

| Folder | |
|---|---|
| [`starters/rust/`](starters/rust) | A complete provider in Rust (axum). Copy it, change `info()` in `main.rs` and `source.rs`, ship it |
| [`starters/go/`](starters/go) | The same provider in Go with no dependencies. Copy it, change `info()` in `main.go` and `source.go`, ship it |
| [`conformance/`](conformance) | Checks a running provider against the contract, no BookOrbit needed |

Both starters serve the same made-up book, pass the same checks, and build into a container image
under a few megabytes. The providers here are Rust because it ran the same provider in a third of
Go's memory (see [MangaBaka](providers/mangabaka#why-rust)); Go builds with nothing but `go run`.
Any language that serves JSON over HTTP works.

The [building guide](https://github.com/bookorbit/bookorbit/blob/main/docs/BUILDING_METADATA_PROVIDERS.md)
covers the contract, recommendations, testing and publishing.

## Releasing a provider

BookOrbit lists the providers in [`directory.json`](directory.json) under **Settings > Metadata >
Providers > External**, with a ready-to-paste Compose snippet. Add a new provider there, and bump its
`version` when you release one, so admins are offered the current image.

Tag the commit `<provider>-v<version>`, such as `mangabaka-v0.2.0`, and push the tag.
[`publish.yml`](.github/workflows/publish.yml) builds the image for `linux/amd64` and `linux/arm64`
and pushes `ghcr.io/bookorbit/<provider>-provider` as `0.2.0`, `0.2` and `latest`. Bump `VERSION` in
the provider's source first: BookOrbit keys its cache on it.

The first time a provider is published, set its package to public under the organisation's
**Packages** settings; GitHub creates new packages private.

## License

[MIT](LICENSE), so a starter can be copied into a provider under any license.
