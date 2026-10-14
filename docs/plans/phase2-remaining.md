# Phase 2 remaining work: a task plan for small models

This plan finishes Phase 2 of `prd.md`. It is written for a small coding model that works one task per session and has not seen the earlier conversations. Each task lists what to read, what to change, how to check it and the commit message to use.

Do the tasks in order. Each one builds on the one before it, except T8, T9 and T10, which only need T7 done.

## Rules for every task

1. **Read before writing.** Open every file in the task's "Read first" list. Copy the style of the code around you: comment density, naming, error handling, test layout.
2. **No new design.** ADR-0028 already decides every WebP behavior. If a step here seems to contradict an ADR or the PRD, stop and ask the owner. Don't pick an answer yourself.
3. **Stay inside the task.** Touch only the files the task names. Don't reformat, rename or "clean up" other code.
4. **Branch.** All tasks T1 to T7 go on one branch, `phase2-webp-codec`, cut from `main`. T8, T9 and T10 each get their own branch from `main` after T7 merges.
5. **Checks before every commit.** All four must pass:
   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo check -p squish-core --no-default-features
   cargo test --workspace
   ```
   The tests run real encodes. The first build takes several minutes because mozjpeg and libwebp compile from C.
6. **Commit policy (ADR-0015).** Turn the hooks on once with `git config core.hooksPath .githooks`. Every commit message is exactly one line. No `Co-authored-by`, no "Generated with", no mention of Claude or Anthropic. Author and committer are `idarciooliveira <idarciooliveira@gmail.com>`. Use the message given in the task.
7. **`unsafe` is forbidden** in the workspace (`unsafe_code = "forbid"`). The `webp` and `image-webp` crates have safe APIs, so you never need it.
8. **Never commit anything from `bench/images/` or `bench/tinypng/`.** They are gitignored on purpose.

## Background you need

- `squish-core` takes bytes and returns bytes. `crates/core/src/lib.rs` calls `preflight` (format, animation and pixel-limit checks), then `codec::compress`, then the size guard `guard::apply`. The guard already returns the input when the output isn't smaller, so codecs don't need to check size.
- `crates/core/src/format.rs` already detects WebP, reads its dimensions and spots animated files. `preflight` already returns `Error::Animated(Format::WebP)` for animated files.
- `crates/core/src/codec/mod.rs` still returns `Error::Unsupported` for WebP. Your job is to replace that with `codec/webp.rs`.
- `crates/core/src/metadata.rs` holds the metadata policy for JPEG segments and PNG chunks. WebP chunk handling goes there too.
- The decisions, from ADR-0028 and PRD sections 4.3 and 4.4:

  | Input | `Quality::Auto` | `Quality::Fixed(n)` | `Quality::Lossless` |
  |---|---|---|---|
  | lossy (`VP8 `) | libwebp lossy q71 | libwebp lossy qn | no re-encode, metadata policy only |
  | lossless (`VP8L`) | libwebp lossless | libwebp lossless (n is ignored) | libwebp lossless |

  | Effort | libwebp `method` |
  |---|---|
  | `Fast` | 2 |
  | `Default` | 4 |
  | `Max` | 6 |

  There is no quality search for WebP at any effort.

---

## T1. Add the WebP crates

**Goal.** The workspace depends on `webp` (encoder) and `image-webp` (decoder) with no extra features.

**Read first.** `Cargo.toml` (root), `crates/core/Cargo.toml`, PRD section 8 (the dependency table).

**Change.**

1. In the root `Cargo.toml`, under `# codecs and metric`, add:
   ```toml
   webp = { version = "0.3", default-features = false }
   image-webp = "0.2"
   ```
   `default-features = false` matters. The default `img` feature pulls in the whole `image` crate, which we don't use and which grows the binary.
2. In `crates/core/Cargo.toml`, under `[dependencies]`, add `webp.workspace = true` and `image-webp.workspace = true`.

**Done when.** `cargo build -p squish-core` succeeds and `cargo tree -p squish-core -e normal | grep -c "image v"` prints `0`.

**Commit.** `Add the webp and image-webp crates to squish-core`

---

## T2. RIFF chunk helpers in `metadata.rs`

