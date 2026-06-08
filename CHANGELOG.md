# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Until `0.1.0` is tagged, everything lives under `Unreleased`.

## [Unreleased]

### Added (SonarQube Cloud scanning)

- New `.github/workflows/sonar.yaml` runs the
  `SonarSource/sonarqube-scan-action` on every push to `main` and on
  PRs from the same repo. PRs from forks are skipped via an `if:`
  guard because `SONAR_TOKEN` is not exposed to fork workflows.
  Action pinned by SHA per project policy.
- New `sonar-project.properties` at the repo root:
  `projectKey=Yolgie_libation-webviewer`, `organization=yolgie`,
  `sources=src`, `tests=tests`, plus exclusions for `target/`,
  `tests/fixtures/`, and `assets/`.

### Changed (action SHA pinning)

- Every GitHub Actions reference in the workflows is now pinned by
  commit SHA with the major-version tag as a trailing comment, e.g.
  `uses: actions/checkout@df4cb1c0...  # v6`. Covers all three
  workflow files (`ci.yaml`, `codeql.yaml`, `release.yaml`) and
  every action used: `actions/{checkout, upload-artifact}`,
  `dtolnay/rust-toolchain`, `Swatinem/rust-cache`,
  `taiki-e/install-action`, `github/codeql-action/{init, analyze,
  upload-sarif}`, `docker/{setup-qemu-action, setup-buildx-action,
  login-action, metadata-action, build-push-action}`,
  `aquasecurity/trivy-action`, `anchore/sbom-action`,
  `sigstore/cosign-installer`, `release-drafter/release-drafter`,
  `romeovs/lcov-reporter-action`.
