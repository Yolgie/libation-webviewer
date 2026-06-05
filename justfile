default:
    @just --list

# Install system packages mise can't manage. Run once on a fresh host.
bootstrap:
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends sqlite3 file ffmpeg musl-tools

# Run the full local CI suite (mirrors .github/workflows/ci.yaml).
ci:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo nextest run --all
    cargo llvm-cov nextest --lcov --output-path lcov.info
    cargo audit
    cargo deny check

# Test only (faster than `ci`).
test:
    cargo nextest run --all

# Run the binary against env-configured paths.
run:
    cargo run

# Build the multi-arch container image locally.
image tag="latest":
    docker buildx build --platform linux/amd64,linux/arm64 -t libation-webviewer:{{tag}} .

# Regenerate the binary test fixtures (requires ffmpeg from `just bootstrap`).
fixtures-m4b:
    @echo "TODO: build tests/fixtures/tiny.m4b via ffmpeg"

fixtures-mp3:
    @echo "TODO: build tests/fixtures/tiny.mp3 via ffmpeg"

fixtures-db:
    @echo "TODO: trim + anonymise the source DB into tests/fixtures/sample.db"
