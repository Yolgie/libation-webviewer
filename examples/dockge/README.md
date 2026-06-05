# Author's Dockge stack

The setup the author runs personally. Other users almost certainly want
`compose/` instead; this exists as a worked example for a specific
homelab topology.

## Assumptions

- **Dockge** manages stacks under `/opt/stacks/<name>/`.
- A shared external Docker network named `proxy` exists, with Caddy
  attached, terminating TLS using a wildcard cert for an internal
  domain.
- Reachable only from the operator's VPN. No public ingress.
- `.env` files at `chmod 640`, owner `root:docker`.

## Deploy

1. Drop this directory into `/opt/stacks/libation-webviewer/`.
2. Create `.env` alongside `compose.yaml` with:
   ```
   LIBATION_CONFIG_DIR=/opt/stacks/libation/config
   LIBATION_BOOKS_DIR=/opt/stacks/libation/books
   LIBATION_GID=<gid of the libation container's user>
   ADMIN_PASSWORD=<something>
   ```
3. Append the contents of `caddy.snippet` to your Caddyfile and reload
   Caddy.
4. `docker compose up -d` (Dockge does this for you).
5. Browse to `https://books.<internal-domain>`.

## First write rehearsal

Before relying on the admin "re-queue" button, do one round-trip:

1. Log in, pick a book you're willing to re-download, click re-queue.
2. Wait for the next scheduled Libation scan (typically nightly).
3. Confirm Libation actually re-downloads the file.

Document the outcome here once verified.
