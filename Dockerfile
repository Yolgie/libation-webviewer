# ---- builder ----
FROM rust:1-bookworm AS builder
RUN rustup target add x86_64-unknown-linux-musl \
 && apt-get update \
 && apt-get install -y --no-install-recommends musl-tools \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY assets/ assets/
COPY templates/ templates/
RUN cargo build --release --target x86_64-unknown-linux-musl --bin libation-webviewer

# ---- runtime ----
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/libation-webviewer /libation-webviewer
ENV CACHE_DIR=/cache
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/libation-webviewer"]
