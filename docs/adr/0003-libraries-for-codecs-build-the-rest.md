# ADR-0003: Use libraries for codecs, build everything around them

- Status: Accepted
- Date: 2026-10-11

## Context

For each part of squish we can either write it ourselves or depend on a library. Codecs hold years of tuning. A baseline JPEG encoder is around 1,500 lines, but mozjpeg's savings come from trellis quantization and tuned progressive scans, which is research-level work. WebP and AVIF sit on top of video codecs (VP8, AV1). On the other side, the logic that decides which encoder to run, with which settings, and when to give up is small, and it's what makes squish its own product.

## Decision

Use libraries for decoding, encoding, lossless PNG optimization, PNG color reduction (ADR-0004) and zip writing.

Build ourselves:

- format detection (magic bytes)
- the per-format pipeline and its settings
- the size guard (ADR-0011) and metadata policy (ADR-0010)
- concurrency (on top of rayon and tokio)
- the CLI (on top of clap)
- the HTTP server (on top of axum)
- the web UI
- later, an automatic quality mode that searches for the lowest quality that stays above an SSIM target

## Consequences

- We reach TinyPNG-level output without writing a codec.
- Our code stays small, so it's easy to test.
- We inherit our dependencies' licenses and build requirements, so license checks are part of adding any codec.

## Alternatives considered

- **Use an all-in-one library such as libcaesium or libvips.** Less code for us. libcaesium pulls in GPL imagequant, which conflicts with ADR-0004. libvips is a large LGPL shared library, which conflicts with the single-binary goal. Both stay useful as references.
- **Write a quantizer ourselves now.** Kept as a fallback in ADR-0004, not the starting point.
