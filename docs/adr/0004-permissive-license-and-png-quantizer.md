# ADR-0004: Permissive license, quantette as the PNG quantizer

- Status: Accepted
- Date: 2026-10-11

## Context

Most PNG savings come from reducing a 24-bit or 32-bit image to a palette of at most 256 colors, with dithering. The best library for this is `imagequant` (the engine inside pngquant). It's licensed GPL-3.0-or-later, with a paid commercial license. Linking it into squish would make every distributed binary GPL.

We want squish under a permissive license (MIT OR Apache-2.0) so it can be used anywhere, including in closed-source or paid products later.

`quantette` is a permissively licensed Rust quantizer. It's younger, and we haven't measured its quality against imagequant yet.

## Decision

1. squish is licensed MIT OR Apache-2.0.
2. Ship `quantette` as the PNG quantizer.
3. Put the quantizer behind a `Quantizer` trait so the PNG pipeline doesn't know which implementation it's calling.
4. In Phase 0, benchmark quantette against imagequant. imagequant is used only in the benchmark program and is never a dependency of a shipped crate.
5. Confirm in Phase 0 that quantette handles transparency (RGBA input with a palette that includes alpha). Its docs confirm Floyd-Steinberg dithering but say nothing about alpha (checked against quantette 0.6.0 on 2026-10-11). Logos and icons with transparency are a common PNG case, so this is a hard requirement.
6. If quantette is clearly worse on our test set (more than ~5% bigger at the same SSIM), or it can't handle alpha, decide between writing our own quantizer and buying the imagequant commercial license. That decision gets its own ADR. Until it's made, PNGs that quantette can't handle go through lossless oxipng only.

## Consequences

- No GPL obligations for anyone using squish.
- PNG output might start a bit bigger than TinyPNG's. Phase 0 tells us by how much.
- The trait makes swapping implementations a one-file change.

## Alternatives considered

- **GPL with imagequant.** Best quality right away. Ties every user and future product to the GPL.
- **Commercial imagequant license.** Best quality and closed source allowed, but it costs money before we know we need it.
- **Write our own quantizer first.** Median cut + k-means + Floyd-Steinberg dithering fits in about 500 lines. Matching imagequant on gradients and alpha is much harder. Kept as the fallback.
