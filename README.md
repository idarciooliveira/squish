# squish

A fast, local image compressor in the spirit of [TinyPNG](https://tinypng.com). Give it PNG, JPEG or WebP files and get smaller files back in the same format. Use it as a CLI, or run `squish serve` to get a web page where you can drop images and download the results one by one or as a zip.

It's written in Rust and ships as a single binary with no runtime dependencies.

> **Status:** the CLI compresses PNG and JPEG. WebP, `--zip` and the web UI are next. Read the [PRD](prd.md) for the full plan and [`docs/adr/`](docs/adr/README.md) for the decisions behind it.

## Usage

```sh
cargo build --release                # binary at target/release/squish

squish photo.jpg                     # writes photo.min.jpg next to it
squish ./images -r -o ./out          # a whole folder, recursively, mirrored into ./out
squish ./images --in-place --yes     # replace the originals
squish - < in.png > out.png          # stdin to stdout
```

Coming later:

```sh
squish ./images --zip squish.zip     # everything into one zip
squish serve                         # web UI on http://127.0.0.1:8080
```

`squish --help` lists every flag. [ADR-0019](docs/adr/0019-cli-design.md) has the rules for which files a folder walk picks up, where outputs go and what the exit codes mean.

## Contributing

Turn on the repository's git hooks after cloning:

```sh
git config core.hooksPath .githooks
```

Commit messages are one line, and the owner is the only author. See [ADR-0015](docs/adr/0015-commit-policy.md).
