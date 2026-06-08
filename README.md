# libation-webviewer

A small self-hosted web viewer for a Dockerized [Libation](https://github.com/rmcrackan/Libation) audiobook library.

Browse, sort, filter, download, and — admin-gated — re-queue books so Libation will re-download them on its next scan. Read-only against Libation's DB and books by default; the single legitimate write target is `UserDefinedItem.BookStatus = 0`.

## Status

v1 feature-complete. The library list, per-book detail, cover
pipeline, live books-folder scan, admin requeue, graceful degradation,
and `/healthz` DB ping are all wired up and covered by integration
tests. The [design doc](./PLAN.md) is the source of truth for the
threat model and the open ops decisions. End-to-end HTMX browser tests
are tracked separately in
[`docs/e2e-tests-plan.md`](./docs/e2e-tests-plan.md).

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

## Deployment topology

The intended topology is **a TLS-terminating reverse proxy in front
of the viewer container**. The author's reference stack is Caddy with
Let's Encrypt:

```
browser  ──HTTPS──▶  Caddy (LE certs)  ──HTTP──▶  libation-webviewer
```

A minimal `Caddyfile` looks like:

```caddyfile
library.example.org {
    reverse_proxy libation-webviewer:8080
    # Optional but recommended: bound login brute-force attempts.
    # (Requires the caddy-ratelimit module.)
    rate_limit {
        zone admin_login {
            key {remote_host}
            events 10
            window 1m
        }
        match {
            path /admin/login
            method POST
        }
    }
}
```

Two consequences for the viewer:

- **Session cookies are issued with `Secure`** (i.e. browsers will only
  send them back over HTTPS). Serving the viewer over plain HTTP will
  silently drop the cookie and break the admin flow.
- **Login rate-limiting is delegated to the reverse proxy** by design.
  The viewer doesn't ship an in-app rate limiter, on the assumption
  that the deployer has Caddy (or nginx, Traefik, etc.) doing the
  edge-level work. If you can't run a proxy with rate-limiting,
  protect the viewer with a private network instead.

For long-lived sessions across container restarts, set
`SESSION_SECRET` (see [Configuration](#configuration)). Without it
the HMAC key is generated at startup and sessions drop on restart —
the same effective behaviour as the original in-memory session map.

## Features

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
| `SESSION_SECRET`       | _unset_          | 64-char hex (32 bytes) anchoring the HMAC that signs session cookies. Unset = a key is generated at startup and sessions drop on restart |
| `LIBATION_DB_RW`       | _unset_          | Writable path to the same SQLite file; unset = admin writes are no-ops                         |
| `ALLOW_UNKNOWN_SCHEMA` | _unset_          | When set to `1`/`true`, allow admin writes even when the EF Core migration head isn't in the known-good list shipped in the binary. Logged at WARN with the actual head |
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
