# ADR-0008: "Download all" builds a STORE zip in the browser

- Status: Accepted
- Date: 2026-10-11

## Context

Users need to download every compressed file at once. Since the server keeps nothing (ADR-0007), the browser already holds every result.

Compressed images don't shrink further under deflate. Deflating them again inside the zip only burns CPU.

## Decision

The browser builds the zip with `fflate` or `client-zip`, in STORE mode (no compression). The CLI's `--zip` option does the same with the `zip` crate.

## Consequences

- Zipping is close to instant and costs the server nothing.
- The library adds a few kilobytes to the page.
- Zip file size equals the sum of the images plus a small header per file.

## Alternatives considered

- **A server endpoint that builds the zip.** The browser would upload every result a second time, or the server would have to store results, which breaks ADR-0007.
- **Deflate inside the zip.** Slower with almost no size gain.
