# ADR-0006: One Cargo workspace, one binary

- Status: Accepted
- Date: 2026-10-11

## Context

We want a CLI first and a web version later, with the same compression results in both. If the two had separate code paths, fixes in one would miss the other.

## Decision

One Cargo workspace with three crates:

- `squish-core` is a library with no file I/O, no network and no printing. It takes bytes and options and returns bytes and stats.
- `squish-server` is a library with the axum routes and the embedded web UI.
- `squish` is the binary. It owns the CLI and exposes the server as `squish serve`.

The web UI files are embedded into the binary at build time.

## Consequences

- One file to install, copy or put in a `FROM scratch` Docker image.
- Every change to the core shows up in both the CLI and the web version.
- The core can be unit-tested without touching the disk.
- People who only want the CLI still download the server code. We can add a cargo feature to leave it out if the size matters.

## Alternatives considered

- **Separate `squish` and `squish-server` binaries.** Smaller binaries, two things to ship and version.
- **One crate, no workspace.** Simpler to start, but the core would end up depending on clap and axum.
