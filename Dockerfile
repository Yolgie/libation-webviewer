# ---- builder ----
FROM rust:1-bookworm AS builder
ARG TARGETARCH

# buildx sets TARGETARCH for each platform leg; pick the matching musl
# triple so each platform builds natively (under QEMU when the runner
# arch != target arch).
RUN case "$TARGETARCH" in \
        amd64) TRIPLE="x86_64-unknown-linux-musl" ;; \
        arm64) TRIPLE="aarch64-unknown-linux-musl" ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac && \
    echo "$TRIPLE" > /target && \
    apt-get update && \
    apt-get install -y --no-install-recommends musl-tools && \
    rm -rf /var/lib/apt/lists/* && \
    rustup target add "$TRIPLE"

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY assets/ assets/
COPY templates/ templates/
RUN TRIPLE="$(cat /target)" && \
    cargo build --release --target "$TRIPLE" --bin libation-webviewer && \
    mkdir -p /out && \
    cp "target/${TRIPLE}/release/libation-webviewer" /out/libation-webviewer

# ---- runtime ----
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /out/libation-webviewer /libation-webviewer
ENV CACHE_DIR=/cache
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/libation-webviewer"]
