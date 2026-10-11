# ADR-0011: Never return a larger file

- Status: Accepted
- Date: 2026-10-11

## Context

Re-encoding a file that's already optimized can make it bigger. That happens with CDN WebP, small palette PNGs, or JPEGs saved at low quality. A compressor that sometimes grows files is worse than useless.

## Decision

After encoding, the core compares sizes. If the result is greater than or equal to the original, it returns the original bytes unchanged and sets `kept_original = true` on the output.

Cheap checks run before encoding to skip work that can't win. Examples are a PNG that's already a small palette image, or a JPEG whose estimated quality is already below our target.

A test asserts the size rule on every file in the benchmark set.

## Consequences

- Users never lose bytes, so "compress everything in this folder" is always safe.
- The CLI summary and the web UI report these files as "already optimized".
- Encoding still happens for borderline files, and that time is spent for nothing. The early checks reduce how often.

## Alternatives considered

- **Always return the re-encoded file.** Simpler, but it breaks user trust.
