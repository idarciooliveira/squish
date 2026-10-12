# ADR-0024: Per-file quality search is the default

- Status: Accepted
- Date: 2026-10-12
- Supersedes: in ADR-0009, the fixed `auto` starting values and the deferral of the automatic mode to Phase 4. The rest of ADR-0009 stands.

## Context

ADR-0009 made `--quality auto` the default with fixed values per format (PNG quality 65 to 80, JPEG 80), and put an SSIM-driven search off until Phase 4. Phase 0 showed that fixed values can't meet NFR-1:

- **JPEG.** mozjpeg at a fixed q75 came out 23% bigger than TinyPNG, because TinyPNG is lossier on most photos. Searching quality per file met TinyPNG's quality on all 21 files at 4.4% fewer bytes. The chosen quality ranged from q61 to q90, with a median of q66, so no single value fits.
- **PNG.** A fixed quality range made imagequant smaller than TinyPNG on screenshots only by being lossier, and bigger on alpha photos. Searching the palette size per file is what got imagequant to 41 of 41 files at 6.2% fewer bytes, and quantizr (ADR-0022) to its best result.

TinyPNG's own quality varies a lot per file. Its dssim on `bench/` runs from 0.0011 to 0.0049 for JPEG (median 0.0022) and from 0 to 0.0025 for PNG (median 0.0003). That points to TinyPNG choosing per file too.

## Decision

1. `--quality auto`, still the default, means a per-file search: the lowest setting whose quality score meets a per-format target.
   - JPEG: binary search over mozjpeg quality.
   - PNG: binary search over quantizr's palette size, 2 to 256 colors. If 256 colors misses the target, keep 256 (ADR-0022).
2. The score comes from a permissive metric in `squish-core`, chosen in Phase 1 from `ssimulacra2`, `butteraugli` and `image-compare`. `dssim` stays bench-only because it's AGPL.
3. The targets get calibrated in Phase 1 by running the chosen metric over `bench/` until NFR-1 holds. The Phase 0 reference points in dssim are the TinyPNG medians above.
4. `--effort fast` skips the search and uses fixed values: JPEG q66 and PNG at up to 256 colors.
5. `--quality N` stays a fixed setting with no search.

## Consequences

- Meeting NFR-1 becomes possible in Phase 1, where the PRD's exit criteria expect it.
- Encoding gets slower. In the spike, built at opt-level 2 with dssim as the metric, the JPEG search took a median of 10 s per file (62 s max) and the PNG search 0.8 s (5 s max). Phase 1 has to cut that: a narrower starting range around q66, fewer steps, and scoring a downscaled copy are the first things to try.
- `squish-core` gains a metric dependency and a search loop that every format shares.
- WebP gets the same treatment in Phase 2.

## Alternatives considered

- **Fixed defaults, tuned harder.** Measured above. Can't track TinyPNG on both photos and screenshots.
- **Search with dssim.** Its license is AGPL, which ADR-0004 rules out.
- **Keep the search for Phase 4.** It would mean shipping Phase 1 without meeting NFR-1.
