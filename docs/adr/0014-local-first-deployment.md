# ADR-0014: Local first, ready to harden for public use

- Status: Accepted
- Date: 2026-10-11

## Context

We haven't decided whether the web version will be public. A public site needs rate limits, a privacy note and hosting. A local tool needs none of that.

## Decision

- `squish serve` binds to `127.0.0.1:8080` by default. Exposing it takes an explicit `--host 0.0.0.0`.
- The safety limits ship from day one because they also protect a local machine. They are the body size limit, pixel limit, job semaphore and per-request timeout.
- Per-IP rate limiting, a privacy note and deployment docs wait until we decide to go public. That decision gets its own ADR.

## Consequences

- Safe default. Nobody exposes squish by accident.
- Going public later is mostly configuration and a few middleware layers, not a redesign, because the server is already stateless (ADR-0007).

## Alternatives considered

- **Design for public hosting now.** Work we may never need.
- **Local only, with no limits.** A single oversized image could still exhaust the machine's memory.
