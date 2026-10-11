# ADR-0013: Vanilla HTML and JS for the web UI, embedded in the binary

- Status: Accepted
- Date: 2026-10-11

## Context

The web UI is one page: a drop zone, a list of files with before and after sizes, a download button per file, "Download all" and a settings drawer. We want the binary to stay small and the build to need nothing besides Cargo.

## Decision

- Write the UI in plain HTML, CSS and JavaScript, with no framework, bundler or `node_modules`.
- The only third-party script is the zip library (ADR-0008). We vendor it as a file in `web/`.
- Embed `web/` into the binary at build time with `rust-embed`.

## Consequences

- Building squish needs only Cargo.
- The page is a few tens of kilobytes.
- If the UI grows a lot, plain JS gets harder to maintain. That would be the time to revisit this decision.

## Alternatives considered

- **Svelte or Preact with Vite.** Nicer components, but adds Node.js to the build.
- **HTMX with server-rendered HTML.** Small, but per-file progress and zipping in the browser fit plain JS better.