**Goal.** Functions that read the chunks of a WebP file, decide which ones the metadata policy keeps, and write a new WebP container. No codec yet.

**Read first.** `crates/core/src/metadata.rs` (all of it, especially the `// PNG` section and `minimal_exif`, `tiff_orientation`), ADR-0028 Decision point 5, ADR-0010, and the WebP container spec: <https://developers.google.com/speed/webp/docs/riff_container>.

**Facts about the format.**

- File = `RIFF` + u32 LE size (file length minus 8) + `WEBP` + chunks.
- Chunk = 4-byte FourCC + u32 LE payload size + payload + one zero pad byte when the size is odd. The pad byte isn't counted in the size.
- `VP8X` payload is 10 bytes: a flags byte, 3 reserved bytes, canvas width minus 1 (u24 LE), canvas height minus 1 (u24 LE).
- `VP8X` flag bits: ICC `0x20`, alpha `0x10`, EXIF `0x08`, XMP `0x04`, animation `0x02`.
- The `EXIF` payload is a TIFF header (`II*\0` or `MM\0*`). Some writers put `Exif\0\0` in front of it. Accept both.

**Change.** Add a `// WebP` section after the PNG section in `metadata.rs`:

```rust
pub(crate) struct WebpChunk<'a> {
    pub kind: [u8; 4],
    /// The payload, without the 8-byte header or the pad byte.
    pub payload: &'a [u8],
}

/// Walks the top-level chunks after `WEBP`, stopping at the first damaged one.
pub(crate) fn webp_chunks(bytes: &[u8]) -> Vec<WebpChunk<'_>>;

/// True when the image data is a `VP8L` bitstream. A lossy file with alpha has `ALPH` + `VP8 `.
pub(crate) fn webp_is_lossless(bytes: &[u8]) -> bool;

/// Orientation 1 to 8 from a WebP `EXIF` payload, with or without the `Exif\0\0` prefix.
pub(crate) fn webp_orientation(exif: &[u8]) -> Option<u8>;

/// The chunks around the image data that go into the output.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct WebpMeta {
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    /// Chunks squish doesn't know, in their original order. Only `keep-all` fills this.
    pub unknown: Vec<([u8; 4], Vec<u8>)>,
}

pub(crate) fn webp_meta(bytes: &[u8], policy: MetaPolicy) -> WebpMeta;

/// Builds a WebP file from the image chunks of `encoded` (a complete file from libwebp,
/// or the original) and `meta`. `width` and `height` are the canvas size.
pub(crate) fn webp_assemble(encoded: &[u8], meta: &WebpMeta, width: u32, height: u32) -> Vec<u8>;
```

Rules for `webp_meta`:

- `StripAll`: everything empty.
- `StripKeepIcc`: `icc` from `ICCP`. If the `EXIF` chunk has an orientation other than 1, `exif = Some(minimal_exif(o)[EXIF_SIGNATURE.len()..].to_vec())`. `minimal_exif` returns a JPEG APP1 payload that starts with `Exif\0\0` (`EXIF_SIGNATURE`), and the WebP chunk holds the bare TIFF data. Read the orientation with the existing private `tiff_orientation`. Nothing else.
- `KeepAll`: `ICCP`, `EXIF` and `XMP ` unchanged, and every chunk that is not one of `VP8X`, `VP8 `, `VP8L`, `ALPH`, `ICCP`, `EXIF`, `XMP `, `ANIM`, `ANMF` goes into `unknown`.

Rules for `webp_assemble`:

- Take the `ALPH`, `VP8 ` and `VP8L` chunks from `encoded`, in the order they appear.
- If `meta` is empty and there is no `ALPH` chunk, write a simple file: `RIFF`, size, `WEBP`, the one bitstream chunk.
- Otherwise write `VP8X` first, then `ICCP`, then the image chunks (`ALPH` before `VP8 `), then the `unknown` chunks, then `EXIF`, then `XMP `.
- `VP8X` flags: ICC if `icc` is set, EXIF if `exif` is set, XMP if `xmp` is set, alpha if `encoded` has an `ALPH` chunk or its `VP8X` had the alpha flag, or its `VP8L` header has the alpha bit (bit 28 of the u32 LE that follows the `0x2F` signature byte).
- Pad odd chunks. Set the RIFF size last, from the finished buffer.

