# libation-webviewer

A small self-hosted web viewer for a Dockerized [Libation](https://github.com/rmcrackan/Libation) audiobook library.

Browse, sort, filter, download, and — admin-gated — re-queue books so Libation will re-download them on its next scan. Read-only against Libation's DB and books by default; the single legitimate write target is `UserDefinedItem.BookStatus = 0`.

## Status

Early development. The repo currently holds the [design doc](./PLAN.md)
plus a compiling scaffold. The HTTP surface is stubbed; the DB, cover
pipeline, and admin write path are not yet wired up. Vertical-slice
work tracked in issues.

## Quick start (Docker)

```sh
docker run -d \
  -p 8080:8080 \
  -v /path/to/libation/config:/data/libation-config:ro \
  -v /path/to/libation/books:/data/books:ro \
  -v "$(pwd)/cache":/cache \
  -e LIBATION_DB=/data/libation-config/LibationContext.db \
  -e LIBATION_BOOKS=/data/books \
  -e CACHE_DIR=/cache \
  --user "65532:$(stat -c %g /path/to/libation/config)" \
  ghcr.io/yolgie/libation-webviewer:latest
```

Open <http://localhost:8080>.

Compose? See [`compose/`](./compose/).
Author's specific Dockge + Caddy stack? See [`examples/dockge/`](./examples/dockge/).

## Features (planned for v1)

- Read-only browse list with sort/filter, cover art, detail pages, download links
- `.m4b` and `.mp3` first-class (embedded `covr` / `APIC` cover atoms)
- Admin-gated re-queue → Libation re-downloads on its next scan
- Graceful degradation when metadata, cover, or files are missing
- Multi-arch image (amd64 + arm64) signed with `cosign`, shipped with a CycloneDX SBOM

## Configuration

| Variable               | Default          | Purpose                                                                                        |
| ---------------------- | ---------------- | ---------------------------------------------------------------------------------------------- |
| `LIBATION_DB`          | _required_       | Path to `LibationContext.db` inside the container                                              |
| `LIBATION_BOOKS`       | _required_       | Path to the books folder inside the container                                                  |
| `CACHE_DIR`            | `/cache`         | Writable dir for the cover/thumb cache                                                         |
| `LISTEN_ADDR`          | `0.0.0.0:8080`   | Bind address                                                                                   |
| `ENABLE_ADMIN`         | `false`          | Mount the admin routes                                                                         |
| `ADMIN_PASSWORD`       | _unset_          | Plain-text password. Empty + `ENABLE_ADMIN=true` = anonymous admin (only on a trusted network) |
| `LIBATION_DB_RW`       | _unset_          | Writable path to the same SQLite file; unset = admin writes are no-ops                         |
| `ALLOW_UNKNOWN_SCHEMA` | `false`          | Allow admin writes even when the EF Core migration head isn't in the known-good list           |
| `RUST_LOG`             | `info`           | Standard `tracing-subscriber` filter                                                           |

## Development

```sh
mise install        # provisions Rust + cargo dev tools (see mise.toml)
just bootstrap      # sqlite3, file, ffmpeg, musl-tools via apt
just ci             # full local CI check (fmt + clippy + tests + coverage + audit + deny)
just test           # tests only
just run            # local cargo run
just image          # multi-arch docker build
```

## Changelog

See [`CHANGELOG.md`](./CHANGELOG.md). The `Unreleased` section is updated
in the same commit as each change.

## Licence

MIT. See [`LICENSE`](./LICENSE).
