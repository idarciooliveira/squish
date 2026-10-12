# squish: product requirements and implementation plan

| | |
|---|---|
| Owner | Idarcio Oliveira (@idarciooliveira) |
| Status | Phase 0 done. Phase 1 next. |
| Last updated | 2026-10-12 |
| Decisions | [`docs/adr/`](docs/adr/README.md) (ADR-0001 to ADR-0026) |

## 1. Summary

squish is a local, fast image compressor modeled on [TinyPNG](https://tinypng.com). You give it images and get back smaller files in the same format. It ships as one Rust binary with two faces:

- a **CLI** that compresses files and folders in parallel and can write a zip
- a **web UI** (`squish serve`) where you drop images, watch each one shrink, and download them one by one or all at once as a zip

Both use the same core library, so they produce identical results.

## 2. Goals and non-goals

### Goals

1. Output sizes within about 5% of TinyPNG on our benchmark set, at no visible loss of quality.
2. Keep the input format. A PNG stays a PNG and a JPEG stays a JPEG.
3. Never return a file bigger than the input ([ADR-0011](docs/adr/0011-never-return-a-larger-file.md)).
4. Use every CPU core when compressing a batch.
5. One self-contained binary with no runtime dependencies, under about 15 MB.
6. Permissive license, MIT OR Apache-2.0 ([ADR-0004](docs/adr/0004-permissive-license-and-png-quantizer.md)).

### Non-goals for v1

- Converting between formats (PNG to WebP, for example)
- Resizing, cropping or any other editing
- Accounts, storage, history, or a public API with keys
- Animated images. APNG, animated WebP and GIF pass through untouched and get reported as unsupported.
- AVIF, unless the trigger in [ADR-0012](docs/adr/0012-avif-deferred.md) fires

## 3. Users and use cases

| Who | Wants to | Uses |
|---|---|---|
| Developer before a deploy | Shrink every image in `public/` without thinking about it | `squish ./public -r --in-place` |
| Designer exporting assets | Drop 20 exports, grab a zip | Web UI |
| Writer with phone photos | Remove GPS data and make files small enough to upload | Web UI or `squish *.jpg` |
| CI pipeline (later) | Fail if a committed image could shrink by more than X% | `squish --dry-run --json` |

## 4. Format support

| Format | Phase | Treatment |
|---|---|---|
| PNG | 1 | Lossy: reduce to at most 256 colors with dithering, then optimize losslessly. Falls back to lossless only when quality would suffer. |
| JPEG | 1 | Re-encode with mozjpeg (progressive, trellis quantization, optimized Huffman tables), then apply the metadata policy. |
| WebP | 2 | Re-encode with libwebp. Lossy input stays lossy and lossless input stays lossless. |
| AVIF | 2, conditional | Only if needed ([ADR-0012](docs/adr/0012-avif-deferred.md)). Until then it's detected and reported as "not supported yet". |

### 4.1 PNG pipeline

1. Detect the format from the magic bytes. Skip APNG.
2. Check dimensions from the header against the pixel limit.
3. Decode to RGBA8.
4. Quantize through the `Quantizer` trait (quantizr, [ADR-0022](docs/adr/0022-quantizr-as-the-png-quantizer.md)) with Floyd-Steinberg dithering. Search the palette size per file for the smallest one that scores fs2 ≥ 90 on a copy of about 1 MP ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)). If 256 colors misses the target, keep 256.
5. If quantizing fails, skip to step 7 with the original.
6. Encode an indexed PNG with the `png` crate.
7. Optimize with `oxipng::optimize_from_memory` at the preset for the chosen effort.
8. Apply the size guard.

### 4.2 JPEG pipeline

1. Detect the format (`FF D8 FF`) and read dimensions, estimated quality, ICC profile and EXIF orientation.
2. Decode once. Make the ~1 MP scoring copy and the fs2 reference from it.
3. Search mozjpeg quality per file for the lowest one that scores fs2 ≥ 88.7, in two stages ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)): cheap baseline probes from q74, then trellis probes within three points of where stage 1 landed. Encode the result progressive, with 4:2:0 chroma subsampling. Encode results bigger than the input hand the size guard the strongest fitting probe instead of the input.
4. Apply the metadata policy: keep ICC and orientation, drop the rest ([ADR-0010](docs/adr/0010-metadata-policy.md)).
5. Apply the size guard.