- Matches the project policy in PLAN.md ("actions pinned by commit
  SHA, Dependabot keeps them bumped"). Dependabot's
  `github-actions` ecosystem will now bump the SHAs in place.

### Added (supplements)

- **PDF supplement links** on the book detail page. Libation's
  `Supplement` table holds post-purchase URLs (e.g. companion PDFs);
  the detail page now surfaces them under a "Supplements" section
  with each URL as an outbound link (`target="_blank"`,
  `rel="noopener noreferrer"`). The viewer doesn't fetch or proxy
  the bytes — it just exposes the URL Libation already stored.
- `BookDetail.supplements: Vec<String>` plus a new `SUPPLEMENTS_SQL`
  query (`SELECT Url FROM Supplement WHERE BookId = ?1 ORDER BY
  SupplementId`).
- 4 new tests: `get_book_by_asin` returns the supplement URL for
  "Die Zwerge" (the one book in the sample DB with a supplement)
  and an empty list for books without any; the detail page renders
  the `<h2>Supplements</h2>` section with the URL when present and
  hides it entirely when absent.

### Added (admin write path slice)

- **Admin login, logout, and `POST /books/{asin}/requeue`.** Behind
  the `ENABLE_ADMIN` flag, the viewer now mounts:
  - `GET /admin/login` — renders the password form.
  - `POST /admin/login` — constant-time password compare (subtle),
    issues a random 256-bit session token stored in an in-memory
    `HashMap`, sets `lwv_session=<token>; HttpOnly; SameSite=Strict;
    Path=/; Max-Age=86400`, redirects to `/`.
  - `POST /admin/logout` — invalidates the token, clears the cookie,
    redirects to `/`.
  - `POST /books/{asin}/requeue` — writes `BookStatus = 0` in
    `UserDefinedItem` via the second (writable) DB mount, so
    Libation re-downloads the book on its next scan. Auth-gated,
    schema-gated, and gated on `LIBATION_DB_RW` being set.
- **New env vars driving the slice:**
  - `ENABLE_ADMIN=1` mounts the admin routes (otherwise they're
    absent from the router entirely, not 403).
  - `ADMIN_PASSWORD=<plaintext>` requires a login; unset = anonymous
    admin (startup logs WARN).
  - `LIBATION_DB_RW=<path>` is the second mount of the Libation DB
    (Dockerfile/compose templates already set this up).
- **`AdminContext`** in `view::` rolls up `{enabled, logged_in,
  writes_allowed, requires_password}` per request. Library + book
  templates use it to render the login/logout link, a "schema head
  unknown - admin writes disabled" banner, and the requeue button.
- New `src/auth.rs` (`AuthBackend`, `make_set_cookie`,
  `make_clear_cookie`, `extract_session_token`).
- New `src/routes/admin.rs` with three handlers + two templates
  (`admin_login.html`, `requeue_result.html`).
- New `db::Library::open_rw` + `book_id_for_asin` + `requeue_book`
  (`BEGIN IMMEDIATE; UPDATE UserDefinedItem SET BookStatus = 0
  WHERE BookId = ?1; COMMIT`).
- `AppState` grows `db_path_rw`, `enable_admin`, and a shared
  `Arc<AuthBackend>`.
- 31 new tests bringing the suite to 96 passing:
  - 11 `src/auth.rs` unit tests cover anonymous mode, empty
    password, constant-time verify, token issue/verify/invalidate,
    expiry/eviction, header parsing, and the cookie attributes.
  - 19 `tests/admin.rs` integration tests cover: admin routes
    return 404 when disabled, the login form, login submit with
    right/wrong password, logout clears the cookie, requeue
    without auth → 401, requeue when writes disabled or
    `LIBATION_DB_RW` unset → 503, requeue unknown ASIN → 404,
    anonymous-mode requeue flips `BookStatus` to 0, requeue with
    a valid cookie writes through, requeue only touches the
    `BookStatus` column (`IsFinished`, `Tags`, `LastDownloaded`
    are untouched), library shows the login link / logout / schema
    banner appropriately, detail shows/hides the requeue button
    based on auth state.

### Added (schema-drift guard slice)

- **Schema-drift guard at startup.** `db::Library::check_schema`
  reads the latest `MigrationId` from `__EFMigrationsHistory` and
  compares it against a baked-in `KNOWN_GOOD_MIGRATIONS` list
  (currently just `20260427201829_ReAddCategoryName2`, the sample
  DB's head as of June 2026). The result lives on `AppState` as
  `admin_writes_allowed` and is consulted by the admin write path
  (next slice) so an unknown schema head is fail-closed.
- New `ALLOW_UNKNOWN_SCHEMA` env var — when set to `1`/`true`,
  overrides the guard and enables admin writes regardless. Logged at
  WARN with the actual schema head so it shows up clearly in the
  log.
- 3 new tests in `tests/db_queries.rs`: known head against
  `sample.db`, fabricated unknown head in a temp DB, and missing
  `__EFMigrationsHistory` table.

### Added (sort/filter slice)

- **Library list query params** + HTMX partial-swap. `GET /` accepts
  `?q=<text>&sort=<key>&status=<state>` to filter the rendered list.
  Recognised sort keys: `title` (default, case-insensitive),
  `author`, `length` (descending), `date_added` (descending).
  Recognised statuses: `all` (default), `downloaded`,
  `not_downloaded`.
- New `GET /partial/library` route returns just the `<tbody>` block
  with the filtered rows. The library page's filter form binds to it
  via `hx-get` + `hx-target="#library-rows"` so the table updates as
  you type / pick options without reloading the page.
- `HX-Push-Url` response header on `/partial/library` keeps the
  address bar in sync with active filters (so `Ctrl+R` lands on the
  same view).
- `src/query.rs` (new) holds `LibraryQuery`, `apply`, and
  `url_querystring`. 10 unit tests cover default + custom sorts,
  case-insensitivity, search across title / subtitle / author /
  narrator, status filtering, the active-filters detector, and the
  URL round-trip.
- 7 new integration tests in `tests/handlers.rs`: search filters
  to the matching row, status filter returns the empty-state copy,
  length-sort puts the long Super Powereds book ahead of PHM,
  the partial route returns rows-only (no layout), the partial
  sets `HX-Push-Url` with active filters, the full page renders
  the filter form, and the form preserves the active query.

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

- Library list and book detail pages now use askama templates instead
  of `format!`-built HTML. `templates/_layout.html` shares the
  scaffolding; `library.html`, `book.html`, `files_fragment.html`,
  `error_db.html`, and `not_found.html` cover the surfaces. Askama's
  auto-escape replaces the hand-written `html_escape` helper.
- Bundled CSS (`assets/style.css`) and HTMX 2.0.4 (`assets/htmx.min.js`)
  embedded into the binary via `rust-embed` and served at `/static/{*}`.
  The base layout pulls them in so every page has htmx loaded — the
  partial-swap routes follow in the next slice.
- DB string fields (`title`, `subtitle`, contributor and series names)
  are now run through `html::decode_entities` before they reach the
  template, fixing the `&amp;amp;` double-encode regression: source
  text like `Spells, Swords, &amp; Stealth` reduces to a single `&amp;`
  in the rendered HTML.
- Book descriptions: Libation stores them as raw HTML markup
  (`<p><b>...</b></p>`); we now run them through `html::paragraphs`
  in the book handler, so the template renders plain prose paragraphs
  inside our own `<div class="description"><p>...</p></div>` block
  rather than leaking escaped tag markers.
- `examples/dockge/compose.yaml`: parameterize the container user as
  `${LIBATION_UID:-65532}:${LIBATION_GID:-65532}` so the stack can run
  as the same uid Libation uses (e.g. `997:986`), avoiding any
  permission-mode gymnastics on the bind-mounted DB and books.

- New `src/html.rs` helper module (`strip_tags`, `decode_entities`,
  `paragraphs`) — no external dependency, just regex and a small
  manual entity table for the handful Libation actually emits.
- New `src/static_assets.rs` (`#[derive(RustEmbed)]`) and
  `src/routes/static_route.rs` serving `/static/{*path}` with
  long-cached `Cache-Control: public, max-age=86400`.
- 14 new tests bringing the suite to 44 passing:
  6 unit tests in `src/html.rs` cover `strip_tags`, named + numeric
  entity decoding, paragraph splitting on `</p><p>`, and edge cases;
  3 new `tests/handlers.rs` tests assert the library list has no
  `&amp;amp;` left, the detail page strips description HTML, and
  every page links to `/static/style.css` + `/static/htmx.min.js`;
  5 `tests/static_route.rs` tests cover the new route across css/js/
  svg, the cache header, and 404 on unknown paths.

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
