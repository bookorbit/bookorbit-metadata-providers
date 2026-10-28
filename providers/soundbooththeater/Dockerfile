FROM rust:1.90-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY testdata ./testdata
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=soundbooththeater-target,sharing=locked \
    cargo build --release --locked && cp target/release/soundbooththeater-provider /soundbooththeater-provider

FROM scratch
COPY --from=build /soundbooththeater-provider /soundbooththeater-provider
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/soundbooththeater-provider"]