# ADR-0005: mozjpeg for JPEG encoding

- Status: Accepted
- Date: 2026-10-11

## Context

JPEG savings come from re-encoding with an encoder that uses trellis quantization, progressive scans and optimized Huffman tables, then dropping metadata. Three candidates:

- **mozjpeg.** Mozilla's encoder. Most image optimizers use it, and the `mozjpeg` crate builds it from source.
- **jpegli.** Google's newer encoder from libjxl. It can beat mozjpeg at high quality settings. It's C++, and the Rust bindings are less mature.
- **jpeg-encoder.** A pure-Rust encoder. Easy to build and WASM-friendly, but its files are bigger.

## Decision

Use mozjpeg through the `mozjpeg` crate. In Phase 0, try jpegli as well if its bindings build without much effort. If jpegli wins clearly on our test set, write a new ADR.

## Consequences

- Proven output, close to what TinyPNG produces.
- The build needs a C compiler, and `nasm` for SIMD speed.
- The JPEG encoder doesn't compile to plain `wasm32-unknown-unknown`. A future in-browser mode would need Emscripten for JPEG or would fall back to jpeg-encoder.

## Alternatives considered

- **jpegli.** See above. Evaluated in Phase 0, not chosen for now.
- **jpeg-encoder.** Kept as the option for a future WASM build.
- **Lossless-only JPEG optimization** (Huffman optimization and progressive conversion, like jpegtran). It saves only about 5 to 15%. Might become a `--lossless` mode later.
