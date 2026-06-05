# Libation Library Web Viewer — Implementation Plan

## Context

A small self-hosted web viewer for a Dockerized Libation audiobook library.
The viewer must:

- Present a browsable list with sort/filter, cover art, book detail pages,
  and browser download links to the underlying files.
- Read Libation's SQLite DB and book files **read-only by default**.
- Provide an admin-gated write path back into Libation's DB so toggling a
  book's "should be downloaded" flag causes Libation to (re-)download it on
  its next scan. This is the *only* legitimate write target; nothing else
  writes to Libation's data.
- Ship as a single small static-binary container that any operator can
  `docker run` with a few env vars. The author's own deployment (Dockge
  stack on a shared `proxy` network with Caddy and a wildcard cert) is
  a worked example, **not** the project's default contract.
- Be a public open-source project from day one. Test coverage is
  therefore a first-class requirement: green tests must be sufficient
  to auto-bump dependencies and republish images via CI.
- Support both **m4b** (MP4 container, Libation's default) and **mp3**
  files as first-class library content from v1 — covers, downloads,
  and detail rendering must work identically for both.
- **Always render something useful**, even when supporting data is
  missing. Missing `metadata.json`, missing embedded cover, missing
  files on disk, or a DB row with no matching folder must each
  degrade gracefully to a less informative but never-broken page.

## Decisions locked in during planning

| Decision        | Choice                                                                              |
| --------------- | ----------------------------------------------------------------------------------- |
| Language        | Rust                                                                                |
| HTTP / runtime  | axum + tokio                                                                        |
| Templating      | askama (compile-time templates; cleaner errors than maud for HTMX partials)         |
| DB driver       | rusqlite (bundled SQLite, WAL-aware, separate RO and RW connections)                |
| MP4 tag reader  | mp4ameta (for the `covr` atom in m4b/m4a)                                           |
| MP3 tag reader  | id3 (first-class for MP3; dispatched by file extension alongside mp4ameta)          |
| Image resize    | image + fast_image_resize                                                           |
| Frontend        | Server-rendered HTML + HTMX for sort/filter/admin partials                          |
| Project layout  | Single crate with modules under `src/` — no multi-crate workspace                   |
| Auth            | Anonymous reads always; admin password set via plaintext `ADMIN_PASSWORD` env var   |
| Writable state  | Cover/thumbnail cache only; **no status overlay store** (writes go to Libation)     |
| Container base  | `gcr.io/distroless/static-debian12:nonroot` (need a uid but no shell needed)        |
| CI host         | GitHub Actions on `ubuntu-latest`                                                   |
| Registry        | GitHub Container Registry (`ghcr.io/<owner>/libation-webviewer`)                    |
| Dep bumps       | Dependabot (GitHub-native) — grouped, auto-merge on green for patch/minor           |
| Code scanning   | CodeQL (Rust) + Trivy (container) — SARIF uploaded to the GitHub Security tab       |

## Live-system findings (verified against sample DB and Project Hail Mary)

### Schema (Libation EF Core, observed in 258 KB sample DB, 64 books)

```
Books              core book metadata; AudibleProductId is the ASIN
LibraryBooks       per-account membership (Account, DateAdded, IsAudiblePlus, IsDeleted, AbsentFromLastScan)
UserDefinedItem    Libation's writable state: BookStatus (THE flag), IsFinished, LastDownloaded*, PdfStatus, Tags, user ratings
Contributors       authors / narrators / publishers
BookContributor    join with Role enum (1=Author, 2=Narrator, 3=Publisher — inferred from data shape, verify against source)
Series             series with AudibleSeriesId
SeriesBook         join with Order as TEXT ("1", "2", "1.5")
Categories         flat list of categories with AudibleCategoryId
CategoryLadders    hierarchical paths (Audible has multiple ladders per book)
CategoryCategoryLadder  category↔ladder M:N join (PK columns are `_categoriesCategoryId`, `_categoryLaddersCategoryLadderId`)
BookCategory       book↔ladder (not book↔category; navigate via ladder)
Supplement         per-book extra-content URLs (PDFs etc.)
__EFMigrations*    EF Core metadata; used for schema-drift detection
```

Key column for write-back: **`UserDefinedItem.BookStatus`** (INTEGER).
Observed value distribution: every book in the sample is `1`. Treat the
mapping as `0 = NotLiberated` (triggers download on next Libation scan),
`1 = Liberated`. Confirm against Libation source before shipping (see
verification section).

### On-disk layout (verified with Project Hail Mary)

- Folder: `<books_root>/<Title> [<ASIN>]/`
- Audio file: `<Title> [<ASIN>].m4b` (MP4 base media, ISO 14496-12)
- Companion: `<Title> [<ASIN>].metadata.json` — Audible product metadata
  snapshot Libation writes alongside (contains `product_images.500`, a
  fully-qualified Audible CDN URL — useful as a future cover provider).
- Embedded cover: JPEG (504 KB observed) in the MP4 `covr` atom.
- Libation also writes private atoms `com.pilabor.tone:*` and
  `org.libation:*` plus the standard `asin`, `CDEK`, `©ART`, `©nrt`,
  `©nam`, `©gen`, etc.

The spec calls out that books *may* be split into per-chapter files. The
sample is single-file, but the metadata reports
`content_delivery_type: MultiPartBook` — Libation chose to merge. The
viewer must therefore handle both layouts: scan the book folder for all
audio files and treat the alphabetically-first as the cover source if
multi-part. ASIN parsing comes from the folder name (regex
`\[([A-Z0-9]{10})\]` against the dir name), not from tags.

### What was NOT found and the plan must defer until first install

- `FileLocations.json` (spec mentioned it) is not in the DB and not in this
  sample. Treat as "may or may not exist in Libation's config dir". The
  plan uses on-disk scanning + ASIN-from-folder-name as the authoritative
  mapping and ignores `FileLocations.json`. If it later turns out to be
  authoritative, swap the scanner for a JSON-driven mapper without
  changing route shape.

## System architecture

```
                       ┌──────────────────────┐
                  ┌────│  Libation container  │ writes its own DB + books
                  │    └──────────────────────┘
host paths        │
─────────────────────────────────────────────────────────────────────────
<LIBATION_CONFIG_DIR>  ──┬─► /data/libation-config        :ro   ◄── default read
                         └─► /data/libation-config-rw     :rw   ◄── admin write only
<LIBATION_BOOKS_DIR>     ───► /data/books                 :ro   ◄── always read-only
./cache (stack-local)    ───► /cache                      :rw   ◄── cover+thumb cache
                  │
                  ▼
            ┌────────────────────────────────────┐
            │ libation-webviewer (Rust + axum)   │
            │                                    │
            │  - RO SQLite handle (always open)  │
            │  - RW SQLite handle (opened only   │
            │    inside admin write transactions)│
            │  - on-disk scanner (cached in mem) │
            │  - cover extraction pipeline       │
            │  - HTMX-driven UI                  │
            └────────────────────────────────────┘
                          │
                          ▼
                ┌────────────────────┐
                │  Caddy (proxy net) │  TLS via wildcard cert, vhost
                └────────────────────┘
                          │
                          ▼
                  internal VPN clients
```

The RO/RW separation is purely an app-layer discipline (mounts always
present); no remount, no `CAP_SYS_ADMIN`, no Docker socket.

## Data model (read path)

A single `BookView` struct rendered by templates and serialized in JSON
endpoints, assembled by:

```sql
SELECT
  b.BookId, b.AudibleProductId, b.Title, b.Subtitle, b.LengthInMinutes,
  b.Locale, b.Language, b.DatePublished, b.PictureId, b.PictureLarge,
  b.Rating_OverallRating, b.IsAbridged,
  lb.Account, lb.DateAdded, lb.IsAudiblePlus, lb.IsDeleted, lb.AbsentFromLastScan,
  udi.BookStatus, udi.IsFinished, udi.LastDownloaded, udi.PdfStatus, udi.Tags
FROM Books b
LEFT JOIN LibraryBooks lb USING (BookId)
LEFT JOIN UserDefinedItem udi USING (BookId)
WHERE lb.IsDeleted = 0
```

Followed by per-book pulls for authors/narrators/series/categories (or one
N+1-avoiding fetch via grouped joins). The list view uses a single
denormalised query with comma-joined author/narrator names via
`GROUP_CONCAT`. Categories are pulled per detail page only.

Enum mappings shipped as Rust enums with `TryFrom<i32>`:

```rust
#[repr(i32)]
enum LiberatedStatus { NotLiberated = 0, Liberated = 1, Error = 2, PartialDownload = 3 }

#[repr(i32)]
enum ContributorRole { Author = 1, Narrator = 2, Publisher = 3 }
```

Unknown enum values logged at WARN, displayed as "Unknown(N)" in the UI
rather than crashing.

## Cover provider chain

Resolution order, computed once per (ASIN, source-mtime, source-size) and
cached:

1. **embedded** — dispatch on extension; PRIMARY source for both formats:
   - `.m4b` / `.m4a` / `.mp4` → mp4ameta reads the `covr` atom.
   - `.mp3` → id3 reads the `APIC` frame.
   Either way the result is a raw JPEG (or PNG) byte buffer handed to
   the resize stage. New formats slot in behind the same
   `CoverExtractor` trait.
2. **audible_cdn** *(future, scaffold the trait, do not ship in v1)* —
   `metadata.json -> product_images["500"]`. Cheap network fetch from
   `m.media-amazon.com`. Out of scope for v1 per spec but the
   `CoverProvider` trait lets us drop it in later without touching the
   cache layer.
3. **placeholder** — bundled SVG/JPEG returned with a long cache header.

Cache layout:

```
/cache/covers/<asin>/orig-<srchash>.jpg     full extracted image
/cache/covers/<asin>/thumb-<srchash>.webp   resized (200×200) for list view
/cache/covers/<asin>/.miss                  empty sentinel for "no cover"
```

Where `<srchash>` = first 16 hex chars of sha256("<mtime>:<size>") of the
source audio file. Files persist across restarts; eviction is by capacity
(see `CACHE_MAX_BYTES` env). On a stale hash, the new file gets written
and the old one is removed in the same operation.

## Routes

| Method | Path                          | Purpose                                                    |
| ------ | ----------------------------- | ---------------------------------------------------------- |
| GET    | `/`                           | Library list (HTML). Query params: `sort`, `q`, `filter[*]` |
| GET    | `/books/{asin}`               | Book detail (HTML)                                         |
| GET    | `/books/{asin}/cover`         | Full-res cover image                                       |
| GET    | `/books/{asin}/thumb`         | Resized thumbnail                                          |
| GET    | `/books/{asin}/download/{n}`  | Stream the n-th audio file (Content-Disposition: attachment) |
| GET    | `/books/{asin}/files`         | List of files for the book (HTML fragment, used by detail page) |
| GET    | `/partial/library`            | HTMX partial: just the library `<table>`/`<ul>` for filter swaps |
| POST   | `/admin/login`                | Verifies password, sets a session cookie                   |
| POST   | `/admin/logout`               | Clears the session cookie                                  |
| POST   | `/books/{asin}/requeue`       | Admin-only. Sets `BookStatus = 0` via the RW handle        |
| GET    | `/healthz`                    | Reads one row from the RO handle; 200 OK                   |
| GET    | `/static/*`                   | Bundled CSS / HTMX / favicon (embedded via `rust-embed`)   |

All write routes:

- Require a valid admin session cookie.
- Return 403 if `ADMIN_PASSWORD` is set but session is missing/invalid.
- Are no-ops with 503 if `ADMIN_PASSWORD` is unset *and*
  `LIBATION_DB_RW` env is also unset. (i.e. fully read-only deployment.)
- Are gated even further by a config flag `ENABLE_ADMIN=true` so an
  operator can disable the entire admin surface in a hardened deployment.

## Admin-mode mechanics

Two SQLite handles, both held by the app:

- **RO handle** (always open):
  `OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI`,
  URI: `file:/data/libation-config/LibationContext.db?immutable=0&mode=ro`,
  `busy_timeout = 5000`, WAL-aware.
- **RW handle** (lazy, opened per write batch, dropped after):
  `OpenFlags::SQLITE_OPEN_READ_WRITE`,
  path: `/data/libation-config-rw/LibationContext.db`,
  `busy_timeout = 5000`, statement-prepared cache disabled (write batches
  are tiny and infrequent).

Write transaction body (the only DML the viewer ever issues):

```sql
BEGIN IMMEDIATE;
UPDATE UserDefinedItem SET BookStatus = 0 WHERE BookId = ?1;
COMMIT;
```

No optimistic concurrency check in v1. In normal operation Libation
runs on a schedule (typically once nightly) while the viewer is used
interactively during the day, so the write windows don't overlap.
The handler logs `changes()` so a `0` (no row matched, e.g. ASIN out
of sync) shows up clearly. A v2 enhancement could detect that
Libation is currently scanning and pause writes until it finishes —
explicitly out of v1.

Schema-drift guard: on startup the app runs a fixed check query
(`SELECT MigrationId FROM __EFMigrationsHistory ORDER BY MigrationId
DESC LIMIT 1`) and compares to a baked-in list of known-good migration
IDs. If the latest is unknown, the app logs a WARN and disables the
write path until an operator sets `ALLOW_UNKNOWN_SCHEMA=1`. Read-only
endpoints remain available so the viewer never goes fully dark on a
Libation bump.

Auth gate (deliberately simple — low-sensitivity, easy setup):

- `ADMIN_PASSWORD` env var holds the password in plain text. The
  operator drops it into their `.env`; no hashing step.
- `/admin/login` constant-time-compares the submitted password against
  `ADMIN_PASSWORD`. On success, the app generates a random 256-bit
  token, stores it in an in-memory `HashMap<Token, Expiry>` (24h TTL),
  and sets it as a cookie. No HMAC, no `SESSION_SECRET` — the token is
  only valid because the map holds it; restart clears all sessions.
- If `ADMIN_PASSWORD` is unset and `ENABLE_ADMIN=true`, every
  authenticated check returns *granted* — the "personal install on a
  VPN" mode. Startup logs a WARN so accidental deployments are obvious.
- If `ENABLE_ADMIN=false` (default in the generic stack), the admin
  routes are absent from the router entirely (not just 403), and the
  UI never renders the toggle.

Threat model: the viewer sits behind a trust boundary (VPN or LAN).
The password is one click of friction — against a misclick or a
wandering browser tab — not a hardened secret. If the threat model
ever tightens, swap in argon2id + signed cookies behind the same
`AuthBackend` trait.

## What lives in `/cache` (the only app-writable surface)

```
/cache/covers/...        cover + thumbnail bytes (described above)
/cache/scan.log          rotating debug log of cover extraction outcomes
                          (helps diagnose books without recoverable covers)
```

Book metadata is held in memory: at startup the app runs the
assembly query once and keeps the result in an
`Arc<RwLock<Vec<BookView>>>`. The library is small (the sample DB
holds 64 books; even a 10× larger library is well under a megabyte),
so an in-memory snapshot is fine. The snapshot reloads if the
Libation DB's mtime moves between requests. The on-disk cache is
fully disposable; deleting it costs only the next cold-start
image-extraction pass.

## Graceful degradation (must always render)

The user's requirement: missing data makes the experience less
informative but must never make it broken. Every failure mode below
has a defined fallback; none escalates to a 5xx on read paths.

| Failure                                              | Fallback                                                                                                  |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Libation DB unreadable at startup                    | `/healthz` returns 503; root page shows a static "data unavailable" message with the underlying error.    |
| DB row present, no folder/file on disk               | Book renders in the list with a "missing files" badge; cover falls through to placeholder; download → 404 |
| Folder present on disk, no matching DB row           | Surfaced in an "orphaned files" view (linkable but off the main list); ASIN parsed from the folder name.  |
| Folder name has no `[ASIN]` token                    | Folder skipped; INFO log entry with the path so the operator can rename it.                               |
| File present, no embedded cover                      | Placeholder served; `.miss` sentinel written so we don't re-parse on every request.                       |
| mp4ameta / id3 errors mid-parse                      | Placeholder cover; WARN log; `.miss` sentinel written; book otherwise functional.                         |
| `metadata.json` missing                              | Detail page renders DB-derived fields only; no review/summary section.                                    |
| `metadata.json` present but unparseable              | Same as missing; WARN log with the line/column.                                                           |
| Multiple audio files in one folder (split chapters)  | First (alpha-sorted) is the cover source; all files exposed under `/books/<asin>/files`.                  |
| Mixed format inside one folder (.m4b + .mp3)         | First (alpha-sorted) drives format detection; all files still downloadable.                               |
| Disk cache write fails (out of space, ro filesystem) | Pipeline still returns freshly decoded bytes; `/healthz` flips to a warning state with the cause.         |
| Unknown enum value (`BookStatus`/`Role`/etc.)        | Rendered as `Unknown(N)`; WARN log; book otherwise functional.                                            |
| `__EFMigrationsHistory` head unknown                 | Admin write path disabled with a banner; reads continue.                                                  |

Implementation rule of thumb: every fallible operation in the render
path returns `Result<T, RenderHint>` where `RenderHint` is a short
note attached to the page rather than an aborted response. Logs carry
the technical detail. Read paths never `unwrap()`; write paths can fail
loudly because the admin is watching.

## Repo layout

```
libation-webviewer/
├── Cargo.toml
├── Cargo.lock
├── src/
│   ├── main.rs              # axum router + state init + tracing setup
│   ├── db.rs                # rusqlite queries, enum mappings, schema check
│   ├── fs.rs                # book folder scanner, ASIN parser
│   ├── cover.rs             # mp4ameta + id3 extraction, image resize, cache I/O
│   ├── auth.rs              # plaintext-password verify + in-memory session map
│   ├── view.rs              # BookView assembly + askama context structs
│   └── routes/
│       ├── mod.rs
│       ├── library.rs       # list + filter + HTMX partials
│       ├── book.rs          # detail, cover, thumb, files, download
│       ├── admin.rs         # login + requeue handlers
│       └── health.rs        # /healthz
├── templates/               # askama .html files
├── assets/                  # CSS, htmx.min.js, placeholder cover (embedded)
├── tests/
│   ├── fixtures/
│   │   ├── sample.db        # trimmed Libation DB (~10-20 books, anonymised)
│   │   ├── tiny.m4b         # ~50 KB synthetic m4b with known covr (regen script)
│   │   └── books/           # fake on-disk tree mirroring layout
│   ├── db_queries.rs        # against fixtures/sample.db
│   ├── cover_pipeline.rs    # round-trip extract+resize from tiny.m4b
│   ├── http_handlers.rs     # axum oneshot tests
│   ├── admin_writes.rs      # writable copy of sample.db; verify only BookStatus changes
│   ├── concurrency.rs       # parallel writer simulating Libation under WAL
│   └── golden_html/         # snapshot tests for askama output
├── compose/
│   ├── compose.yaml         # generic upstream stack (the README's default)
│   └── README.md            # env-var reference for the generic stack
├── examples/
│   └── dockge/              # the author's specific Dockge install
│       ├── compose.yaml
│       ├── caddy.snippet
│       └── README.md        # walkthrough for this exact setup
├── Dockerfile
├── justfile                 # build, test, fixtures, image
├── .github/
│   ├── workflows/
│   │   ├── ci.yaml             # fmt + clippy + nextest + llvm-cov + audit + deny
│   │   ├── codeql.yaml         # CodeQL (Rust) → Security tab
│   │   ├── release.yaml        # build, push, Trivy scan, SBOM, cosign sign
│   │   └── schedule-tests.yaml # nightly re-run of CI on main
│   ├── dependabot.yml          # cargo + actions + docker bumps (grouped)
│   ├── CODEOWNERS
│   ├── SECURITY.md
│   ├── pull_request_template.md
│   └── ISSUE_TEMPLATE/
│       ├── bug_report.md
│       └── feature_request.md
└── README.md
```

## Test strategy (first-class requirement)

The user's bar: green tests must be enough to auto-bump dependencies and
republish a Docker image. That requires (a) coverage across every layer
that could silently break on a lib upgrade, and (b) tests that don't
depend on network or absolute paths.

### Hard rules

- All tests run offline. No `m.media-amazon.com` calls; the audible-CDN
  provider is mocked at the trait boundary.
- No reliance on a system-installed sqlite, ffmpeg, exiftool, etc. The
  Rust binary's deps are the test surface.
- Every test uses a temp dir for `/cache` (`tempfile::TempDir`).
- Tests run in parallel via `cargo nextest`.
- Coverage is measured with `cargo-llvm-cov` and reported in CI (lcov
  artifact + a PR comment summarising the delta). **No fail threshold.**
  Reviewers are expected to push back on PRs that add code without
  matching tests; the bar is reviewer-enforced rather than numeric.

### Test fixtures

- **`tests/fixtures/sample.db`** — a trimmed, anonymised copy of the real
  Libation DB. ~10–20 books across the interesting cases (single author,
  multi-narrator, audible-plus, series, supplement, AbsentFromLastScan).
  Account field rewritten to `test@example.com`. Generated by a
  `just fixtures-db` script that takes a source DB and applies a
  trimming SQL.
- **`tests/fixtures/tiny.m4b`** — ≤100 KB m4b with one second of silence
  and a known JPEG in `covr`. Generated by a `just fixtures-m4b` script
  using `ffmpeg` (developer-side only; the binary file is committed so
  CI doesn't need ffmpeg).
- **`tests/fixtures/tiny.mp3`** — ≤30 KB MP3 with a known JPEG in an
  ID3v2 `APIC` frame. Same `just fixtures-mp3` script. MP3 is a
  first-class format from v1.
- **`tests/fixtures/no_cover.m4b`** — same audio as `tiny.m4b` but with
  the `covr` atom stripped. Drives the "no embedded cover → placeholder
  + `.miss` sentinel" path.
- **`tests/fixtures/no_cover.mp3`** — MP3 with no `APIC` frame; same
  fallback path for the id3 dispatch.
- **`tests/fixtures/corrupt.m4b`** — truncated MP4 box header. Verifies
  the parser-error fallback.
- **`tests/fixtures/books/`** — fake on-disk tree mirroring
  `<title> [<ASIN>]/...`, exercising:
  - one book with full `metadata.json`
  - one book without `metadata.json`
  - one book with a corrupt `metadata.json`
  - one folder whose name has no `[ASIN]` token (must be ignored)
  - one DB row whose folder is absent (missing-files badge)
  - one folder with no DB row (orphan view)
  - one folder containing a split book (multiple `.m4b` files)
  - one folder containing an `.mp3` book (format dispatch)

### Layers covered

| Layer                  | Test                                                                |
| ---------------------- | ------------------------------------------------------------------- |
| DB queries             | Run each query against `sample.db`; assert row shapes and counts.    |
| Enum mapping           | Unit tests for `TryFrom<i32>` on each enum, incl. unknown variants. |
| Schema-drift check     | Two DBs: known-good and one with a fabricated migration id.         |
| ASIN parser            | Property test (`proptest`): random strings, only valid ASINs match. |
| Folder scanner         | Walk fake `books/`; assert (folder → ASIN → files) map.             |
| Cover extraction       | `tiny.m4b` and `tiny.mp3` → bytes; sha256 matches expected per format. |
| Format dispatch        | `.m4b`/`.m4a`/`.mp4` → mp4ameta; `.mp3` → id3; correct route taken. |
| Graceful degradation   | One test per row of the degradation matrix; assert response is 2xx and the documented fallback (badge, placeholder, hint) is present. |
| Thumbnail resize       | Deterministic resize; sha256 matches expected per-platform vector.  |
| Cache hit / miss / staleness | mtime+size change invalidates and rewrites; `.miss` honored.  |
| Auth gate              | Correct password → token in map + cookie set; wrong password → 401; expired token → 401; constant-time compare verified via timing harness. |
| HTTP read handlers     | `oneshot` axum requests for every GET; status, body, headers.       |
| HTTP write handlers    | With/without admin session; success path; verify only `BookStatus` row is touched. |
| Admin write isolation  | After `requeue`, only `BookStatus` differs in the row vs. baseline. |
| WAL co-tenancy         | Spawn a thread holding a write lock on a different row; the viewer's `UPDATE` still completes within `busy_timeout`. Belt-and-braces given the v2 "pause while Libation scans" enhancement isn't shipping in v1. |
| Golden HTML            | Snapshot render of library and detail templates with fixture data.   |
| End-to-end smoke       | Boot the binary against fixtures via `tokio::spawn`; `reqwest`       |
|                        | against `/`, `/books/<asin>`, `/books/<asin>/thumb`, `/healthz`.     |

### What is NOT tested

- Real Libation. We cannot embed a Libation instance in CI. Schema
  changes are caught only by the migration-id guard at runtime; CI tests
  catch behaviour against the *baked-in* schema.
- Actual Caddy wiring. The plan ships a Caddy snippet but its correctness
  is the operator's smoke test, not a unit test.

## CI / CD (GitHub-native)

The full pipeline is GitHub Actions on `ubuntu-latest`. Every workflow
follows house-style hardening: actions pinned by commit SHA (Dependabot
keeps them bumped), explicit `permissions:` block per job at minimum
viable scope, `concurrency:` to cancel superseded runs.

### `ci.yaml` (PR + push to main)

Matrix: stable Rust.

1. Checkout (`actions/checkout` pinned by SHA)
2. `Swatinem/rust-cache@v2` for `~/.cargo` + `target/`
3. `cargo fmt --check`
4. `cargo clippy --all-targets -- -D warnings`
5. `cargo nextest run --all`
6. `cargo llvm-cov nextest --lcov --output-path lcov.info` → uploads
   the artifact and posts a coverage-delta comment on the PR
   (`romeovs/lcov-reporter-action` or equivalent). No failure
   threshold; this is informational. Reviewers enforce "tests for new
   code".
7. `cargo audit` (RustSec advisory DB)
8. `cargo deny check` (licences + duplicate crates + sources)

### `codeql.yaml` (PR + push + weekly schedule)

GitHub's native semantic code-analysis pipeline:

- `github/codeql-action/init@v3` with `languages: rust`
- `github/codeql-action/analyze@v3` → SARIF auto-uploaded to the
  repo's Security → Code scanning tab. Findings appear inline on PRs.
- Weekly cron so a new query pack catches regressions even on quiet
  weeks.

### `release.yaml` (push to main, manual dispatch)

1. Re-run the test suite (belt and braces).
2. `docker buildx build --platform linux/amd64,linux/arm64` against a
   musl-static Rust target. arm64 is first-class because Raspberry Pi
   is a common Libation host and therefore a natural viewer host too.
3. Push to `ghcr.io/<owner>/libation-webviewer` with tags `:latest`,
   `:sha-<7>`, and `:date-YYYY-MM-DD`.
4. `aquasecurity/trivy-action` scans the built image; SARIF uploaded
   to Security → Code scanning. CRITICAL findings fail the workflow.
5. `anchore/sbom-action` (Syft) emits a CycloneDX SBOM; attached to
   the GitHub Release as an asset.
6. `sigstore/cosign-installer` + `cosign sign` (keyless OIDC,
   GitHub-issued identity) signs the image and attests the SBOM.
   Verification is documented in the README so downstream users can
   verify without sharing keys.
7. `release-drafter` updates the draft release notes from PR labels.

### Auto-bump loop (Dependabot, GitHub-native)

`.github/dependabot.yml` opens grouped PRs for:

- `cargo` (weekly, grouped by `patch`/`minor`/`major`)
- `github-actions` (weekly; bumps the SHA pins so the pin-by-SHA
  policy doesn't bit-rot)
- `docker` (weekly; tracks the distroless base)

Auto-merge on green (via the GitHub-native auto-merge button, enabled
by a tiny `peter-evans/enable-pull-request-automerge` step) is wired
for `patch` and `minor` bumps. `major` bumps stay open for human
review. Because `release.yaml` triggers on `push: main`, an
auto-merged Dependabot PR turns straight into a new image — exactly
the "tests green → republish" loop the requirement calls for.

### Other GitHub-native security & hygiene

- **Dependabot security alerts** + **secret scanning** + **push
  protection** are enabled at the repo level (documented in
  `SECURITY.md`).
- **Branch protection** on `main`: linear history, required status
  checks = `ci`, `codeql`, and `release` re-run; PR review required
  for non-Dependabot contributions.
- **CODEOWNERS** assigns reviews automatically.
- **PR / issue templates** under `.github/` to keep external
  contributions structured.
- `permissions: read-all` at the top of every workflow with
  per-job escalation, plus `id-token: write` only on the signing
  job. Minimises the blast radius if any single action gets
  compromised.

### `schedule-tests.yaml` (nightly)

Re-runs `ci.yaml` against `main` even without commits. Catches a
transitive break (new RustSec advisory, distroless base change) within
24h, before the next Dependabot cycle would notice.

## Dockerfile (sketch)

```dockerfile
# ---- builder ----
FROM rust:1-bookworm AS builder
RUN rustup target add x86_64-unknown-linux-musl \
 && apt-get update && apt-get install -y --no-install-recommends musl-tools
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY templates/ templates/
COPY assets/ assets/
RUN cargo build --release --target x86_64-unknown-linux-musl --bin webviewer

# ---- runtime ----
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/webviewer /webviewer
ENV CACHE_DIR=/cache
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/webviewer"]
```

Final image ~12–20 MB. `nonroot` is uid 65532 — the host paths must be
readable by that uid (covered below).

## Deployment: generic vs. opinionated

Two deployment recipes ship in the repo so external users get a clean
"docker run and go" path while the author's own setup stays
maintainable in the same place.

### Generic (`compose/compose.yaml`) — the README's default

Minimal stack with no opinions about reverse proxy, network names, or
TLS. Publishes a port on the host and lets the operator handle the
rest.

```yaml
services:
  libation-webviewer:
    image: ghcr.io/<owner>/libation-webviewer:latest
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      LIBATION_DB:         /data/libation-config/LibationContext.db
      LIBATION_BOOKS:      /data/books
      CACHE_DIR:           /cache
      ENABLE_ADMIN:        "false"                  # opt-in
      LIBATION_DB_RW:      ${LIBATION_DB_RW:-}      # blank = read-only deployment
      ADMIN_PASSWORD:      ${ADMIN_PASSWORD:-}      # plain text; unset = no-auth admin (only safe on a trusted network)
    volumes:
      - ${LIBATION_CONFIG_DIR}:/data/libation-config:ro
      - ${LIBATION_CONFIG_DIR}:/data/libation-config-rw   # ignored if writes disabled
      - ${LIBATION_BOOKS_DIR}:/data/books:ro
      - ./cache:/cache
    user: "65532:${LIBATION_GID:-65532}"
    read_only: true
    tmpfs:
      - /tmp
```

Defaults make the generic stack safe: writes are off until both
`ENABLE_ADMIN=true` and `ADMIN_PASSWORD` are set (or the operator
explicitly opts into the no-auth mode). The double bind-mount is harmless
when the write path isn't used; the app simply never opens the RW handle.

### Opinionated (`examples/dockge/`) — the author's specific Dockge install

The author's stack, kept in the repo as a worked example. It assumes:

- Dockge under `/opt/stacks/libation-webviewer/`
- A shared external `proxy` network with Caddy attached
- Wildcard TLS for an internal domain via Caddy
- VPN-only reachability (no public ingress)
- `.env` files at `chmod 640`, owner `root:docker`

`examples/dockge/compose.yaml`:

```yaml
services:
  libation-webviewer:
    image: ghcr.io/<owner>/libation-webviewer:latest
    restart: unless-stopped
    environment:
      LIBATION_DB:         /data/libation-config/LibationContext.db
      LIBATION_DB_RW:      /data/libation-config-rw/LibationContext.db
      LIBATION_BOOKS:      /data/books
      CACHE_DIR:           /cache
      ENABLE_ADMIN:        "true"
      ADMIN_PASSWORD:      ${ADMIN_PASSWORD:-}        # plain text; blank = anonymous admin (VPN-only fine)
    volumes:
      - ${LIBATION_CONFIG_DIR}:/data/libation-config:ro
      - ${LIBATION_CONFIG_DIR}:/data/libation-config-rw
      - ${LIBATION_BOOKS_DIR}:/data/books:ro
      - ./cache:/cache
    networks:
      - proxy
    read_only: true
    tmpfs:
      - /tmp
    user: "65532:${LIBATION_GID:-65532}"

networks:
  proxy:
    external: true
```

`examples/dockge/caddy.snippet`:

```caddy
books.<internal-domain> {
    reverse_proxy libation-webviewer:8080
}
```

Paired README under `examples/dockge/README.md` walks through:
locating Libation's host paths, picking `LIBATION_GID`, building the
`.env`, adding the Caddy snippet, and the first-run rehearsal.

### Notes shared by both flavours

- `LIBATION_GID` is the host gid of the user Libation runs as; the
  viewer joins that gid so writes through the `-rw` mount land with
  the correct ownership. Leaving it at the default 65532 only works
  by coincidence. The generic README must call this out clearly.
- Both stacks keep the read-only handle as the default and the
  writable handle as a second mount the app opens lazily inside admin
  write transactions. No `CAP_SYS_ADMIN`, `--privileged`, or socket
  access needed.
- For defence-in-depth, an optional Caddy `basic_auth` block is
  documented in both READMEs; not enabled by default.

## Phase-0 host verification (before the first real deploy)

These items the planning conversation could *not* verify against the
real system. They must be ticked off on the operator's host:

1. **Locate** the actual Libation `<LIBATION_CONFIG_DIR>` and
   `<LIBATION_BOOKS_DIR>` from the existing Dockge stack at
   `/opt/stacks/libation/compose.yaml`.
2. **Confirm uid/gid** of the Libation container's writer. Set
   `LIBATION_GID` accordingly.
3. **Confirm the `BookStatus` enum mapping** by reading Libation's source
   on GitHub (`LibationFileManager` / `DataLayer`) — specifically
   that `0` means "queue for download" in Libation's scan loop. This is
   the single most important external assumption.
4. **Confirm the `__EFMigrationsHistory` head** of the production DB and
   add it to the known-good list shipped in the binary.
5. **First write rehearsal**: pick a book the user is willing to
   re-download, hit `/books/<asin>/requeue` from an admin session, wait
   for the next Libation scan, confirm Libation actually re-downloads.
   Document the observed behaviour in the README.
6. **Format spot-check**: confirm the disk mix is m4b-only or m4b+mp3.
   Both are first-class, but if anything else shows up (e.g. .ogg,
   .flac) the cover-extractor trait gets a new impl; not a v1 blocker.

## Open items / future work (explicitly out of v1)

- **Audible CDN cover provider** — scaffolded via the `CoverProvider`
  trait but not wired into the chain. Add when the embedded path can't
  serve a book (e.g. a corrupt download).
<!-- Shipped: PDF supplements are now surfaced on the book detail page. -->

- **Multi-account UI affordance** — `LibraryBooks.Account` could become
  a filter when more than one distinct Account exists in the DB.
- **Mark-as-finished toggle** — `UserDefinedItem.IsFinished` is a
  natural second write target; same mechanics as `BookStatus`. Defer.
- **User tag editing** — `UserDefinedItem.Tags` is freeform text;
  editing it from the viewer is feasible with the same write path.
  Defer.
- **`FileLocations.json` adapter** — if the operator confirms Libation
  authoritatively writes it, swap the on-disk scanner for a JSON reader
  behind the same trait.

## End-to-end verification plan

After the first build:

1. `just test` — full nextest run + coverage report (no threshold).
2. `just image` — builds the multi-arch image locally.
3. `docker compose up -d` in `compose/` against a copy of the real
   Libation paths.
4. `curl https://books.<internal-domain>/healthz` from the VPN.
5. Browse `/`, sort by date added, filter by series, open a detail page,
   download an m4b, confirm covers render and thumbnails are crisp.
6. Set `ADMIN_PASSWORD`, log in, click "requeue" on a book the user
   is willing to re-download. Watch the Libation logs on the next scan.
7. Tear down the cache (`rm -rf ./cache && docker compose restart`) and
   confirm cold start succeeds, no data loss, no Libation impact.

## Critical files to touch (or create)

- `src/db.rs` — queries, enums, schema check.
- `src/fs.rs` — folder scanner + ASIN parser.
- `src/cover.rs` — mp4ameta/id3 extract → resize → cache.
- `src/auth.rs` — plaintext-password verify + in-memory session map.
- `src/main.rs` — axum router + state init.
- `src/routes/admin.rs` — login + requeue handlers.
- `templates/library.html`, `templates/book.html`,
  `templates/_admin_bar.html` — askama HTMX templates.
- `tests/fixtures/sample.db`, `tests/fixtures/tiny.m4b`,
  `tests/fixtures/tiny.mp3`, `tests/fixtures/books/...` — fixtures.
- `compose/compose.yaml`, `examples/dockge/compose.yaml`,
  `examples/dockge/caddy.snippet`, `Dockerfile`, `justfile`,
  `.github/workflows/*` — deployment + CI.
