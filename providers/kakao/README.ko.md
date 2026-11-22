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

| 변수                    | 기본값                                  | 설명                                                                                                          |
| ----------------------- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `PORT`                  | `8080`                                  | HTTP 서버 포트                                                                                                |
| `KAKAO_REST_API_KEY`    | 필수                                    | Kakao REST API 키. `Authorization: KakaoAK <key>` 헤더로 전송됩니다.                                          |
| `AUTH_TOKEN`            | 미설정                                  | 설정하면 프로바이더 endpoint 요청의 `Authorization` 헤더에 해당 토큰이 필요합니다.                            |
| `USER_AGENT`            | `bookorbit-kakao-provider/<version>`    | 외부 HTTP 요청의 User-Agent                                                                                   |
| `KAKAO_API_BASE_URL`    | `https://dapi.kakao.com/v3/search/book` | Kakao API endpoint. 호환 프록시 또는 로컬 Mock 서버를 사용할 때 변경할 수 있습니다.                           |
| `REQUESTS_PER_SECOND`   | `2`                                     | upstream 요청 시작 속도 제한. 검색 요청이 제한을 공유합니다. `1`은 초당 1건, `0`은 시작 간격 제한 해제입니다. |
| `MAX_INFLIGHT_REQUESTS` | `2`                                     | 동시에 진행할 수 있는 upstream HTTP 요청의 최대 개수. 1보다 작은 값은 1로 보정됩니다.                         |

Kakao REST API 키를 BookOrbit 클라이언트에 노출하거나 로그에 기록하지 마세요.

## API 엔드포인트

| 경로                                                    | 설명                                |
| ------------------------------------------------------- | ----------------------------------- |
| `GET /provider/info`                                    | 프로바이더 메타데이터 및 지원 필드  |
| `GET /search?query=&limit=&author=&target=&sort=&page=` | 도서 검색                           |
| `GET /search?isbn=`                                     | ISBN 검색 (`target=isbn` 자동 적용) |
| `GET /book/{id}`                                        | ISBN으로 도서 조회                  |

`limit` 기본값은 5이며 최대 20입니다. `target`은 `title`, `isbn`, `publisher`, `person`을 지원하고, `sort`는 `accuracy`, `latest`를 지원합니다. `page`는 Kakao가 지원하는 1~50 범위로 제한됩니다. `isbn`이 전달되면 `query`보다 우선합니다. Kakao의 `size`는 프로바이더 결과 제한값을 기준으로 설정합니다.

## 메타데이터 매핑

| BookOrbit 필드      | Kakao 필드             | 설명                                                           |
| ------------------- | ---------------------- | -------------------------------------------------------------- |
| `title`             | `title`                | 도서 제목                                                      |
| `description`       | `contents`             | 도서 설명                                                      |
| `url`               | `url`                  | Daum 도서 상세 URL                                             |
| `authors`           | `authors`              | 저자만 매핑합니다. 번역자는 upstream 응답에서 별도 제공됩니다. |
| `publisher`         | `publisher`            | 출판사명                                                       |
| `publishedDate`     | `datetime`             | ISO 8601 timestamp를 `YYYY-MM-DD`로 변환                       |
| `publishedYear`     | `datetime`             | 출간일의 앞 네 자리 연도                                       |
| `cover`             | `thumbnail`            | thumbnail의 `fname` 파라미터에서 추출한 원본 표지 URL          |
| `isbn13` / `isbn10` | `isbn`                 | ISBN-13과 ISBN-10을 분리해 제공합니다.                         |
| `providerId`        | `isbn13` 또는 `isbn10` | ISBN-13을 우선 사용하며 ISBN이 없는 레코드는 제외합니다.       |

문서화된 Kakao 응답에는 시리즈와 장르를 안정적으로 제공하는 필드가 없어 매핑하지 않습니다. 가격, 판매가, 번역자, 상태는 upstream에서 제공될 수 있지만 현재 BookOrbit 메타데이터 구조에는 포함하지 않습니다.

Kakao가 지원하는 미디어 유형은 book뿐입니다. 검색 결과에 만화책이 포함될 수 있지만, 별도의 만화 미디어 유형이나 구분 데이터가 제공되지 않으므로 모든 검색 결과를 book으로 분류합니다. 또한 개별 판본의 언어 정보가 제공되지 않으므로 languageCode는 생략합니다.

## 조회 동작 및 제한 사항

Kakao 도서 검색 API에는 ID 전용 조회 endpoint가 없습니다. ISBN-13(없으면 ISBN-10)을 영구 `providerId`로 사용하고 `/book/{id}`는 ISBN 대상 검색으로 조회합니다. ISBN이 없는 레코드는 제외하므로 메모리 캐시 없이 재시작 이후에도 조회할 수 있습니다.

검색 1회당 Kakao 요청 1건을 전송합니다. `page`는 받지만 여러 페이지를 자동으로 가져오지는 않습니다. Kakao에는 별도의 `author` 요청 파라미터가 없으므로 저자 일치 결과를 반환된 페이지 안에서 우선 정렬합니다.

Kakao의 `contents`는 약 250자 내외의 짧은 발췌문으로 문장 중간에서 끝날 수 있습니다. 유효한 ISBN이 없는 레코드는 의도적으로 제외합니다. `status` 값은 변경될 수 있는 표시용 문자열입니다. 서로 다른 판본이 별도 결과로 반환될 수 있으며, API 할당량과 가용성은 Kakao가 관리합니다.

## 요청 속도 제한

`REQUESTS_PER_SECOND`는 upstream 요청을 시작하는 속도를 제한합니다. 기본값 `2`는 요청 시작 시점을 최소 500ms 간격으로 유지합니다. `MAX_INFLIGHT_REQUESTS`는 실제 동시에 처리 중인 HTTP 요청 수를 별도로 제한하며 기본값은 2입니다. 두 설정은 독립적으로 적용됩니다.

## 테스트

테스트는 로컬 Mock upstream 서버를 사용해 Provider HTTP route, 캡처된 응답 fixture, 응답 매핑 및 upstream 오류 변환을 검증합니다. Kakao 운영 API를 호출하지 않습니다.

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
