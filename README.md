# squish

A fast, local image compressor in the spirit of [TinyPNG](https://tinypng.com). Give it PNG, JPEG or WebP files and get smaller files back in the same format. Use it as a CLI, or run `squish serve` to get a web page where you can drop images and download the results one by one or as a zip.

It's written in Rust and ships as a single binary with no runtime dependencies.

> **Status:** planning. No code yet. Read the [PRD](prd.md) for the full plan and [`docs/adr/`](docs/adr/README.md) for the decisions behind it.

## Planned usage

```sh
squish photo.jpg                     # writes photo.min.jpg next to it
squish ./images -r -o ./out          # a whole folder, recursively
squish ./images --zip squish.zip     # everything into one zip
squish serve                         # web UI on http://127.0.0.1:8080
```

## Contributing

Turn on the repository's git hooks after cloning:

```sh
git config core.hooksPath .githooks
```

Commit messages are one line, and the owner is the only author. See [ADR-0015](docs/adr/0015-commit-policy.md).