### 4.3 WebP pipeline (Phase 2)

1. Detect `RIFF....WEBP`. Read the VP8 or VP8L chunk to tell lossy from lossless. Skip animated files.
2. Decode with `image-webp`.
3. Re-encode with libwebp. Lossy input gets lossy quality ~75, method 4. Lossless input gets lossless.
4. Apply the size guard.

### 4.4 Default settings

`--quality auto` searches per file against a target ([ADR-0024](docs/adr/0024-per-file-quality-search-by-default.md), [ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)). The targets are the Phase 0 grid values. Phase 1 recalibrates them with the real search.

| Setting | fast | default | max |
|---|---|---|---|
| PNG | up to 256 colors, no search | smallest palette with fs2 ≥ 90 | same target, more probes allowed |
| oxipng preset | 1 | 2 | 5 |
| JPEG | q66, no search | lowest quality with fs2 ≥ 89.3 | same target, more probes allowed |
| mozjpeg trellis | off | on | on, with scan optimization |
| WebP method | 2 | 4 | 6 |

## 5. Functional requirements

### 5.1 Core (`squish-core`)

- **FR-1** `detect(&[u8]) -> Option<Format>` identifies PNG, JPEG, WebP and AVIF from magic bytes, plus whether the file is animated.
- **FR-2** `compress(&[u8], &Options) -> Result<Output, Error>` runs the pipeline for the detected format.
- **FR-3** `Output` holds the bytes, the format, the original size and `kept_original: bool`.
- **FR-4** Rejects images above `max_pixels` before decoding (default 50 megapixels).
- **FR-5** Does no file I/O, no network calls and no printing.
- **FR-6** The quantizer is pluggable through a trait. Cargo features choose the implementation.
- **FR-6a** Option types implement `FromStr`, and `Error` has stable codes. The CLI and the API share both ([ADR-0017](docs/adr/0017-shared-options-and-errors.md)).

```rust
pub enum Format { Png, Jpeg, WebP, Avif }

pub struct Options {
    pub quality: Quality,       // Auto | Fixed(u8) | Lossless
    pub effort: Effort,         // Fast | Default | Max
    pub metadata: MetaPolicy,   // StripKeepIcc | StripAll | KeepAll
    pub max_pixels: u64,
}

pub struct Output {
    pub bytes: Vec<u8>,
    pub format: Format,
    pub original_size: usize,
    pub kept_original: bool,
}

pub trait Quantizer {
    /// At most `max_colors` colors. The quality search lives outside the trait (ADR-0024).
    fn quantize(&self, rgba: &[u8], width: u32, height: u32, max_colors: u16, dithering: f32)
        -> Option<(Vec<[u8; 4]>, Vec<u8>)>; // palette, indices
}
```

### 5.2 CLI (`squish`)

Full design, including file selection, output paths, the JSON report and Ctrl-C handling: [ADR-0019](docs/adr/0019-cli-design.md).

- **FR-7** Accepts files, folders, globs and `-` for stdin. `-r` recurses into folders. Folder walks only pick image extensions and skip `*.min.*`, hidden files and symlinks.
- **FR-8** Output modes:
  - default: `name.min.ext` next to the input
  - `-o, --out-dir DIR`: mirror the input tree into DIR with the original names
  - `--in-place`: overwrite, after a confirmation prompt unless `--yes` is passed
  - `--zip FILE`: write everything into one STORE zip ([ADR-0008](docs/adr/0008-download-all-zip-in-browser.md))
- **FR-9** Flags `--quality auto|0-100|lossless`, `--lossless`, `-e, --effort fast|default|max`, `--metadata strip-keep-icc|strip-all|keep-all`, `--max-pixels MP`, `-j, --jobs N` (defaults to the number of cores), `-n, --dry-run`, `--json`, `-y, --yes`, `-q, --quiet`.
- **FR-10** Processes files in parallel with rayon and shows a progress bar on a TTY.
- **FR-11** Ends with a summary: file count, bytes before and after, percent saved, elapsed time, and the lists of files that were already optimized, unsupported, or failed.
- **FR-12** Exit codes: `0` when everything succeeded or was skipped as unsupported, `1` when at least one file failed, `2` for usage errors, `130` when interrupted.

