# ADR-0012: AVIF only in Phase 2, and only if needed

- Status: Accepted
- Date: 2026-10-11

## Context

TinyPNG supports PNG, JPEG, WebP and AVIF. AVIF is the most expensive format to encode by a wide margin, often seconds per photo, while the other three take a fraction of that. It's also the format people least often have in a bloated form. Supporting it means an encoder (`ravif`) and a separate decoder (libavif or dav1d), which add build complexity and binary size.

## Decision

- Phase 1 supports PNG and JPEG. Phase 2 adds WebP.
- AVIF is a Phase 2 candidate, built only if needed. It's needed when at least one of these is true:
  - the owner has real AVIF files to compress
  - a user asks for it
  - the Phase 2 benchmark shows meaningful savings on AVIF files from common sources
- Until then, AVIF input gets detected and reported as "not supported yet". The CLI skips it and the server answers `415`. The file is never altered.
- If we build it, it sits behind a cargo feature (`avif`), uses a faster speed preset by default, has a per-image timeout, and counts as several CPU slots on the server.
- Choose the AVIF decoder when the trigger fires, not now. On 2026-10-11 the `libavif` crate had no release since July 2024, while `dav1d` (0.11.x) was maintained.

## Consequences

- Smaller, simpler first releases with a faster build.
- No full format parity with TinyPNG until the trigger fires.
- The feature flag keeps AVIF optional even after we add it.

## Alternatives considered

- **AVIF in Phase 2 unconditionally.** Format parity sooner, paid for with build complexity we may never need.
- **AVIF in Phase 1.** Slows down the first useful release.