**Tests.** Add them to the existing `mod tests` in `metadata.rs`, with a small helper that builds a WebP file from `(kind, payload)` pairs, like the existing `png(...)` helper:

- `webp_chunks` reads odd-sized chunks and stops on a truncated one without panicking.
- `webp_is_lossless` is true for `VP8L`, false for `VP8 ` and for `ALPH` + `VP8 `.
- `webp_orientation` reads both byte orders and the `Exif\0\0` prefix, and returns `None` for garbage.
- `webp_meta` follows each policy. `keep-all` keeps two unknown chunks in their order.
- `webp_assemble` with empty meta gives a simple file. With ICC and EXIF it writes `VP8X` with the right flags and canvas size, the right chunk order, and a RIFF size equal to `len - 8`.
- Fed random and truncated bytes, no function panics (copy the shape of `damaged_jpeg_never_panics`).

**Done when.** The rule-5 checks pass. Clippy may flag the new functions as unused until T4. If so, add `#[allow(dead_code)]` on them with a `// Used by codec/webp.rs from the next commit.` comment, and remove it in T4.

**Commit.** `Add RIFF chunk helpers for WebP metadata: read chunks, apply the policy, assemble the container`

---

## T3. Decode WebP

**Goal.** `codec/webp.rs` exists and can decode to RGBA, so `squish_core::score` works on WebP.

**Read first.** `crates/core/src/codec/mod.rs`, `crates/core/src/codec/png.rs` (its `decode_rgba` and how it respects `max_pixels`), `crates/core/src/lib.rs` (`score`).

**Change.**

1. Create `crates/core/src/codec/webp.rs` with a module doc comment in the style of `jpeg.rs`, for example `//! WebP pipeline (PRD 4.3, ADR-0028): decode with image-webp, re-encode with libwebp at a fixed quality.`
2. Add a private `decode` that returns RGBA8 or RGB8 plus width, height and an alpha flag:
   ```rust
   let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes))
       .map_err(|e| Error::Decode(e.to_string()))?;
   let (width, height) = decoder.dimensions();
   let alpha = decoder.has_alpha();
   let size = decoder.output_buffer_size().ok_or_else(|| Error::Decode("image too large".into()))?;
   let mut pixels = vec![0; size];
   decoder.read_image(&mut pixels).map_err(|e| Error::Decode(e.to_string()))?;
   ```
   `read_image` writes RGBA when `has_alpha()` is true and RGB otherwise. Check `width * height` against `max_pixels` before allocating, as `png::decode_rgba` does.
3. Add `pub(crate) fn decode_rgba(bytes: &[u8], max_pixels: u64) -> Result<(Vec<u8>, u32, u32), Error>` that expands RGB to RGBA with alpha 255, like `jpeg::decode_rgba`.
4. In `codec/mod.rs`, add `pub(crate) mod webp;` and route `Format::WebP` in `decode_rgba` to it. Leave `compress` alone for now.
5. In `lib.rs`, change the `score` doc comment from "PNG or JPEG" to "PNG, JPEG or WebP".

**Tests.** In `webp.rs`, a `#[cfg(test)] mod tests` that encodes a small gradient with `webp::Encoder::from_rgb(...).encode_lossless()`, decodes it with `decode_rgba` and checks the pixels match exactly. A second test checks that random bytes after a valid `RIFF....WEBP` header give `Err(Error::Decode(_))`, not a panic.

**Commit.** `Decode WebP with image-webp so score works on WebP files`

---

## T4. Compress WebP

**Goal.** `compress` handles still WebP files as the tables in "Background" say.

**Read first.** `codec/jpeg.rs` (`compress`, for structure and the `Report`), `lib.rs` (`SUPPORTED_FORMATS`, `Report`), `options.rs`, the metadata functions from T2.

**Change.** In `webp.rs`:

```rust
/// libwebp quality for lossy files at every effort (ADR-0028).
pub(crate) const QUALITY: u8 = 71;

fn method(effort: Effort) -> i32 {
    match effort {
        Effort::Fast => 2,
        Effort::Default => 4,
        Effort::Max => 6,
    }
}
```

