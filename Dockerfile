FROM rust:1-bookworm AS builder

ARG TARGETARCH
ARG CARGO_BUILD_JOBS

ENV DEBIAN_FRONTEND=noninteractive
ENV CC=musl-gcc
ENV AR=ar
ENV RUST_BACKTRACE=full

RUN apt-get update && apt-get install -y musl-tools

RUN mkdir /dist /dist/data

WORKDIR /src
ADD . .
RUN find

RUN rustup --version

RUN case "$TARGETARCH" in \
      arm64) TARGET=aarch64-unknown-linux-musl ;; \
      amd64) TARGET=x86_64-unknown-linux-musl ;; \
      *) echo "Does not support $TARGETARCH" && exit 1 ;; \
    esac && \
    rustup target add $TARGET && \
    cargo build --profile release-build --target $TARGET && \
    mv target/$TARGET/release-build/maxmind-geoip-api /dist/


# Copy the binary into an empty docker image
FROM scratch

LABEL org.opencontainers.image.authors="Stefan Sundin"
LABEL org.opencontainers.image.url="https://github.com/stefansundin/maxmind-geoip-api"

COPY --from=builder /dist /

ENV DATA_DIR=/data
VOLUME [ "/data" ]

ENV PORT=80
EXPOSE 80

ENTRYPOINT [ "/maxmind-geoip-api" ]