```text
$ squish ./images -r -o ./out
  48 files   182.4 MB -> 51.9 MB   (-71.5%)   3.2 s on 12 threads
  3 already optimized, kept as is
  1 unsupported (AVIF): images/hero.avif
```

### 5.3 HTTP server (`squish serve`)

Full design, including headers, error bodies and timeout behavior: [ADR-0018](docs/adr/0018-http-api.md).

- **FR-13** `squish serve [--host 127.0.0.1] [--port 8080] [--open] [--jobs N] [--max-body MB] [--max-pixels MP] [--timeout SECS]`, with `SQUISH_HOST` and `SQUISH_PORT` env vars. Binds to localhost by default ([ADR-0014](docs/adr/0014-local-first-deployment.md)).
- **FR-14** Routes:

| Route | Behavior |
|---|---|
| `GET /` and static assets | The embedded web UI |
| `POST /api/compress?quality=&effort=&metadata=` | The body is the raw image bytes. The response is the compressed image, with `Content-Type`, `X-Original-Size`, `X-Compressed-Size`, `X-Kept-Original` and `Server-Timing`. |
| `GET /api/info` | Version, supported formats, limits and defaults, as JSON |
| `GET /api/health` | `200 ok` |

- **FR-15** Error responses carry a JSON body `{"error": code, "message": text}`: `400` for an empty body or a bad option, `413` above the body limit (default 25 MB) or the pixel limit, `415` for unsupported or animated formats, `422` when decoding fails, `429` with `Retry-After` when every CPU slot is busy, `500` when encoding fails, `504` when the per-request timeout passes (default 30 s).
- **FR-16** Stateless. Nothing is written to disk ([ADR-0007](docs/adr/0007-stateless-server-side-compression.md)).
- **FR-17** CPU work runs in `spawn_blocking`, gated by a semaphore sized to the core count.

### 5.4 Web UI

- **FR-18** A drop zone and a file picker. Accepts multiple files and folders.
- **FR-19** A client-side queue with up to 4 uploads in flight and a backoff retry on `429`.
- **FR-20** One row per file: thumbnail, name, size before and after, percent saved, status, and a Download button.
- **FR-21** A running total at the top ("Saved 131 MB, 72%").
- **FR-22** "Download all" builds a STORE zip in the browser with client-zip or fflate.
- **FR-23** A settings drawer, closed by default, for quality and effort.
- **FR-24** Plain HTML, CSS and JS, embedded with rust-embed, with no build step ([ADR-0013](docs/adr/0013-vanilla-frontend-embedded.md)).

## 6. Non-functional requirements

| ID | Requirement | Target |
|---|---|---|
| NFR-1 | Compression | Per format on `bench/`, with default settings: total bytes at most 5% above TinyPNG's, median per-file dssim ratio (ours / TinyPNG's, floored at 0.0002) at most 1.0, and no file above 2.0. For PNG, files at 256 colors don't count toward the 2.0 limit while ADR-0022 stands ([ADR-0025](docs/adr/0025-nfr-1-against-a-fixed-target.md)). |
| NFR-2 | Batch throughput | Near-linear scaling up to the core count. The quality search costs at most 4x one plain encode (median CPU on `bench/`), and no file takes more than 10 s of CPU ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)). |
| NFR-3 | Binary size | Under 15 MB, stripped release build |
| NFR-4 | Server idle memory | Under 20 MB RSS |
| NFR-5 | Peak memory per job | About 4 bytes per pixel times 3, bounded by `max_pixels` |
| NFR-6 | Safety | No panics on malformed input. A fuzz test on `detect` and `compress` runs for an hour with no crash. |
| NFR-7 | Portability | Release builds for Linux (static musl), macOS and Windows on x86_64 and arm64 |
| NFR-8 | Privacy | No telemetry. The server stores nothing. GPS data is removed by default. |

## 7. Architecture

