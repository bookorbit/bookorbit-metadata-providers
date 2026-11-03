# BookOrbit Kakao 도서 메타데이터 프로바이더

[English](README.md) | [한국어](README.ko.md)

Kakao Daum 도서 검색 API(`GET https://dapi.kakao.com/v3/search/book`)를 사용하는 Rust 기반 BookOrbit 외부 메타데이터 프로바이더입니다.

## 실행

BookOrbit과 동일한 Docker 네트워크에 다음 서비스를 추가합니다.

```yaml
services:
  kakao:
    build: .
    restart: unless-stopped
    environment:
      KAKAO_REST_API_KEY: ${KAKAO_REST_API_KEY}
      # 선택 사항: upstream API 주소 재정의 (기본값은 Kakao 운영 API)
      # KAKAO_API_BASE_URL: https://dapi.kakao.com/v3/search/book
      # upstream 요청 시작 속도 제한: 초당 2건 (0이면 간격 제한 해제)
      # REQUESTS_PER_SECOND: 2
      # 동시에 처리할 upstream HTTP 요청 최대 개수 (기본값 2)
      # MAX_INFLIGHT_REQUESTS: 2
      # 선택 사항: 프로바이더 자체 인증 토큰
      # AUTH_TOKEN: ${BOOKORBIT_PROVIDER_TOKEN}
```

`KAKAO_REST_API_KEY`에는 유효한 Kakao REST API 키를 설정해야 합니다. BookOrbit의 **설정 > 메타데이터 > 프로바이더 > 외부 프로바이더**에 `http://kakao:8080`을 추가합니다.

로컬 실행 및 검증:

```bash
cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## 환경 설정

| 변수 | 기본값 | 설명 |
|---|---|---|
| `PORT` | `8080` | HTTP 서버 포트 |
| `KAKAO_REST_API_KEY` | 필수 | Kakao REST API 키. `Authorization: KakaoAK <key>` 헤더로 전송됩니다. |
| `AUTH_TOKEN` | 미설정 | 설정하면 프로바이더 endpoint 요청의 `Authorization` 헤더에 해당 토큰이 필요합니다. |
| `USER_AGENT` | `bookorbit-kakao-provider/<version>` | 외부 HTTP 요청의 User-Agent |
| `KAKAO_API_BASE_URL` | `https://dapi.kakao.com/v3/search/book` | Kakao API endpoint. 호환 프록시 또는 로컬 Mock 서버를 사용할 때 변경할 수 있습니다. |
| `REQUESTS_PER_SECOND` | `2` | upstream 요청 시작 속도 제한. 검색 요청이 제한을 공유합니다. `1`은 초당 1건, `0`은 시작 간격 제한 해제입니다. |
| `MAX_INFLIGHT_REQUESTS` | `2` | 동시에 진행할 수 있는 upstream HTTP 요청의 최대 개수. 1보다 작은 값은 1로 보정됩니다. |

Kakao REST API 키를 BookOrbit 클라이언트에 노출하거나 로그에 기록하지 마세요.

## API 엔드포인트

| 경로 | 설명 |
|---|---|
| `GET /provider/info` | 프로바이더 메타데이터 및 지원 필드 |
| `GET /search?query=&limit=&author=&target=&sort=&page=` | 도서 검색 |
| `GET /search?isbn=` | ISBN 검색 (`target=isbn` 자동 적용) |
| `GET /book/{id}` | 메모리 캐시에서 이전 검색 결과 조회 |

`limit` 기본값은 5이며 최대 20입니다. `target`은 `title`, `isbn`, `publisher`, `person`을 지원하고, `sort`는 `accuracy`, `latest`를 지원합니다. `page`는 Kakao가 지원하는 1~50 범위로 제한됩니다. Kakao의 `size`는 프로바이더 결과 제한값을 기준으로 설정합니다.

## 메타데이터 매핑

| BookOrbit 필드 | Kakao 필드 | 설명 |
|---|---|---|
| `title` | `title` | 도서 제목 |
| `description` | `contents` | 도서 설명 |
| `url` | `url` | Daum 도서 상세 URL |
| `authors` | `authors` | 저자만 매핑합니다. 번역자는 upstream 응답에서 별도 제공됩니다. |
| `publisher` | `publisher` | 출판사명 |
| `publishedDate` | `datetime` | ISO 8601 timestamp를 `YYYY-MM-DD`로 변환 |
| `publishedYear` | `datetime` | 출간일의 앞 네 자리 연도 |
| `cover` | `thumbnail` | Kakao 표지 썸네일 URL |
| `languageCode` | — | 프로바이더 기본값으로 `ko` 설정 |
| `providerId` | 파생값 | ISBN, URL 또는 대체 메타데이터에서 생성한 SHA-256 16진수(64자). 프로바이더 접두사는 붙이지 않습니다. |

문서화된 Kakao 응답에는 시리즈와 장르를 안정적으로 제공하는 필드가 없어 매핑하지 않습니다. 가격, 판매가, 번역자, 상태는 upstream에서 제공될 수 있지만 현재 BookOrbit 메타데이터 구조에는 포함하지 않습니다.

## 조회 동작 및 제한 사항

Kakao의 문서화된 도서 검색 API에는 ID 전용 조회 endpoint가 없습니다. 프로바이더는 사용 가능한 메타데이터에서 안정적인 ID를 만들고, `/book/{id}` 요청을 처리할 수 있도록 검색 결과를 프로세스 메모리에 캐시합니다. 따라서 재시작 이후이거나 현재 프로세스에서 검색하지 않은 레코드는 조회 시 404가 반환될 수 있습니다. 이는 Kakao API 자체의 기능이 아닌 프로바이더의 명시적인 제한입니다.

검색 1회당 Kakao 요청 1건을 전송합니다. `page`는 받지만 여러 페이지를 자동으로 가져오지는 않습니다. Kakao에는 별도의 `author` 요청 파라미터가 없으므로 저자 필터는 반환된 페이지에 로컬 적용합니다. `target=person`은 upstream 검색 대상 필드로 전달됩니다.

Kakao의 `status` 값은 표시용 문자열이며 변경될 수 있습니다. ISBN은 ISBN-10, ISBN-13, 둘 다 또는 어느 것도 포함하지 않을 수 있습니다. 서로 다른 판본이 별도 결과로 반환될 수 있으며, API 할당량과 가용성은 Kakao가 관리합니다.

## 요청 속도 제한

`REQUESTS_PER_SECOND`는 upstream 요청을 시작하는 속도를 제한합니다. 기본값 `2`는 요청 시작 시점을 최소 500ms 간격으로 유지합니다. `MAX_INFLIGHT_REQUESTS`는 실제 동시에 처리 중인 HTTP 요청 수를 별도로 제한하며 기본값은 2입니다. 두 설정은 독립적으로 적용됩니다.

## 테스트

테스트는 로컬 Mock upstream 서버를 사용해 Provider HTTP route, 캡처된 응답 fixture, 응답 매핑 및 upstream 오류 변환을 검증합니다. Kakao 운영 API를 호출하지 않습니다.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
