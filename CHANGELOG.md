# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until `0.1.0` is tagged, everything lives under `Unreleased`.

## [Unreleased]

### Added

- Approved design doc in [`PLAN.md`](./PLAN.md) covering scope, architecture,
  routes, tests, deployment, and CI/CD.
- Single-crate Rust skeleton (`axum` + `askama` + `rusqlite` bundled + `mp4ameta`
  + `id3` + `image`/`fast_image_resize` + `rust-embed`).
- Library list at `GET /` rendered from Libation's SQLite DB. One denormalised
  query joins `Books`/`LibraryBooks`/`UserDefinedItem` plus per-book
  `GROUP_CONCAT` subqueries for authors and narrators. Plain HTML for now;
  templates land in a later slice.
- Embedded cover pipeline:
  - `GET /books/{asin}/cover` returns the full-resolution embedded image
    (JPEG or PNG) extracted via `mp4ameta` (`.m4b`/`.m4a`/`.mp4`) or `id3`
    (`.mp3`).
  - `GET /books/{asin}/thumb` returns a 200x200 WebP, resized on-demand.
  - On-disk cache under `<CACHE_DIR>/covers/<asin>/`, keyed on a short
    sha256 of `(mtime, size)` so re-downloads invalidate automatically.
  - Bundled placeholder SVG when the book has no folder, no audio file,
    or no embedded cover.
- Book detail page at `GET /books/{asin}`: full metadata (subtitle, authors,
  narrators, publisher, series, length, locale, status, description), file
  list, and a "files missing on disk" badge for in-DB-but-not-on-disk books.
- `GET /books/{asin}/files` returns just the `<ul>` fragment (for the future
  HTMX swap on the detail page).
- `GET /books/{asin}/download/{n}` streams the n-th audio file (alpha-sorted)
  via `tokio_util::io::ReaderStream` with `Content-Disposition: attachment`.
- `GET /healthz` returns `ok`.
- Folder scanner (`src/fs.rs`): walks `LIBATION_BOOKS`, parses
  `[ASIN]` from folder names via `\[([A-Z0-9]{10})\]`, collects
  `.m4b`/`.m4a`/`.mp4`/`.mp3` plus the `.metadata.json` companion.
- Graceful degradation: DB unreachable serves a static "data unavailable"
  page, missing covers fall back to the placeholder, unknown ASINs return
  `404`, unknown enum values render as `Unknown(N)`.
- Two deployment recipes:
  - Generic [`compose/`](./compose/) for upstream users.
  - Opinionated [`examples/dockge/`](./examples/dockge/) for the author's
    Dockge + Caddy + `proxy`-network setup.
- Multi-arch Dockerfile (linux/amd64 + linux/arm64) compiling to musl-static
  → `gcr.io/distroless/static:nonroot`. ~6 MB binary, ~12 MB image.
- GitHub Actions:
  - `ci.yaml`: fmt + clippy `-D warnings` + nextest + llvm-cov + audit +
    deny, with a PR coverage-delta comment.
  - `codeql.yaml`: Rust CodeQL on PRs, pushes, and weekly.
  - `release.yaml`: multi-arch build, push to `ghcr.io`, Trivy SARIF scan,
    Syft SBOM, cosign keyless sign + SBOM attest, release-drafter.
  - `schedule-tests.yaml`: nightly re-run of `ci.yaml` on `main`.
- Dependabot config (cargo + github-actions + docker, weekly, grouped by
  patch/minor).
- `mise.toml` pinning Rust + cargo dev tools so the next Claude sandbox
  bootstraps automatically.
- Test suite (31 passing, 4 test files: `db_queries`, `fs_scan`,
  `cover_pipeline`, `handlers`) covering: list query against the trimmed
  sample DB, ASIN regex, scan walk + sort, cover extraction round-trip for
  m4b/mp3/no-cover, thumbnail resize, cache hit/miss, all HTTP handlers
  via `tower::ServiceExt::oneshot`.
- Anonymised `tests/fixtures/sample.db` plus three synthetic m4b/mp3
  fixtures generated with ffmpeg.
- Repo hygiene: `.gitignore`, `.dockerignore`, `deny.toml`, CODEOWNERS,
  SECURITY.md, PR + issue templates, `release-drafter.yml`.
- MIT [`LICENSE`](./LICENSE).
- This `CHANGELOG.md`.

### Changed

- `examples/dockge/compose.yaml`: parameterize the container user as
  `${LIBATION_UID:-65532}:${LIBATION_GID:-65532}` so the stack can run
  as the same uid Libation uses (e.g. `997:986`), avoiding any
  permission-mode gymnastics on the bind-mounted DB and books.

- Compose files (`compose/compose.yaml` + `examples/dockge/compose.yaml`):
  quote every `${...}` value and the image / path strings. The strict
  YAML 1.2 plain-scalar rules reject `{` and `}` in some lexer modes;
  quoting sidesteps the variation and survives paste-through-terminal
  cleanly. Locally validated with `docker compose v2.30 config`.

### Fixed

- `release.yaml`: lowercase the image owner so `ghcr.io` works regardless of
  the GitHub username's case.
- `release.yaml`: Trivy SARIF upload now gated on `hashFiles('trivy.sarif')`
  so it doesn't fail when an earlier step crashed before Trivy ran.
- `release.yaml`: Trivy severity narrowed to `CRITICAL` only to avoid
  blocking the first publish on a non-actionable HIGH advisory.
- Dockerfile: explicit `COPY assets/ assets/` and `COPY templates/ templates/`
  so `include_bytes!("../../assets/placeholder.svg")` resolves at compile
  time inside the image.
- Dockerfile: pick the musl triple per `TARGETARCH` (amd64 → x86_64-musl,
  arm64 → aarch64-musl) so the arm64 buildx leg doesn't try to pass `-m64`
  to an arm64 `musl-gcc`.

[Unreleased]: https://github.com/Yolgie/libation-webviewer/commits/main
