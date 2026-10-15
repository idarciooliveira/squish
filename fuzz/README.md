# Fuzzing squish-core (NFR-6)

One target, `fuzz_compress`, feeds arbitrary bytes through
`squish_core::detect` then `squish_core::compress` with bounded options
(sampled fixed qualities, all efforts, all metadata policies). Both calls
must return `Err` on bad input — never panic or crash. The mozjpeg calls
already run under `catch_unwind` (`crates/core/src/codec/jpeg.rs`).

`fuzz/` lives outside the Cargo workspace (root `Cargo.toml` `exclude`),
so normal `cargo test` / `cargo build` never touch it.

## Run

Requires nightly Rust and cargo-fuzz:

```sh
cargo install cargo-fuzz
cargo +nightly fuzz run fuzz_compress -- -max_total_time=3600
```

Seed corpus: `fuzz/corpus/fuzz_compress/`, copied from
`crates/core/tests/fixtures/` (all formats, including `animated.*`).
Refresh it after adding a fixture:

```sh
cp crates/core/tests/fixtures/* fuzz/corpus/fuzz_compress/
```

## Replay a crasher

A crash writes `fuzz/artifacts/fuzz_compress/crash-<sha1>`. Re-run it:

```sh
cargo +nightly fuzz run fuzz_compress fuzz/artifacts/fuzz_compress/crash-<sha1>
```

Then file it as a failing unit test first, fix second — never just extend
the timeout. The minimal reproducer usually belongs in
`crates/core/tests/` next to the format's other regression tests; check the
input into the seed corpus only if it covers a new path.