```text
.
├── Cargo.toml              # workspace, shared deps and lints, release profile
├── crates/
│   ├── core/               # squish-core: detect, options, errors, size guard, metric, search, codec/, quant/
│   ├── server/             # squish-server: axum routes, limits, embedded web/
│   └── cli/                # squish, the binary: clap, rayon, indicatif, zip, `serve`
├── web/                    # index.html, app.js, style.css, vendor/ zip lib
├── bench/                  # outside the workspace: corpus (gitignored), spike/, scripts, results
├── fuzz/                   # Phase 4, outside the workspace
└── docs/adr/
```

The file-level layout and the rules around it are in [ADR-0020](docs/adr/0020-repository-layout.md).

One binary, three crates ([ADR-0006](docs/adr/0006-workspace-and-single-binary.md)).

```text
 CLI ─────────────┐
                  ├──> squish-core ──> codec adapters ──> oxipng, quantizr, mozjpeg, libwebp
 Browser ─> HTTP ─┘    detect -> decode -> search (encode, score with fast-ssim2) -> encode -> size guard
```

### Release profile

```toml
[profile.release]
lto = "fat"
codegen-units = 1
panic = "unwind"  # mozjpeg panics on bad input, see ADR-0017
strip = true
```

## 8. Dependencies

Versions and licenses checked against crates.io and npm on 2026-10-11.

| Purpose | Crate | Version | License | Notes |
|---|---|---|---|---|
| Decoding | `image` | 0.25 | MIT OR Apache-2.0 | |
| Faster JPEG decoding (optional) | `zune-jpeg` | 0.5 | MIT OR Apache-2.0 OR Zlib | |
| Indexed PNG writing | `png` | 0.18 | MIT OR Apache-2.0 | |
| Lossless PNG optimization | `oxipng` | 10.2 | MIT | `optimize_from_memory` confirmed in docs.rs |
| PNG quantization | `quantizr` | 1.4 | MIT | RGBA and dithering confirmed in Phase 0 ([ADR-0022](docs/adr/0022-quantizr-as-the-png-quantizer.md)) |
| Quality metric | `fast-ssim2` | 0.9 | BSD-2-Clause | ssimulacra2 with SIMD, scored at ~1 MP. Needs Rust 1.89+ ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)) |
| JPEG encoding | `mozjpeg` / `mozjpeg-sys` | 0.10 / 2.2 | IJG (+ Zlib, BSD-3-Clause in -sys) | Compiles vendored C. Without `nasm` it builds without SIMD, which is slower. Last release Feb 2025. |
| WebP encoding (Phase 2) | `webp` / `libwebp-sys` | 0.3 / 0.14 | MIT OR Apache-2.0 / MIT | |
| WebP decoding (Phase 2) | `image-webp` | 0.2 | MIT OR Apache-2.0 | |
| Metadata chunks | `img-parts` | 0.4 | MIT OR Apache-2.0 | Moves ICC and EXIF segments without re-encoding |
| EXIF orientation | `kamadak-exif` | 0.6 | BSD-2-Clause | Last release Nov 2024. We could replace it with ~50 lines of our own. |
| HTTP | `axum`, `tokio`, `tower-http` | 0.8, 1.53, 0.7 | MIT | |
| Embedding the UI | `rust-embed` | 8.13 | MIT | |
| CLI | `clap`, `indicatif` | 4.6, 0.18 | MIT OR Apache-2.0, MIT | |
| Parallelism | `rayon` | 1.12 | MIT OR Apache-2.0 | |
| Zip (CLI) | `zip` | 9.0 | MIT | STORE mode |
| Zip (browser) | `client-zip` or `fflate` | 2.5 / 0.8 | MIT | Vendored into `web/` |

**Benchmark only, never linked into squish:** `imagequant` 4.4 (GPL-3.0-or-later, with a commercial license available) and `dssim` 3.5 (AGPL-3.0, with a commercial license available).

**AVIF, only if ADR-0012 triggers:** `ravif` 0.13 (BSD-3-Clause) for encoding, and `dav1d` 0.11 (MIT) for decoding. The `libavif` crate is stale (last release July 2024).

**Not used, kept as references:** `libcaesium` (Apache-2.0, but its default features pull in GPL imagequant) and `rimage` (MIT OR Apache-2.0).

## 9. Roadmap