An `encode(pixels, width, height, alpha, quality: Option<u8>, effort) -> Result<Vec<u8>, Error>` helper. `None` means lossless:

```rust
let encoder = if alpha {
    webp::Encoder::from_rgba(pixels, width, height)
} else {
    webp::Encoder::from_rgb(pixels, width, height)
};
let mut config = webp::WebPConfig::new().map_err(|()| Error::Encode("libwebp config".into()))?;
config.method = method(effort);
match quality {
    Some(q) => {
        config.quality = f32::from(q);
        config.alpha_compression = 1;
    }
    None => {
        config.lossless = 1;
        config.exact = 1; // keep RGB under transparent pixels: lossless means no pixel changes
    }
}
let memory = encoder.encode_advanced(&config).map_err(|e| Error::Encode(format!("{e:?}")))?;
Ok(memory.to_vec())
```

Check the exact error types in `~/.cargo/registry/src/*/webp-0.3.1/src/` and adjust the `map_err` calls so they compile. Don't change the logic. Lossy files with alpha keep libwebp's defaults for `alpha_quality` and `exact` (ADR-0028 point 4).

Then `pub(crate) fn compress(bytes: &[u8], options: &Options) -> Result<Encoded, Error>`:

1. `let lossless = metadata::webp_is_lossless(bytes);` and `let meta = metadata::webp_meta(bytes, options.metadata);`
2. If the input is lossy and `options.quality == Quality::Lossless`, don't decode. Return `webp_assemble(bytes, &meta, w, h)` with `Report::default()`. This reuses the original bitstream and only applies the metadata policy. `w` and `h` come from `crate::format::dimensions`.
3. Otherwise decode, then pick the quality: lossless input gives `None`. Lossy input gives `Some(n)` for `Quality::Fixed(n)` and `Some(QUALITY)` for `Auto`.
4. Encode, then `webp_assemble(&encoded, &meta, width, height)`.
5. Report: `setting: Some(q as u32)` for lossy and `None` for lossless. `score`, `target` and `probes` stay at their defaults, because nothing is scored.

In `codec/mod.rs`, route `Format::WebP` in `compress` to `webp::compress`. In `lib.rs`, change `SUPPORTED_FORMATS` to `&[Format::Png, Format::Jpeg, Format::WebP]`. Update the `Report::setting` doc comment to mention WebP quality. Remove any `#[allow(dead_code)]` added in T2.

**Expected test fallout.** `crates/server/tests/routes.rs` line 63 asserts the format list is `["png", "jpeg"]`. Change it to `["png", "jpeg", "webp"]`. That is the only existing assertion that should break. If any other test breaks, stop and report it rather than editing the test.

**Unit tests in `webp.rs`.** Build inputs in the test with `webp::Encoder`:

- A lossy q95 input of a noisy 128x96 image comes out smaller, still lossy, same dimensions, `report.setting == Some(71)`.
- A lossless input comes out lossless (`webp_is_lossless`) and decodes to exactly the input pixels.
- `Quality::Fixed(50)` on a lossless input still gives lossless output.
- `Quality::Lossless` on a lossy input keeps the original `VP8 ` payload byte for byte.
- An RGBA lossy input keeps its alpha: the output has an `ALPH` chunk, and the `VP8X` alpha flag is set.

**Commit.** `Add the WebP codec: lossy at fixed q71, lossless stays lossless, method by effort, RIFF metadata policy`

---

## T5. WebP fixtures

**Goal.** Small committed WebP files so CI tests WebP without the gitignored corpus.

**Read first.** `bench/fixture-gen/src/main.rs`, `bench/fixture-gen/Cargo.toml`, `crates/core/tests/fixtures/README.md`.

**Change.**

