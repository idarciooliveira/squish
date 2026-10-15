# Phase 4: ship and extras — task plan

This plan finishes `prd.md` §Phase 4. Phase 3 is already merged (PR #5:
`squish-server` with the ADR-0018 routes, embedded web UI, `squish serve`);
its only gaps are the Firefox check and a post-WebP 50-file retest, tracked
separately. Do the tasks in order. T4 is an owner decision that gates T5.
T6 is parked and needs ADRs before any code.

## Rules for every task

1. **Read before writing.** Open every file in the task's "Read first" list.
2. **No new design.** ADRs 0006, 0007, 0014, 0015, 0020, 0021 already decide
   the shape. If a step contradicts an ADR or the PRD, stop and ask.
3. **Stay inside the task.** Touch only the files the task names.
4. **Branch.** One branch per task from `main`: `phase4-release`,
   `phase4-docker`, `phase4-fuzz`. T4/T5/T6 produce no code branches until
   the owner decides.
5. **Checks before every commit.**
   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo check -p squish-core --no-default-features
   cargo test --workspace
   ```
6. **Commit policy (ADR-0015).** One-line messages, no `Co-authored-by`,
   no AI attribution. Author `idarciooliveira <idarciooliveira@gmail.com>`.

---

## T1. Release binaries for Linux, macOS, Windows (NFR-7)

**Goal.** Tagged pushes produce downloadable binaries; PRs still only run
`test` + the musl size gate. NFR-3 (15 MB) keeps applying per artifact.

**Read first.** `.github/workflows/ci.yml`, `rust-toolchain.toml`,
root `Cargo.toml` (`[profile.release]`, workspace `exclude`), ADR-0006,
ADR-0023 (mozjpeg builds without nasm, just slower).

**Change.**

1. Add a `publish` job (or workflow) that runs on `tags: ["v*"]` with a
   matrix — minimum: `x86_64-unknown-linux-musl` (existing static build),
   `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`,
   `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`. macOS/Windows link
   the C codecs natively; musl stays the `FROM scratch` input for T2.
2. Install per-runner build deps (`musl-tools`, `nasm` on Linux;
   `nasm` handling on macOS/Windows per what the `mozjpeg`/`libwebp-sys`
   build scripts accept). If a target's C build fails, report the log and
   stop — don't swap encoders to make it fit.
3. Upload each binary as a release asset (`gh release` / `upload-artifact`
   then attach on tag). Name them `squish-<target>[/.exe]`.
4. Keep the PR path fast: the existing `test` and `release` (size-gate)
   jobs stay as they are on `pull_request`.

**Done when.** Pushing a `v0.1.0-rc1`-style tag on a branch produces all
matrix assets, each runs `--help`, and the musl asset is still < 15 MB.

**Commit.** `Publish release binaries for Linux, macOS and Windows on tags`

---

## T2. Dockerfile FROM scratch (static binary only)

**Goal.** `docker build -t squish . && docker run -p 8080:8080 squish`
serves the web UI with nothing else in the image.

**Read first.** `crates/server/src/config.rs` (default host/port),
`crates/cli/src/serve.rs`, ADR-0014 (localhost default — the Dockerfile
must pass `--host 0.0.0.0` or document `SQUISH_HOST`).

**Change.**

1. Multi-stage `Dockerfile` at the repo root: stage 1 builds
   `cargo build --release -p squish --target x86_64-unknown-linux-musl`;
   stage 2 is `FROM scratch`, copies only that binary, `EXPOSE 8080`,
   `ENTRYPOINT ["/squish", "serve", "--host", "0.0.0.0", "--port", "8080"]`.
2. `.dockerignore` excluding `bench/images/`, `bench/tinypng/`, `target/`,
   `fuzz/corpus` (T3).
3. Document one `docker build` + `docker run` pair in `README.md`. No
   compose file, no registry push in this task.

**Done when.** A fresh `docker build` + `curl localhost:8080/api/health`
returns `ok`, and `POST /api/compress` shrinks a fixture.

**Commit.** `Serve the static binary from a FROM scratch image`

---

## T3. Fuzz detect and compress (NFR-6)

**Goal.** `cargo fuzz` on `detect` + `compress` runs an hour with no crash
or panic (mozjpeg panics become `Error`, per the release profile note).

**Read first.** `crates/core/src/lib.rs` (`detect`/`compress`/`preflight`),
`crates/core/src/error.rs`, architecture diagram in `prd.md` §7
(`fuzz/` lives **outside** the workspace).

**Change.**

1. `fuzz/` with `cargo-fuzz`: one target
   `fuzz_compress(bytes, quality, effort, metadata)` that calls
   `squish_core::detect` then `squish_core::compress` with bounded options
   (fixed qualities sampled, all efforts, all metadata policies).
   Seed corpus = `crates/core/tests/fixtures/*` (all formats, incl.
   `animated.*`).
2. Document in `fuzz/README.md`: `cargo fuzz run fuzz_compress -- -max_total_time=3600`,
   plus how to replay a crasher into a regression test.
3. File any crasher as a failing unit test first, fix second — never just
   extend the timeout.

**Done when.** 60-minute run locally (or in a nightly workflow, if cheap)
reports no crash, and the seed corpus runs in CI in under a minute.

**Commit.** `Fuzz detect and compress with a fixture-seeded corpus`

---

## T4. Public-web decision (owner decision, no code)

ADR-0014 defers rate limiting, privacy note and deployment docs until we
decide to go public; PRD open question 5 asks the same. A model can't
answer it. Gather facts and wait:

1. Current posture: binds `127.0.0.1:8080`, no rate limit, no telemetry
   (NFR-8), stateless (ADR-0007), limits already on (body 25 MB, pixels,
   semaphore+429, 30 s timeout).
2. Ask the owner: public demo, or stay local-first? If public, that work
   gets its own ADR (per-IP limits, abuse notes, hosting) before T5.
3. If local stays, record it in `prd.md` Q5 with the date. Don't build
   rate limiting speculatively.

**Commit (stay-local).** `Record that the web version stays local-first`

---

## T5. Tinify-compatible routes, only if a trigger fires (deferred)

ADR-0021 is `Deferred` and sets two triggers: (1) a concrete integration
that calls Tinify needs squish, or (2) T4 goes public. Until one fires,
keep `/shrink` and `/output/` free and write no code. When one fires:

1. Capture real Tinify responses with a free key into
   `crates/server/tests/fixtures/tinify/` (empty body, bad image, bad key,
   unsupported option) — the route tests compare against these.
2. Implement `squish serve --tinify` per ADR-0021 (in-memory cache,
   10-min TTL, 256 MB cap, `--tinify-key`, `--public-url`, `preserve`
   only, everything else `400`). The web UI keeps using `/api/compress`.
3. New ADR flipping 0021 to Accepted, and a PRD update.

---

## T6. WASM mode, resize, format conversion (parked, not v1)

PRD non-goals: no format conversion, no resizing/editing. WASM is listed
as "optional experiment after the server version works". None of these
starts in Phase 4 without a new ADR each (options grammar per ADR-0017,
binary-size impact on NFR-3, privacy story for WASM). This task is a
parking lot, not work.
