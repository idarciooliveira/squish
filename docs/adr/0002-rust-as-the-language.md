# ADR-0002: Rust as the only language

- Status: Accepted
- Date: 2026-10-11

## Context

We want a light, fast tool that ships as one binary, first as a CLI and then as a web app. We compared Rust and Go.

The product is compression quality and speed. The best encoders are C or Rust: oxipng, libimagequant, mozjpeg, libwebp, rav1e. Go's standard library JPEG encoder is baseline only (standard Huffman tables, no progressive mode, no trellis quantization), its PNG encoder doesn't reduce colors, and it has no WebP or AVIF encoder. Go reaches the good encoders through cgo, which costs us the easy static binary, or through WASM runtimes like wazero, which are slower than native code.

Go is easier to write, already installed on the dev machine, and its standard library HTTP server is excellent. But the server is a few hundred lines in either language.

## Decision

Write all of squish in Rust: core library, CLI, HTTP server. The browser UI is plain HTML and JS (ADR-0013).

## Consequences

- We link the best codecs directly. The C ones get compiled from source during `cargo build` and linked statically.
- `rayon` gives us parallel batch processing with almost no code.
- The pure-Rust parts of the core can later compile to WASM for an in-browser mode.
- Steeper learning curve and slower compiles than Go.
- The dev machine needs `rustup`, plus `build-essential`, `nasm` and `cmake` for the C codecs.

## Alternatives considered

- **Go + libvips through cgo.** Fast to write and fast to run. It brings a large shared library as a dependency, makes the binary harder to ship, and has no browser reuse.
- **Go, pure.** Easiest build. Clearly bigger output files, which defeats the purpose of the tool.
- **Rust core called from a Go server.** Two toolchains plus FFI. More work than either language alone.
