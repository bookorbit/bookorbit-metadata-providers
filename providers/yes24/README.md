# BookOrbit YES24 Metadata Provider

[English](README.md) | [한국어](README.ko.md)

A Rust-based external metadata provider for BookOrbit. It uses the YES24 Open API for product search and product detail retrieval.

- Product search: `GET https://apis.yes24.com/v1/goods/itemList`
- Product detail: `GET https://apis.yes24.com/v1/goods/itemDetail`
- Authentication: `X-Api-Key: <YES24_API_KEY>`
- API docs: https://developers.yes24.com/api-doc/goods-item-list
- API docs: https://developers.yes24.com/api-doc/goods-item-detail

## Configuration

Required environment variable:

- `YES24_API_KEY`: API key issued by YES24 Developers.

Optional environment variables:

- `PORT`: HTTP server port. Default: `8080`.
- `AUTH_TOKEN`: When set, incoming BookOrbit requests must include the same value in the `Authorization` header.
- `USER_AGENT`: User-Agent sent to the YES24 API. Defaults to a value based on the package version.
- `YES24_API_BASE_URL`: Base URL for the YES24 goods API. Default: `https://apis.yes24.com/v1/goods`. Override it to use a compatible proxy or local mock server.
- `REQUESTS_PER_SECOND`: Maximum upstream request start rate. Default: 2 requests per second (500 ms between request starts); `0` disables pacing.
- `MAX_INFLIGHT_REQUESTS`: Maximum number of concurrent upstream HTTP requests. Default: `2`; minimum: `1`.

Keep the YES24 API key inside the provider container. Do not expose it to BookOrbit clients.

## Run

```bash
YES24_API_KEY=your_api_key cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## Docker

Build and run the image:

```bash
docker build -t bookorbit-yes24-provider:latest .
docker run -d --name bookorbit-yes24-provider \
  --restart unless-stopped -p 8081:8080 \
  -e YES24_API_KEY=your_api_key \
  bookorbit-yes24-provider:latest
```

Example service definition for an existing BookOrbit Compose project:

```yaml
yes24-provider:
  container_name: bookorbit-yes24-provider
  image: bookorbit-yes24-provider:latest
  restart: unless-stopped
  init: true
  environment:
    YES24_API_KEY: ${YES24_API_KEY?required}
    # Default: https://apis.yes24.com/v1/goods
    # YES24_API_BASE_URL: https://apis.yes24.com/v1/goods
    # Upstream request start rate (default 2/sec; 0 disables pacing)
    # REQUESTS_PER_SECOND: 2
    # Maximum concurrent upstream HTTP requests (default 2, minimum 1)
    # MAX_INFLIGHT_REQUESTS: 2
    # Optional: protect requests coming into this provider from BookOrbit
    # AUTH_TOKEN: ${BOOKORBIT_PROVIDER_TOKEN}
  ports:
    - "8081:8080"
```

If BookOrbit and this provider share a Compose network, configure the external provider URL in BookOrbit as `http://bookorbit-yes24-provider:8080`. Add `YES24_API_KEY` to the `.env` file in the Compose working directory or use your existing environment-variable management method.

## Endpoints

| Route | Description |
|---|---|
| `GET /provider/info` | BookOrbit provider metadata |
| `GET /search?query=clean-code` | Search products |
| `GET /search?isbn=9788966260959` | Search by ISBN |
| `GET /book/12345678` | Retrieve details by YES24 item ID |

Search parameters:

- `query`: Required search term.
- `isbn`: Used as the ISBN search term when `query` is omitted.
- `target`: `title`, `isbn`, `publisher` or `person`. The YES24 search API does not expose an equivalent target field. ISBN-13 uses the detail API; publisher and author filters are applied locally to returned search results.
- `author`: Filter search results by author string.
- `sort`: `accuracy` (YES24 `DEFAULT`) or `latest` (YES24 `RECENT`).
- `page`: Page number, starting at 1.
- `limit`: Number of results. Default: 5; maximum: 20.
- `category`: YES24 category. Default: `BOOK`; accepted values: `ALL`, `BOOK`, `FOREIGN`, `EBOOK`, `MUSIC`, `DVD`.

Search requests set `detail=Y` to map the extended information YES24 makes available. Product detail lookup calls YES24's ItemId detail API directly rather than relying on the search-result cache, so it continues to work after a provider restart.

## Metadata mapping and limitations

- `providerId`: `<ItemId>`.
- `title`: Product title; when a subtitle exists, the format is `Title: Subtitle`.
- `authors`: The single YES24 author string is converted into a list.
- `description`: Book introduction, synopsis and table of contents are combined with section labels.
- `cover`: Cover image URL.
- `publishedDate` / `publishedYear`: Extracted from the publication date.
- `series`: Series name mapped to the BookOrbit series format.
- `url`: YES24 product URL.
- `language`: YES24 documentation does not provide a language field, so `ko` is used as the default, matching the existing Kakao provider behavior.

## Request rate limiting

`REQUESTS_PER_SECOND` limits the start rate of upstream requests shared by search and detail operations. The default value `2` keeps request starts at least 500 ms apart. `MAX_INFLIGHT_REQUESTS` independently limits the number of requests being processed concurrently; its default is `2`. Setting `REQUESTS_PER_SECOND=0` disables only the start-rate pacing. `MAX_INFLIGHT_REQUESTS` has a minimum value of 1.

## Tests

Tests use a local Axum mock server to exercise the full Provider Router → HTTP Client → Mock upstream path. They do not call the YES24 production API.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
