# End-to-end HTMX test plan

Status: not started. This file captures the design so a future slice
can pick it up without re-discovering the trade-offs.

## Gap

The integration tests in `tests/handlers.rs` and `tests/admin.rs` cover
the HTTP layer: they render templates, inspect status codes, and
string-match on the HTML for HTMX attributes (`hx-get`, `hx-target`,
`hx-post`) and the `HX-Push-Url` response header. That gives us
confidence that the server *emits* the right HTMX wiring.

What those tests cannot verify:

- The browser actually performs the swap when the user interacts.
- The swapped fragment slots into the correct target without breaking
  the surrounding DOM.
- `HX-Push-Url` updates the address bar, and a refresh reproduces the
  filtered view.
- The HTMX library version we ship in `assets/static/` is compatible
  with the attributes we emit.
- CSS / progressive-enhancement: the page is still usable with JS off
  (a stated v1 nice-to-have).

A real browser is the only way to close this gap.

## Approaches

### Option A — fantoccini (Rust + geckodriver)

- WebDriver client written in Rust.
- Talks to a `geckodriver` (Firefox) or `chromedriver` (Chromium)
  process.
- Tests live in `tests/e2e/` and run under `cargo test`.
- Dev/CI must provide the driver binary and a headless browser.

Pros: stays in the Rust toolchain; tests run with the existing
`cargo test` / `just test` flow; no Node dependency.

Cons: smaller ecosystem than playwright; fewer affordances for
flakiness (auto-wait, retries, traces).

### Option B — playwright (Node + Chromium/Firefox/WebKit)

- Mature browser-automation library; bundles its own browser binaries.
- Tests live in a top-level `e2e/` directory and run with
  `npx playwright test`.
- Dev/CI must provide Node and let playwright install its browsers.

Pros: best-in-class debugging (trace viewer, video on failure,
auto-wait); cross-browser; large community.

Cons: introduces Node to the dev and CI matrix; second test runner to
keep working.

### Recommendation

Start with **fantoccini**. Two reasons:

1. The crate ships as a single Rust binary; adding Node to CI for a
   handful of smoke tests is heavy.
2. The behaviour we want to verify is narrow — HTMX swaps and address
   bar updates — and fantoccini's `WebDriver::find` / `wait_for_find`
   covers it without needing playwright's richer toolkit.

Revisit if the suite grows past ~10 cases or we start needing trace
artifacts on flakes.

## Recommended starter scope

Three smoke tests that mirror the existing handler-level coverage but
verify the browser-side outcome:

1. **Library filter swap.** Load `/`, type into the search input,
   confirm the `<tbody>` (or list container) swaps to the filtered set
   and that the URL bar reflects `?q=…`.
2. **Detail page files fragment.** Load `/books/{asin}`, confirm the
   files list arrives via the deferred `hx-get="/books/{asin}/files"`
   into `#files-list`.
3. **Admin requeue swap.** Log in with the test password, click the
   requeue button on a detail page, confirm the button state changes
   (e.g. is disabled / shows confirmation) and the detail row reflects
   `BookStatus = 0`.

Each test boots the same `routes::router(state)` we already use in
handler tests, against a fixture DB and a tempdir books root, on a
random port. The test then drives the browser at
`http://127.0.0.1:<port>/`.

## CI / dev environment changes

- `Justfile`: add a `just e2e` recipe that starts `geckodriver` and
  runs `cargo test --test e2e -- --test-threads=1`.
- `.github/workflows/`: a separate job that installs Firefox +
  geckodriver before running `just e2e`. Keep it off the critical
  `just test` path so a flaky browser test doesn't block normal PRs;
  fail the e2e job non-blocking until it has been stable for a week.
- `mise.toml`: pin the geckodriver version so local and CI stay in
  sync.

## File layout

```
tests/
  e2e/
    common.rs        boots router on random port, opens fantoccini
                     client, returns (client, base_url, teardown)
    library.rs       library filter swap test
    detail.rs        files fragment swap test
    admin.rs         admin requeue swap test
```

`tests/e2e/common.rs` is shared via a `mod common;` declaration in each
test file — Rust's standard pattern for shared integration-test
helpers.

## Open questions

- Does the existing fixture DB at `tests/fixtures/sample.db` work for
  e2e or do we need a smaller curated one? (64 books is a lot to load
  through a real browser per test.)
- Do we wait for the upstream HTMX version we ship to provide a stable
  hook for "swap complete", or do we poll for the swapped DOM marker?
  fantoccini's `wait_for_find` covers the polling case but introduces
  one more tunable timeout per test.
- Should the admin requeue test run against the real RW handle (and
  reset the DB between tests) or use a per-test temp copy? Temp copy
  is simpler and matches the existing handler test pattern.
