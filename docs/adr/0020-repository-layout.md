# ADR-0020: Repository layout

- Status: Accepted
- Date: 2026-10-12

## Context

ADR-0006 fixed three crates in one workspace. Before Phase 0 writes its first file, we need to know where code, tests, the web UI, benchmark data and throwaway spikes live.

Phase 0 also links `imagequant` (GPL) and `dssim` (AGPL) for comparison. Those must never end up in the squish binary (ADR-0004), and the easiest way to prevent it is to keep them out of the workspace entirely.

## Decision

```text
.
├── Cargo.toml                  # [workspace], [workspace.dependencies], [workspace.lints], release profile
├── Cargo.lock                  # committed, squish is a binary
├── rust-toolchain.toml         # pinned stable toolchain
├── .gitignore                  # target/, bench/images/, bench/out/
├── crates/
│   ├── core/                   # package squish-core: bytes in, bytes out (ADR-0006)
│   │   ├── src/
│   │   │   ├── lib.rs          # public API: detect, compress, Options, Output, Error, Format
│   │   │   ├── format.rs       # magic bytes, animation check, header dimensions
│   │   │   ├── options.rs      # Options, Quality, Effort, MetaPolicy, FromStr (ADR-0017)
│   │   │   ├── error.rs        # Error and its stable codes (ADR-0017)
│   │   │   ├── guard.rs        # size guard and early skips (ADR-0011)
│   │   │   ├── metadata.rs     # ICC and orientation handling (ADR-0010)
│   │   │   ├── codec/
│   │   │   │   ├── mod.rs      # dispatch by Format
│   │   │   │   ├── png.rs
│   │   │   │   ├── jpeg.rs
│   │   │   │   ├── webp.rs     # feature "webp"
│   │   │   │   └── avif.rs     # feature "avif", only if ADR-0012 fires
│   │   │   └── quant/
│   │   │       ├── mod.rs      # Quantizer trait
│   │   │       └── quantette.rs
│   │   └── tests/
│   │       ├── fixtures/       # tiny images, committed, under 1 MB in total
│   │       ├── detect.rs
│   │       ├── compress.rs
│   │       └── golden.rs       # runs over bench/images/ when present
│   ├── server/                 # package squish-server: axum router and embedded UI
│   │   ├── src/
│   │   │   ├── lib.rs          # pub fn router(Config) -> Router, pub async fn run(Config)
│   │   │   ├── config.rs       # limits and bind address
│   │   │   ├── api.rs          # /api/compress, /api/info, /api/health (ADR-0018)
│   │   │   ├── assets.rs       # rust-embed handler for web/
│   │   │   └── error.rs        # ApiError -> status and JSON body
│   │   └── tests/routes.rs     # one test per route and per error status
│   └── cli/                    # package squish, the binary
│       ├── src/
│       │   ├── main.rs         # parse args, dispatch, map to exit code
│       │   ├── args.rs         # clap definitions (ADR-0019)
│       │   ├── inputs.rs       # paths, globs, folder walks -> list of jobs
│       │   ├── outputs.rs      # suffix, out-dir, in-place, zip, stdout writers
│       │   ├── run.rs          # rayon pool, progress bar, Ctrl-C
│       │   ├── report.rs       # text summary and JSON report
│       │   └── serve.rs        # `squish serve` -> squish_server::run
│       └── tests/cli.rs        # end-to-end runs in temp folders
├── web/                        # embedded by squish-server (ADR-0013)
│   ├── index.html
│   ├── app.js
│   ├── style.css
│   └── vendor/                 # the zip library, vendored (ADR-0008)
├── bench/                      # NOT a workspace member
│   ├── README.md               # how to fill images/ and run a benchmark
│   ├── images/                 # the corpus, gitignored
│   ├── tinypng/                # TinyPNG baseline outputs, gitignored
│   ├── spike/                  # Phase 0 throwaway Cargo project, may link GPL/AGPL crates
│   ├── scripts/
│   └── results/                # CSVs per run, committed
├── fuzz/                       # cargo-fuzz targets, Phase 4, not a workspace member
├── docs/adr/
├── .githooks/
├── prd.md
└── README.md
```

### Rules that go with the tree

- **Plain folder names, prefixed package names.** Folders say what the crate is: `crates/core`, `crates/server`, `crates/cli`, next to the root `web/`. Packages keep the project prefix, `squish-core`, `squish-server` and `squish`, because Cargo package names share one namespace with every dependency, and a package called `core` or `server` would be ambiguous in `Cargo.lock`, error messages and `cargo test -p`. The binary is `squish`.
- **No dependency renames.** Members depend on each other by package name, for example `squish-core = { path = "../core" }`, and code writes `squish_core::` and `squish_server::`. Cargo's `package = "..."` rename isn't used to shorten these.
- **Dependency direction.** `squish` depends on `squish-server` and `squish-core`. `squish-server` depends on `squish-core`. `squish-core` depends on neither and never on clap, axum, tokio, rayon or serde.
- **Versions live once.** Every third-party crate is declared in `[workspace.dependencies]` and members use `foo.workspace = true`.
- **Lints live once.** `[workspace.lints]` sets `unsafe_code = "forbid"` and the clippy defaults, and every member inherits them. Unsafe code stays inside the codec crates we depend on.
- **Cargo features.** `squish-core` has `webp` and `avif` features and one feature per quantizer (`quant-quantette` on by default). The `squish` package has a `server` feature, on by default, that a CLI-only build can turn off (the option ADR-0006 left open).
- **Workspace membership.** `members = ["crates/*"]`. `bench/spike` and `fuzz` sit outside, so nothing in them can become a dependency of the shipped binary by accident.
- **Tests sit next to their crate.** Unit tests in `#[cfg(test)]` modules, integration tests in each crate's `tests/`. There's no top-level `tests/` folder, since a virtual workspace root can't own one.
- **Golden tests don't need the corpus.** `golden.rs` skips with a message when `bench/images/` is empty, so a fresh clone passes `cargo test`.

## Consequences

- The GPL and AGPL comparison crates can't leak into the product. A license check only needs to look at `crates/`.
- Image collections stay out of git history. Only small fixtures and result CSVs are committed.
- Each crate's tests run alone with `cargo test -p <name>`.
- `web/` sits at the root, so someone working on the UI doesn't have to open a Rust crate. The server reaches it with a relative path in its rust-embed attribute, which means `squish-server` can't be published to crates.io as is. We don't plan to publish it.

## Alternatives considered

- **Folder name equals package name (`crates/squish-core`).** Paths and `cargo -p` names match, but every path repeats the project name and the folders read less plainly than `core`, `server`, `cli` and `web`.
- **Rename dependencies in `Cargo.toml` (`server = { package = "squish-server" }`).** Short names in code, but it can't work for the core: a dependency named `core` shadows Rust's built-in crate and breaks every macro that expands to `::core::...`, such as serde's and thiserror's. Renaming only the server leaves one short name next to one prefixed name. ripgrep uses the plain-folder, prefixed-package pattern (`crates/cli` holds `grep-cli`).
- **Plain package names too (`core`, `server`).** `core` collides with Rust's built-in `core` crate, and `server` says nothing in a dependency tree.
- **Commit the benchmark corpus, or use Git LFS.** Makes results reproducible from a clone, but bloats history, and some photos may not be ours to redistribute.
- **The Phase 0 spike as a workspace member.** One `cargo build` for everything, and one careless `use` away from shipping GPL code.
- **`web/` inside `crates/server/`.** Publishable crate, but the frontend becomes harder to find.
