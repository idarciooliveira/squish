# ADR-0009: No knobs by default, settings for those who want them

- Status: Accepted. Fixed `auto` values and the Phase 4 deferral superseded by ADR-0024.
- Date: 2026-10-11

## Context

Much of TinyPNG's appeal is that you drop files and get good results without choosing anything. Power users still want control over quality and how long encoding takes.

## Decision

- The default is `--quality auto`, with per-format defaults tuned in Phase 0 (starting points: PNG quality range 65 to 80, JPEG 80, WebP 75).
- Three effort presets: `fast`, `default`, `max`. `default` is the out-of-the-box choice.
- The CLI exposes `--quality`, `--lossless` and `--effort` flags.
- The web UI hides those settings in a drawer that's closed by default.
- An automatic mode that searches for the lowest quality above an SSIM target is planned for Phase 4. When it ships, it may replace the fixed `auto` defaults through a new ADR.

## Consequences

- The default path needs no explanation.
- We own the defaults, and they have to be good. Phase 0 exists to set them from data.

## Alternatives considered

- **A quality slider always visible.** More control, more confusion, and the defaults matter less.
- **SSIM-driven auto quality from day one.** Best results, but slower and more complex. Deferred to Phase 4.