1. Add `webp = { version = "0.3", default-features = false }` to `bench/fixture-gen/Cargo.toml`.
2. In `main`, reuse the existing `rgb` photo (192x128) and `rgba` logo (96x96) and write:

   | File | How |
   |---|---|
   | `photo.webp` | lossy q95, method 4, from `rgb` |
   | `photo-lossless.webp` | lossless, method 0 (so there is room to shrink), from `rgb` |
   | `logo-alpha.webp` | lossy q95 with alpha, from `rgba` |
   | `photo-meta.webp` | `photo.webp` rewrapped by hand in a `VP8X` container with the `fake_icc` profile in `ICCP`, an `EXIF` chunk with orientation 6 (reuse `exif_orientation`, minus any `Exif\0\0` prefix), an `XMP ` chunk and one unknown chunk `ZZZZ` |
   | `animated.webp` | `photo.webp` rewrapped with a `VP8X` header that has the animation flag `0x02` set |

   Write a small `riff(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8>` helper in `fixture-gen` for the two rewrapped files. Don't import it from `squish-core`, because the fixtures must not depend on the code they test.
3. Run `cargo run --release --manifest-path bench/fixture-gen/Cargo.toml -- crates/core/tests/fixtures`. Check with `git status` that only the five new `.webp` files appear. If existing fixtures changed, the generator is not deterministic. Restore them with `git checkout crates/core/tests/fixtures` and stop.
4. Add five rows to the README table and update the total size line. All fixtures together must stay under 1 MB (ADR-0020).

**Commit.** `Add WebP fixtures: lossy, lossless, alpha, metadata and animated`

---

## T6. End-to-end tests for WebP

**Goal.** The core, CLI and server tests cover WebP.

**Read first.** `crates/core/tests/compress.rs`, `crates/core/tests/golden.rs`, `crates/cli/tests/cli.rs`, `crates/server/tests/routes.rs`.

**Change.**

1. `compress.rs`:
   - Add the three still `.webp` fixtures plus `photo-meta.webp` to `FIXTURES`. Teach `fixtures_are_detected_by_content` and the `decode` helper about WebP (decode with `image_webp`, which you need as a dev-dependency: `image-webp.workspace = true` under `[dev-dependencies]` in `crates/core/Cargo.toml`).
   - The existing loop tests, such as `every_setting_keeps_dimensions_and_never_grows_a_file`, should now cover WebP for free. Run them and confirm.
   - New tests: `photo-meta.webp` under each `MetaPolicy` (default keeps `ICCP` and an orientation-6 `EXIF`, drops `XMP ` and `ZZZZ`. `strip-all` keeps neither. `keep-all` keeps all four). `animated.webp` gives `Error::Animated(Format::WebP)`. `photo-lossless.webp` stays lossless at every effort and quality.
2. `golden.rs`: update the doc comment and the comment on the `Err` arm, which still say PNG and JPEG only.
3. `cli.rs`: one test that runs `squish` on `photo.webp` and checks that `photo.min.webp` exists and is smaller. Copy the shape of an existing PNG or JPEG test.
4. `routes.rs`: one test that posts `photo.webp` to the compress route and gets `200` with `content-type: image/webp`, and one that posts `animated.webp` and gets `415` with `animated_image`. Copy `animated_png_is_415`.

**Commit.** `Test WebP end to end in core, CLI and server`

---

## T7. Benchmark, docs and the PR

**Goal.** Confirm NFR-1 for WebP on the real corpus, update the docs, and open the PR.

**Read first.** `bench/README.md`, `bench/harness/src/main.rs` (top doc comment), ADR-0025, ADR-0028 "Consequences", `prd.md` sections 4.3, 10 and the Phase 2 checklist, `README.md` status line.

**Change.**

1. Run the golden test with the corpus present: `cargo test -p squish-core --release --test golden`. It must pass. Large WebP files are now compressed instead of skipped.
2. Run the harness on WebP: `cargo run --release --manifest-path bench/harness/Cargo.toml -- --only webp`. If it fails to decode WebP, enable the `webp` feature of the `image` crate in `bench/harness/Cargo.toml`. Read `bench/README.md` for where the results go.
3. Compare with ADR-0028. Expected numbers: about +1.3% bytes vs TinyPNG, median dssim ratio about 0.95, worst file about 1.11x. The pass bar is ADR-0025: bytes at most +5%, median at most 1.0, no file above 2.0.
   - **If WebP passes**, go on.
   - **If it fails**, stop. Don't tune the quality. Report the numbers to the owner, because changing q71 needs a new ADR.