### Phase 0: spike and benchmark (2 to 3 days), [ADR-0016](docs/adr/0016-benchmark-before-building.md). Done 2026-10-12

- [x] Install `rustup` (stable 1.99.0) and `build-essential` on WSL
- [x] Install `nasm` (3.01) and `cmake` (4.2.3) on WSL
- [x] Collect ~50 images in `bench/`: photos, screenshots, logos with alpha, illustrations, already-optimized files
- [x] Run them through TinyPNG once and save the output as the baseline
- [x] Throwaway program: PNG via quantette + oxipng, and via imagequant + oxipng
- [x] Confirm quantette handles RGBA with transparency (it doesn't, see ADR-0022)
- [x] Throwaway program: JPEG via mozjpeg at quality 75, 80 and 85, plus jpegli if its bindings build
- [x] Record bytes, time and SSIM per file

**Exit criteria:** an ADR for each open question (quantizer, JPEG encoder, default values), and a results table committed to `bench/`. Met by [ADR-0022](docs/adr/0022-quantizr-as-the-png-quantizer.md), [ADR-0023](docs/adr/0023-mozjpeg-confirmed-over-jpegli.md), [ADR-0024](docs/adr/0024-per-file-quality-search-by-default.md) and `bench/results/`.

### Phase 1: core and CLI for PNG and JPEG (about 2 weeks)

The metric work is done: fast-ssim2 at about 1 MP, with starting targets from the Phase 0 grid ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)). The estimate grew from one week because the search, its calibration and the benchmark harness are now part of this phase.

- [x] Pick a permissive quality metric and starting targets ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md))
- [x] Workspace with `crates/core` and `crates/cli` (ADR-0020)
- [x] `format.rs` with tests for each magic number and for animation
- [x] `metric.rs`: fast-ssim2 on a ~1 MP box-downscaled copy, alpha composited over white and black
- [x] `search.rs`: the shared per-file search, JPEG from q66 with interpolation, PNG over the palette size, probe limits per effort
- [x] `png.rs`: quantize, indexed encoding, oxipng on the final palette only, lossless fallback
- [x] `jpeg.rs`: two-stage search: fast basline probes, then trellis probes to land, metadata policy, orientation
- [x] Size guard, plus a test that output is never bigger than input
- [x] Benchmark harness in `bench/` that runs `squish-core` over the corpus and checks NFR-1 (ADR-0025) and the search budget in NFR-2, writing the CSV from §10
- [x] Recalibrate the targets with the real search: JPEG 88.7, PNG 90. The 2 to 4 MP scoring cap above 16 MP does not pay: the numbers are in ADR-0026.
- [ ] CLI: files, folders, `-r`, `-o`, `--in-place`, default `.min` suffix
- [ ] rayon, progress bar, summary, exit codes
- [ ] Fixture tests in `crates/core/tests/fixtures/` (committed, under 1 MB) that run in CI, plus golden tests over `bench/images/` that run locally and skip when the corpus is absent (ADR-0020)

**Exit criteria:** the benchmark harness shows NFR-1 met for PNG and JPEG as defined in ADR-0025, the search within the NFR-2 budget, and `squish ./bench -r` runs clean.

### Phase 2: WebP, zip, polish, and AVIF only if needed (about 1 week)

- [ ] `webp.rs`: detect lossy or lossless, encode with libwebp
- [ ] Skip animated files cleanly
- [ ] `--zip`, `--json`, `--dry-run`
- [ ] Effort presets
- [ ] Release profile, musl static build, binary size check
- [ ] Check the AVIF trigger in ADR-0012. If it fires, add `avif.rs` behind the `avif` feature, with a speed preset and a timeout.

**Exit criteria:** NFR-1 met for WebP, and the binary is under 15 MB.

### Phase 3: web version (about 1 week)

- [ ] `server` crate: axum routes, limits, semaphore, timeout, error codes
- [ ] `web/`: drop zone, queue, rows, per-file download, running total
- [ ] "Download all" with a vendored zip library
- [ ] Embed with rust-embed, add the `squish serve` subcommand
- [ ] Manual test with 50 mixed files, plus an integration test for each route

**Exit criteria:** the full flow works in Chrome and Firefox. The server stays under the NFR-4 memory target at idle.

