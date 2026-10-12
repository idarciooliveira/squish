# ADR-0019: CLI design

- Status: Accepted
- Date: 2026-10-12

## Context

The PRD lists the CLI's flags (FR-7 to FR-12) but leaves out the rules people run into on the second day. What happens when you run it twice on the same folder, which files a folder walk picks up, where `-o` puts nested files, what goes to stdout, and how `serve` fits next to a command that takes file paths.

## Decision

### Synopsis

```text
squish [OPTIONS] <PATH>...        compress files, folders, globs, or - for stdin
squish serve [SERVE OPTIONS]      start the web UI
```

Compressing is the default action, so there's no `compress` subcommand. `serve` is the only reserved word. To compress a file literally named `serve`, write `./serve`.

### Compress options

```text
Output (pick at most one; default writes name.min.ext next to the input)
  -o, --out-dir <DIR>      mirror inputs into DIR with their original names
      --in-place           overwrite the inputs (asks first unless --yes)
      --zip <FILE>         write every output into one STORE zip

Inputs
  -r, --recursive          descend into folders

Compression (grammar from ADR-0017)
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

### Serve options

```text
      --host <HOST>        [default: 127.0.0.1] [env: SQUISH_HOST]
  -p, --port <PORT>        [default: 8080]      [env: SQUISH_PORT]; 0 picks a free port
      --open               open the page in the default browser
  -j, --jobs <N>           concurrent compressions [default: core count]
      --max-body <MB>      [default: 25]
      --max-pixels <MP>    [default: 50]
      --timeout <SECS>     [default: 30]
```

On start it prints the real address, for example `squish listening on http://127.0.0.1:8080`.

### Which files get picked up

- A file named on the command line is always tried. If it isn't an image, it's reported as `unsupported`.
- A folder walk (with `-r`, or one level without it) only picks files with an image extension: `png`, `jpg`, `jpeg`, `webp`, `avif`, `gif`, `apng`. Running squish on `public/` doesn't report every `.css` file as unsupported. The format is still decided by magic bytes, not the extension.
- Walks skip files ending in `.min.<ext>`. Running squish twice on a folder doesn't produce `photo.min.min.jpg`.
- Walks skip hidden files and folders (`.git`, `.cache`) and don't follow symlinks.
- If the shell left a glob unexpanded, which is what happens on Windows, squish expands it itself.
- The same file reached twice is processed once.

### Where outputs go

| Mode | `images/icons/logo.png` becomes |
|---|---|
| default | `images/icons/logo.min.png` |
| `-o out` (input arg `images`) | `out/icons/logo.png` |
| `--zip all.zip` (input arg `images`) | entry `icons/logo.png` |
| `--in-place` | `images/icons/logo.png`, replaced |

- `-o` and `--zip` keep the path relative to the argument it came from. A file passed directly lands at the top level.
- If two inputs would map to the same output path or zip entry, squish stops before compressing anything and exits `2`.
- Existing outputs in default and `-o` modes are overwritten. Those are derived files. Only `--in-place` touches originals, and only it asks.
- The extension stays as written. `.jpeg` stays `.jpeg`.
- `already_optimized` files still get a copy in default, `-o` and `--zip` modes, so the output set is complete. With `--in-place` they aren't touched.
- Every file write goes to a temp file in the target folder, then gets renamed over the destination. An interrupted run never leaves half a file. `--in-place` keeps the original file's permissions.

### stdin and stdout

`squish -` reads one image from stdin and writes the result to stdout. It can't be mixed with other paths or output modes. The summary goes to stderr in this mode so the pipe stays clean.

```sh
curl -s https://example.com/a.png | squish - > a.png
```

### What prints where

- Progress bar: stderr, only when stderr is a TTY, hidden by `--json` and `--quiet`.
- Summary (PRD FR-11) or the JSON report: stdout.
- Per-file errors: listed in the summary, and on stderr as they happen.
- The `--in-place` prompt reads from the TTY. If there is no TTY and no `--yes`, squish exits `2` instead of hanging.

### JSON report

One document at the end, so `jq` works on it directly:

```json
{
  "version": "0.1.0",
  "summary": {
    "files": 48, "compressed": 44, "already_optimized": 3, "unsupported": 1, "failed": 0,
    "original_bytes": 191260262, "output_bytes": 54421094, "elapsed_ms": 3204, "jobs": 12
  },
  "files": [
    { "input": "images/a.png", "output": "out/a.png", "format": "png", "status": "compressed",
      "original_bytes": 182344, "output_bytes": 48211, "elapsed_ms": 84 },
    { "input": "images/hero.avif", "output": null, "format": "avif", "status": "unsupported",
      "error": "unsupported_format", "message": "AVIF is not supported yet" }
  ]
}
```

`status` and `error` use the names from ADR-0017. With `--dry-run`, `output` holds the path that would have been written.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | Every file was `compressed`, `already_optimized` or `unsupported` |
| `1` | At least one file `failed` |
| `2` | Usage error: bad flag, conflicting output modes, output collision, `--in-place` with no TTY and no `--yes` |
| `130` | Interrupted with Ctrl-C |

On Ctrl-C, squish stops starting new files, lets the running ones finish their rename, prints the summary for what got done and exits `130`. A second Ctrl-C exits at once. The temp-file-then-rename rule means that's still safe.

### Not in v1

A config file, shell completions, a `--suffix` option and a CI check mode such as `--fail-if-savings-above 10`. Each is small to add and gets its own ADR when needed.

## Consequences

- Running `squish ./images -r` twice is safe and does the same thing both times.
- Scripts get a stable JSON shape and exit codes that tell "bad input" apart from "squish broke".
- The default action with no subcommand keeps the common case short, at the cost of `serve` being a reserved name.
- Skipping `*.min.*` during walks means someone whose real files end in `.min.png` has to pass them by name.

## Alternatives considered

- **An explicit `squish compress` subcommand.** No reserved words, but every common command gets longer for a corner case.
- **NDJSON, one line per file as it finishes.** Better for watching a long run, worse for `jq` and for summaries. Can be added later as `--json=stream`.
- **Skip existing outputs instead of overwriting.** Leaves stale outputs after changing settings, which is harder to notice than an overwrite.
- **Report every non-image file in a walk as unsupported.** Accurate, but buries the useful lines under noise.
