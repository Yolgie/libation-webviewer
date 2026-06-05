# Generic Docker Compose stack

Drop-in stack for any host. Edit `.env` (sibling of this file) to set:

| Variable              | Required | Purpose                                                                                  |
| --------------------- | -------- | ---------------------------------------------------------------------------------------- |
| `LIBATION_CONFIG_DIR` | yes      | Host path to Libation's config dir (contains `LibationContext.db`)                       |
| `LIBATION_BOOKS_DIR`  | yes      | Host path to Libation's books folder                                                     |
| `LIBATION_GID`        | yes      | Group ID of the user Libation runs as on the host. Used so the viewer can write back.    |
| `ENABLE_ADMIN`        | no       | `true` to expose the admin surface (write-back to Libation's DB). Defaults to `false`.   |
| `ADMIN_PASSWORD`      | no       | Plain-text password gating the admin surface. Empty + `ENABLE_ADMIN=true` = anonymous.   |
| `LIBATION_DB_RW`      | no       | When set, the app uses it as the writable DB handle. Leave empty for a read-only deploy. |

Start it:

```
docker compose up -d
```

The viewer listens on port `8080`. Put your own reverse proxy (Caddy,
Traefik, nginx) in front to add TLS.

For the author's own Dockge + Caddy `proxy` network deployment, see
`../examples/dockge/`.