### Phase 4: ship and extras (open-ended)

- [ ] GitHub Actions: tests and release binaries for Linux, macOS and Windows
- [ ] `Dockerfile` `FROM scratch` containing only the static binary
- [ ] A fuzz target for `detect` and `compress` (NFR-6)
- [ ] Optional in-browser WASM mode
- [ ] Optional resizing and format conversion
- [ ] Tinify-compatible routes, only if a trigger in [ADR-0021](docs/adr/0021-tinify-compatible-api.md) fires

## 10. Testing and measurement

- **Unit tests** in `core` for detection, options and the size guard.
- **Golden tests** run every file in `bench/images/` and assert the output decodes, keeps its dimensions, and is never bigger than the input. The corpus is gitignored, so `golden.rs` skips when it's empty and CI relies on the committed fixtures in `crates/core/tests/fixtures/` (ADR-0020).
- **Benchmark script** writes a CSV per run: file, format, original bytes, squish bytes, TinyPNG bytes, time, the shipped metric's score, and dssim against both squish's and TinyPNG's output. dssim is the NFR-1 reference and never ships. Phase exit criteria use it.
- **Server integration tests** cover each route and each error code.
- **Fuzzing** (Phase 4) runs `cargo fuzz` on `detect` and `compress`.

## 11. Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| quantizr's PNG quality stays below TinyPNG's on about half of `bench/` | High | Accepted for now. NFR-1 doesn't count files already at 256 colors toward its 2.0 limit ([ADR-0025](docs/adr/0025-nfr-1-against-a-fixed-target.md)). Revisit triggers and fallbacks (commercial imagequant, our own quantizer) in [ADR-0022](docs/adr/0022-quantizr-as-the-png-quantizer.md). |
| The ~1 MP scoring copy hides artifacts on very large photos (the 40 MP file sets the JPEG target) | Medium | Try a 2 to 4 MP cap above ~16 MP in Phase 1 ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)) |
| The JPEG search misses the speed budget | Medium | Baseline probes, a q66 start with interpolation, and DCT-scaled decodes. `--effort fast` skips the search. |
| Re-encoding grows already-optimized files | High | Size guard and early skips ([ADR-0011](docs/adr/0011-never-return-a-larger-file.md)) |
| Colors shift or photos come out rotated after stripping metadata | Medium | Keep ICC and orientation ([ADR-0010](docs/adr/0010-metadata-policy.md)) |
| C toolchain issues on macOS or Windows | Medium | CI builds all three platforms from Phase 4. mozjpeg builds without `nasm`, just slower. |
| mozjpeg crate goes stale (last release Feb 2025) | Low | jpegli builds and is the measured fallback, about 6% behind on bytes ([ADR-0023](docs/adr/0023-mozjpeg-confirmed-over-jpegli.md)) |
| Decompression bombs | Medium | Pixel limit checked from the header before decoding |
| Hooks bypassed with `--no-verify` | Low | `pre-push` re-checks. A CI check can follow ([ADR-0015](docs/adr/0015-commit-policy.md)). |

## 12. Open questions

These get answered by an ADR when we reach them.

1. ~~Does quantette handle transparency and match imagequant closely enough?~~ No. quantizr ships instead ([ADR-0022](docs/adr/0022-quantizr-as-the-png-quantizer.md)).
2. ~~Does jpegli beat mozjpeg enough to be worth its build cost?~~ No ([ADR-0023](docs/adr/0023-mozjpeg-confirmed-over-jpegli.md)).
3. ~~What are the measured default quality values?~~ A per-file search ([ADR-0024](docs/adr/0024-per-file-quality-search-by-default.md)) with fast-ssim2 targets of 88.7 for JPEG and 90 for PNG as recalibrated in Phase 1 ([ADR-0026](docs/adr/0026-fast-ssim2-as-the-quality-metric.md)).
4. Does the AVIF trigger fire? (Phase 2)
5. Does the web version go public? (After Phase 3, [ADR-0014](docs/adr/0014-local-first-deployment.md))

## 13. Decision log

Every decision behind this plan is recorded in [`docs/adr/`](docs/adr/README.md). When a decision changes, a new ADR supersedes the old one, and this PRD gets updated to match.
