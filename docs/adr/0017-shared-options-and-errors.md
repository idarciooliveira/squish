# ADR-0017: One options grammar and one error list, owned by the core

- Status: Accepted
- Date: 2026-10-12

## Context

The CLI and the HTTP API both take compression options and both report what happened to each file. If each side parses options and names errors its own way, they drift. `--quality 80` on the CLI and `?quality=80` on the API should mean the same thing, and a file that fails in the CLI should fail with the same reason in the browser.

Separately, the PRD's release profile sets `panic = "abort"`. The `mozjpeg` crate reports libjpeg errors by panicking and tells callers to wrap calls in `catch_unwind`. With `abort`, one malformed JPEG would kill the whole server, not just one request.

## Decision

### Options parse in the core

`squish-core` implements `FromStr` and `Display` for every option type, with no clap or serde dependency. The CLI and the server both call these, so the accepted strings are identical everywhere.

| Option | Accepted values | Default |
|---|---|---|
| `quality` | `auto`, `lossless`, or an integer `0` to `100` | `auto` |
| `effort` | `fast`, `default`, `max` | `default` |
| `metadata` | `strip-keep-icc`, `strip-all`, `keep-all` | `strip-keep-icc` |

### One error type with stable codes

```rust
#[non_exhaustive]
pub enum Error {
    Unsupported(Option<Format>), // unknown bytes, or a format not compiled in (AVIF)
    Animated(Format),
    TooManyPixels { width: u32, height: u32, limit: u64 },
    Decode(String),
    Encode(String),
}

impl Error {
    pub fn code(&self) -> &'static str; // stable, snake_case
}
```

| Variant | `code()` |
|---|---|
| `Unsupported` | `unsupported_format` |
| `Animated` | `animated_image` |
| `TooManyPixels` | `too_many_pixels` |
| `Decode` | `decode_failed` |
| `Encode` | `encode_failed` |

The codes are part of the public contract. The CLI's JSON output and the API's error bodies both use them. Renaming one needs a new ADR.

### Four outcomes per file

Every file ends in exactly one status, and the CLI summary, the CLI JSON and the web UI rows all use these names:

| Status | When |
|---|---|
| `compressed` | Output is smaller than the input |
| `already_optimized` | The size guard kept the original (ADR-0011) |
| `unsupported` | `Unsupported` or `Animated` |
| `failed` | Any other error, or a panic inside a codec |

`unsupported` doesn't count as a failure for exit codes. `failed` does.

### Panics are caught per file

The release profile uses `panic = "unwind"`, not `abort`. The CLI wraps each job in `catch_unwind` and marks the file `failed`. The server gets the same result for free, because a panic inside `spawn_blocking` comes back as a `JoinError`, which becomes a `500`.

## Consequences

- A flag value that works in the CLI works as a query parameter, with no second parser to keep in sync.
- Scripts and the web UI can switch on error codes instead of matching message text.
- A codec bug costs one file or one request, not the process.
- `unwind` makes the binary a bit bigger than `abort`. We measure it against the 15 MB target (NFR-3) in Phase 2.

## Alternatives considered

- **Parse options separately in clap and in axum extractors.** Two grammars that will drift.
- **Keep `panic = "abort"` and trust the fuzzer.** mozjpeg panics by design on bad input, so fuzzing can't remove that path.
- **Error codes as HTTP status only.** `413` covers both "body too large" and "too many pixels", and the CLI has no status codes at all.
