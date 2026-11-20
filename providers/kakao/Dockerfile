FROM rust:1.90-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY testdata ./testdata
# The crate registry and the build folder outlive the build, so a rebuild recompiles only what
# changed. The binary is copied out because a cache mount is not part of the image.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=kakao-target,sharing=locked \
    cargo build --release --locked && cp target/release/kakao-provider /kakao-provider

# Static musl binary; rustls carries its own root certificates, so the image needs nothing else.
FROM scratch
ENV KAKAO_API_BASE_URL=https://dapi.kakao.com/v3/search/book \
    REQUESTS_PER_SECOND=2 \
    MAX_INFLIGHT_REQUESTS=2
COPY --from=build /kakao-provider /kakao-provider
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/kakao-provider"]