4. Docs:
   - `bench/harness/src/main.rs` top comment: remove "squish-core has no WebP codec yet ... skipped".
   - `prd.md`: tick "`webp.rs`" and "Skip animated files cleanly" in Phase 2. Update the status line at the top. Add the measured WebP numbers under the Phase 2 exit criteria in the same style as the Phase 3 status note.
   - `README.md` status: the CLI and web UI compress PNG, JPEG and WebP.
   - ADR-0028 already says the codec PR's harness run confirms q71. Only edit it if your numbers differ from its table, and then say so in the PR.
5. Push the branch and open a PR against `main` with `gh pr create`. Title: `Add the WebP codec`. Body: what changed, the harness numbers, and anything that surprised you. No AI attribution lines in the body either.

**Commit.** `Confirm WebP NFR-1 with the harness and mark the codec done in the PRD and README`

---

## T8. Effort presets (needs the owner's input first)

The PRD lists "Effort presets" as open, but `Effort` already exists, the CLI has `--effort` and the server reads `effort`. The PRD does not say what is missing.

**Before writing code, ask the owner what this item means.** Two likely readings:

- **A check that table 4.4 matches the code.** Write one test per format that pins each effort's settings (JPEG `FAST_QUALITY` 66, oxipng presets 1/2/5, WebP methods 2/4/6). If they all match, tick the box.
- **Named presets** that bundle quality, effort and metadata (for example a `web` or `archive` preset). That is new product design and needs an ADR before any code (ADR-0017 covers shared options).

**Commit (first reading).** `Pin each effort's encoder settings to PRD table 4.4 in tests`

---

## T9. Release profile, musl static build, binary size check

**Goal.** A static Linux binary, and CI fails if it reaches 15 MB (NFR-3, NFR-7).

**Read first.** Root `Cargo.toml` (`[profile.release]` is already set: fat LTO, one codegen unit, strip), `.github/workflows/ci.yml`, `rust-toolchain.toml`, ADR-0006.

**Change.**

1. Locally: `sudo apt-get install -y musl-tools`, then `rustup target add x86_64-unknown-linux-musl`, then `cargo build --release -p squish --target x86_64-unknown-linux-musl`. mozjpeg and libwebp compile C code, so the build needs `musl-gcc`. If the C build fails, try `CC_x86_64_unknown_linux_musl=musl-gcc` and report what worked.
2. Check that the binary is static: `file target/x86_64-unknown-linux-musl/release/squish` says "statically linked". Note its size with `stat -c %s`.
3. Run it once: `./target/x86_64-unknown-linux-musl/release/squish crates/core/tests/fixtures/photo.webp -o /tmp/out` must succeed.
4. Add a `release` job to `ci.yml` that installs `musl-tools` and `nasm`, adds the target, builds, and fails when the size is 15,000,000 bytes or more:
   ```sh
   size=$(stat -c %s target/x86_64-unknown-linux-musl/release/squish)
   echo "binary: $size bytes"
   test "$size" -lt 15000000
   ```
   Add the target to `rust-toolchain.toml` (`targets = [...]`) if CI's `rustup toolchain install` is the place that installs it.
5. Tick the PRD box and write the measured size next to it.

If the binary is over 15 MB, stop and report the size and the output of `cargo bloat --release -p squish --crates` (install with `cargo install cargo-bloat`). Don't remove features to make it fit.

**Commit.** `Build a static musl binary in CI and fail above 15 MB`

---

## T10. AVIF trigger check (owner decision, no code)

ADR-0012 builds AVIF only if one of its three triggers fires. A model can't answer the first two, so this task only gathers facts.

1. Count AVIF files in `bench/images/`: `ls bench/images | grep -ci '\.avif$'`.
2. Write the count and the three trigger questions in a short message to the owner:
   - Do you have real AVIF files to compress?
   - Has a user asked for AVIF?
   - (Only if the corpus has AVIF files) do they shrink meaningfully through TinyPNG? Compare sizes in `bench/images/` and `bench/tinypng/`.
3. Wait for the answer. If no trigger fires, update the Phase 2 checklist line in `prd.md` to say the trigger was checked on the date and did not fire, and answer open question 4. If one fires, AVIF gets its own plan. Don't start it here.

**Commit (no trigger).** `Record that the AVIF trigger did not fire in Phase 2`
