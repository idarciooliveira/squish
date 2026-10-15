# CLI reference

Source of truth for humans and LLMs. The binary prints the same via
`squish --help`, `squish help`, `squish serve --help`, `squish help serve`
and `squish serve help`. If this file and `--help` disagree, `--help` wins.

## Synopsis

```text
squish [OPTIONS] <PATH>...     compress files, folders, globs, or - for stdin
squish help [serve]            show help (same text as --help)
squish serve [SERVE OPTIONS]   start the web UI and the HTTP API
```

Compressing is the default action, so there is no `compress` subcommand.
`serve` and `help` are reserved words (ADR-0019): to compress a file
literally named `serve` or `help`, write `./serve` or `./help`.

## Examples

```sh
squish photo.jpg                   # writes photo.min.jpg next to it
squish ./images -r -o ./out        # folder, recursively, mirrored into ./out
squish ./images --in-place --yes   # replace the originals
squish - < in.png > out.png        # stdin to stdout
squish ./images --zip squish.zip   # everything into one zip
squish serve --open                # web UI on http://127.0.0.1:8080
```

## Compress options

```text
Output (pick at most one; default writes name.min.ext next to the input)
  -o, --out-dir <DIR>      mirror inputs into DIR with their original names
      --in-place           overwrite the inputs (asks first unless --yes)
      --zip <FILE>         write every output into one STORE zip

Inputs
  -r, --recursive          descend into folders

Compression
      --quality <Q>        auto | lossless | 0-100       [default: auto]
      --lossless           same as --quality lossless
  -e, --effort <E>         fast | default | max          [default: default]
      --metadata <M>       strip-keep-icc | strip-all | keep-all
      --max-pixels <MP>    refuse images above this many megapixels [default: 50]

Run
  -j, --jobs <N>           parallel jobs                 [default: core count]
  -n, --dry-run            compress in memory, write nothing, report savings
      --json               print a JSON report on stdout instead of text
  -y, --yes                don't ask before --in-place
  -q, --quiet              no progress bar, no summary; errors still print
```

`--quality` has no short form, so `-q` keeps its usual meaning of quiet.

## Serve options

```text
      --host <HOST>        [default: 127.0.0.1] [env: SQUISH_HOST]
  -p, --port <PORT>        [default: 8080]      [env: SQUISH_PORT]; 0 picks a free port
      --open               open the page in the default browser
  -j, --jobs <N>           concurrent compressions [default: core count]
      --max-body <MB>      [default: 25]
      --max-pixels <MP>    [default: 50]
      --timeout <SECS>     [default: 30]
```

Port precedence: `--port` > `$SQUISH_PORT` > `$PORT` > `8080`.
`serve` only listens on localhost unless passed `--host 0.0.0.0`.
On start it prints `squish listening on http://<addr>`.
The page and the HTTP API are described in [ADR-0018](adr/0018-http-api.md).

## Rules LLMs should follow

- One image format in, the same format out. PNG stays PNG, JPEG stays JPEG.
- A file named on the command line is always tried; anything else is
  reported as `unsupported`, never skipped silently.
- A folder walk (one level by default, full tree with `-r`) only picks up
  image extensions (`png`, `jpg`, `jpeg`, `webp`, `avif`, `gif`, `apng`),
  skips `*.min.*`, hidden files/folders and symlinks. The format is decided
  by magic bytes, not the extension.
- Default and `-o` modes overwrite derived outputs without asking. Only
  `--in-place` touches originals, and only it prompts (needs `--yes` when
  stdin is not a TTY).
- If two inputs map to the same output path or zip entry, stop before any
  work and exit `2`.
- `squish -` reads one image from stdin and writes it to stdout; it cannot be
  mixed with other paths, `--out-dir`, `--in-place`, `--zip` or `--json`.
  The summary goes to stderr so the pipe stays clean. Unsupported stdin input
  passes through unchanged; broken input exits `1` with empty stdout.
- Progress bar: stderr, only on a TTY, hidden by `--json`/`--quiet`.
  Summary or JSON report: stdout. Per-file errors: also stderr as they happen.
- `--dry-run` writes nothing but still runs collision checks and reports
  savings. `--json` prints one document shaped like ADR-0019, so `jq` works.
- Every file write is temp-file-then-rename; Ctrl-C stops new files, finishes
  running renames, prints the summary and exits `130`. A second Ctrl-C exits
  at once. A `--zip` interrupted this way still finishes with completed entries.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Every file was `compressed`, `already_optimized` or `unsupported` |
| `1` | At least one file `failed` |
| `2` | Usage error: bad flag, conflicting output modes, output collision, `--in-place` with no TTY and no `--yes`, unknown `help` topic |
| `130` | Interrupted with Ctrl-C |

Full rules: [ADR-0019](adr/0019-cli-design.md).
