# BookOrbit YES24 메타데이터 프로바이더

[English](README.md) | [한국어](README.ko.md)

Rust 기반 BookOrbit 외부 메타데이터 프로바이더입니다. YES24 Open API의 상품 검색 및 상품 상세 조회 API를 사용합니다.

- 상품 검색: `GET https://apis.yes24.com/v1/goods/itemList`
- 상품 상세: `GET https://apis.yes24.com/v1/goods/itemDetail`
- 인증: `X-Api-Key: <YES24_API_KEY>`
- 문서: https://developers.yes24.com/api-doc/goods-item-list
- 문서: https://developers.yes24.com/api-doc/goods-item-detail

## 설정

필수 환경변수:

- `YES24_API_KEY`: YES24 Developers에서 발급받은 API Key

선택 환경변수:

- `PORT`: HTTP 포트. 기본값 `8080`
- `AUTH_TOKEN`: 설정하면 BookOrbit에서 들어오는 요청의 `Authorization` 헤더에 같은 토큰이 있어야 합니다.
- `USER_AGENT`: YES24 API 요청의 User-Agent. 기본값은 패키지 버전 기반 값입니다.
- `YES24_API_BASE_URL`: YES24 상품 API 기본 주소. 기본값은 `https://apis.yes24.com/v1/goods`입니다. 호환 프록시나 로컬 Mock 서버를 사용할 때 변경할 수 있습니다.
- `REQUESTS_PER_SECOND`: YES24 upstream 요청 시작 속도 제한. 기본값은 초당 2건(요청 시작 간격 500ms)이며, `0`이면 제한을 비활성화합니다.
- `MAX_INFLIGHT_REQUESTS`: 동시에 처리할 upstream HTTP 요청의 최대 개수. 기본값은 `2`이며 최소 `1`입니다.

YES24 API Key는 BookOrbit 클라이언트에 노출하지 말고 프로바이더 컨테이너에만 설정하세요.

## 실행

```bash
YES24_API_KEY=your_api_key cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## Docker

```bash
docker build -t bookorbit-yes24-provider:latest .
docker run -d --name bookorbit-yes24-provider \
  --restart unless-stopped -p 8081:8080 \
  -e YES24_API_KEY=your_api_key \
  bookorbit-yes24-provider:latest
```

기존 BookOrbit Compose에 서비스를 추가하는 예시:

```yaml
yes24-provider:
  container_name: bookorbit-yes24-provider
  image: bookorbit-yes24-provider:latest
  restart: unless-stopped
  init: true
  environment:
    YES24_API_KEY: ${YES24_API_KEY?required}
    # 기본값: https://apis.yes24.com/v1/goods
    # YES24_API_BASE_URL: https://apis.yes24.com/v1/goods
    # 초당 upstream 요청 시작 횟수 (기본 2, 0은 제한 비활성화)
    # REQUESTS_PER_SECOND: 2
    # 동시에 처리할 upstream HTTP 요청 수 (기본 2, 최소 1)
    # MAX_INFLIGHT_REQUESTS: 2
    # 선택 사항: BookOrbit 외부 프로바이더 요청도 보호할 때 사용
    # AUTH_TOKEN: ${BOOKORBIT_PROVIDER_TOKEN}
    # 선택 사항: BookOrbit이 동일 네트워크 안에 있지 않은 경우 사용
  # ports:
  #   - "8081:8080"
```

BookOrbit의 외부 메타데이터 프로바이더 URL은 같은 Compose 네트워크라면 `http://bookorbit-yes24-provider:8080`을 사용합니다. `YES24_API_KEY`는 Compose를 실행하는 디렉터리의 `.env`에 추가하거나 기존 환경변수 공급 방식을 사용하세요.

## 엔드포인트

| 경로                             | 설명                            |
| -------------------------------- | ------------------------------- |
| `GET /provider/info`             | BookOrbit 프로바이더 메타데이터 |
| `GET /search?query=클린코드`     | 상품 검색                       |
| `GET /search?isbn=9788966260959` | ISBN으로 검색                   |
| `GET /book/12345678`             | YES24 상품 번호로 상세 조회     |

검색 파라미터:

- `query`: 필수. 검색어
- `isbn`: `query`보다 우선합니다. ISBN-13 또는 ISBN-10을 상세 API로 조회하고, 결과가 없으면 제목 검색으로 대체합니다.
- `target`: `title`, `isbn`, `publisher`, `person`. YES24 검색 API에는 동일한 target 필드가 없으므로 ISBN13은 상세 조회 API를 사용하고, 출판사/저자 필터는 반환된 검색 결과에 로컬 적용합니다.
- `author`: 검색 결과 저자 문자열 필터
- `sort`: `accuracy`(YES24 `DEFAULT`) 또는 `latest`(YES24 `RECENT`)
- `page`: 1 이상 페이지 번호
- `limit`: 결과 수, 기본 5, 최대 20
- `category`: YES24 카테고리. 기본 `BOOK`; 허용값 `ALL`, `BOOK`, `FOREIGN`, `EBOOK`, `MUSIC`, `DVD`

검색 시 `detail=Y`를 요청해 YES24에서 제공하는 확장 정보를 가능한 범위에서 매핑합니다. 상품 상세는 검색 결과 캐시에 의존하지 않고 YES24의 ItemId 조회 API를 호출하므로 프로세스 재시작 이후에도 조회할 수 있습니다.

## 매핑 및 제한

- `providerId`: `<ItemId>`
- `title`: 제목. 부제목이 있으면 `제목: 부제목` 형식
- `authors`: 저자만 포함하며 번역자와 감수자는 제외합니다. 저자가 있을 때 삽화 담당자도 제외합니다.
- `description`: HTML을 제거한 일반 텍스트 도서 소개를 사용하고, 소개가 없으면 줄거리를 사용합니다. 목차는 제외합니다.
- `cover`: 원본 URL이 `/L`이면 더 큰 `/XL` 이미지 URL을 우선 사용하고, 그 외 URL은 유지합니다.
- `publishedDate` / `publishedYear`: 출간일에서 추출
- `series`: BookOrbit 계약 형식인 `[{ "series": "...", "sequence": "" }]`로 매핑
- `pages`, `rating`: BookOrbit 필드명으로 제공합니다. YES24의 0~10점 별점은 0~5점으로 변환합니다. 판본별 언어 정보가 없어 `languageCode`는 생략합니다.
- `mediaType`: 상품 유형 또는 관리 분류가 만화이면 `comic`, 그 외에는 `book`으로 분류합니다. 지원하지 않는 오디오북은 결과에서 제외합니다.
- `url`: YES24 상품 링크

## 요청 속도 제한

`REQUESTS_PER_SECOND`는 검색과 상세 조회가 공유하는 upstream 요청 시작 속도 제한입니다. 기본값 `2`는 요청 시작 시점을 최소 500ms 간격으로 유지합니다. `MAX_INFLIGHT_REQUESTS`는 실제 동시 처리 중인 요청 수를 제한하며 기본값은 2입니다. 두 설정은 독립적으로 적용됩니다. `REQUESTS_PER_SECOND=0`은 시작률 제한만 끄고, `MAX_INFLIGHT_REQUESTS`는 최소 1입니다.

테스트는 로컬 Axum Mock 서버를 통해 Provider Router → HTTP Client → Mock upstream 경로를 검증하며 YES24 운영 API를 호출하지 않습니다.
