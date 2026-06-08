# Observability plan (deferred)

This doc captures the design space for the viewer's observability
slice, which was discovered during the review-feedback-cleanup audit
but explicitly deferred for a separate review. It is a planning doc,
not a decision: each section is an open question.

## Current state

- `main.rs` calls `tracing_subscriber::fmt().with_env_filter(...).init()`
  with `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`.
- Handlers and the db layer emit `info!` / `warn!` / `error!` events
  with structured fields (`?err`, `path = %p.display()`, etc.). These
  land on stdout, which Docker / Caddy / whatever-is-tailing picks up.
- **No request span is created at the router boundary.** A single
  request that touches `routes/book.rs`, `db.rs`, and `fs.rs` produces
  multiple independent log lines with no obvious correlation.
- **`tower-http`'s `trace` feature is enabled** (`Cargo.toml:18`) but
  no `TraceLayer` is mounted on the `Router` in `routes/mod.rs`.
- No metrics endpoint (`/metrics`), no `tracing-opentelemetry`, no
  log-format toggle (JSON vs plain).

## Why this was deferred

The audit flagged this as a "high-value low-churn" item. The user
declined to scope it into the same PR as the auth redesign and DB
pool work because:

1. It's orthogonal to the other slices (no shared files, no shared
   correctness concerns).
2. Format choices (plain vs JSON, span timings yes/no, OTel yes/no)
   are deployer-facing decisions that the author wants to think
   about explicitly.
3. The current logging is *fine* for single-operator homelab use;
   this is upside, not a bug fix.

## Scope options

Each option is independent; they stack.

### Option A — `TraceLayer` only (~5 lines)

Mount `tower_http::trace::TraceLayer::new_for_http()` in
`routes/mod.rs`. Each request gets:

- An auto-generated span at the router boundary (method, URI, version).
- A structured event when the response is sent (status, latency).
- All `error!`/`warn!` events inside handler bodies attach to that
  span automatically.

Downside: the default span/event format is opinionated; some output
fields are noisy in dev (`http.uri.full` repeats the path).

### Option B — Per-handler `info_span!` (above + handler instrumentation)

Add `#[tracing::instrument]` to handler entry points, or wrap the
body in `info_span!("library_list", q = ?q)`. Lets handlers
contribute domain context (the active query, the ASIN being viewed)
to the request span.

Cost: every handler grows a span guard. Worth it if logs are read
during incident response; overkill for "did the server return 200".

### Option C — Log format toggle (`LOG_FORMAT=json`)

Swap the default plain `fmt` formatter for a JSON formatter
(`tracing_subscriber::fmt::format::json()`) when an env var requests
it. Aligns with how Caddy / Loki / Promtail / Datadog ingest logs.

Cost: trivial. Mostly a documentation slice. Open question:
default to JSON in container builds, or stay plain everywhere?

### Option D — `/metrics` endpoint (prometheus)

Add a `metrics-exporter-prometheus` (or equivalent) exporter and
expose `/metrics` for the deployer's scrape config. Tracks:

- Request count by handler + status.
- DB query latency histogram.
- Cover-extraction latency histogram.
- Pool wait time (r2d2 doesn't expose this natively; would need a
  small wrapper).

Cost: a new dependency, a new endpoint that needs the same
admin/auth thinking as `/healthz` (we'd want this restricted to
the LAN or behind Caddy basic-auth).

### Option E — `tracing-opentelemetry` + OTLP export

Full distributed-tracing integration. The viewer is a single
process, so the upside is mostly "spans show up in Grafana
Tempo / Jaeger / whatever the deployer runs" rather than
cross-service correlation.

Cost: meaningful — new deps, new env config, runtime overhead
per span. Hard to justify for a small homelab tool unless the
deployer is already running an OTel collector.

## Open questions for the author

1. **What problem are we actually solving?** Diagnosing a stuck
   request? Spotting requeue failures faster? Producing pretty
   dashboards? The answer picks options A vs B vs D for us.
2. **Plain logs or JSON by default in the container build?** JSON
   is easier to ingest but harder to read on the host with
   `docker logs`.
3. **Is `/metrics` exposed publicly, gated behind the admin
   gate, or only on a separate listener?** If we add it, we need
   to decide before shipping.
4. **Are we OK adding deps?** `tower-http`'s `trace` feature is
   already pulled in; A and B cost nothing new. C is also free.
   D and E add real dep weight (`metrics-exporter-prometheus`,
   `opentelemetry*`).
5. **Do per-handler `info_span!` calls fight Slice 4c's `AppError`?**
   Probably not — `AppError` flows through `?` regardless — but the
   span guard semantics around async boundaries are easy to get
   wrong, and we should write at least one test that proves error
   logs include the request id.

## Recommendation (when this gets picked up)

Start with **Option A (`TraceLayer`)** as a single commit. It is
five lines, gives deployers per-request access logs immediately,
and slots cleanly under the answer to question 1 above (whatever
that turns out to be). Treat B, C, D, E as separate slices to
discuss case-by-case once A is in.
