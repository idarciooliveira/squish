# ADR-0026: fast-ssim2 at about 1 MP as the quality metric

- Status: Accepted
- Date: 2026-10-12
- Completes: ADR-0024 points 2 and 3 (pick a permissive metric, calibrate the targets)

## Context

ADR-0024 made `--quality auto` a per-file search against a quality target, and left the metric and the targets to Phase 1. The search also has to be fast enough for batches. The Phase 0 spike took a median of 10 s per JPEG.

`bench/spike/src/bin/curves.rs` encoded every bench file over a grid (mozjpeg q30 to q93 in steps of 3, quantizr 2 to 256 colors). Each result was scored with dssim and every candidate metric at three scales: full size, half size, and box-downscaled to about 1 MP. Each step was timed. Two JPEGs above 30 MP were scored at ~1 MP only, to fit in memory. `bench/scripts/analyse-curves.py` turns that into `bench/results/curves-analysis.txt`. "Bytes" below means the cheapest setting that meets NFR-1 as defined in ADR-0025.

| metric, scale | license | JPEG bytes at NFR-1 | spread of TinyPNG's JPEG scores (cv) |
|---|---|---|---|
| fast-ssim2 (fs2), ~1 MP | BSD-2-Clause | +3.5% (fs2 ≥ 89.3) | 0.03 |
| butteraugli 3-norm, ~1 MP | BSD-3-Clause | +7.9% | 0.42 |
| butteraugli max-norm, ~1 MP | BSD-3-Clause | +10.5% | 0.34 |
| fs2, full size (18 files) | | +115% | 0.13 |
| fs2, half size (18 files) | | +149% | 0.06 |
| image-compare SSIM and hybrid, ~1 MP | MIT | +75% to +85% | 0.07 to 0.08 |

- fs2 at about 1 MP is the only candidate that meets NFR-1 on JPEG within the byte budget. TinyPNG's own JPEGs land in a narrow band on it, 80 to 90.5. That suggests TinyPNG targets something similar: perceived quality at a normal viewing size, not pixel-level error.
- On PNG, quantizr's limit decides most files (ADR-0022), so the metrics barely differ. fs2 at ~1 MP ≥ 90 gives −2.1% bytes and a median ratio of 0.87.
- fast-ssim2 0.9 gives the same scores as the reference `ssimulacra2` 0.5 crate (within 0.24 points on 33 files) at about a tenth of the CPU.
- Scoring at ~1 MP also caps the metric's memory. The bench run with full-size metrics ran out of 16 GB on the 40 MP and 66 MP photos.

The search's cost per probe was measured at the median, CPU time, release build:

| | JPEG | PNG |
|---|---|---|
| encode a probe | 416 ms (mozjpeg, progressive) | 21 ms (quantizr) |
| decode, or palette lookup | 45 ms | in the above |
| downscale to ~1 MP | 12 ms | 4 ms |
| fs2 at ~1 MP | 63 ms | 30 ms |
| final step, once | none | 186 ms (oxipng) |

## Decision

1. The metric is fast-ssim2 (`fast-ssim2` crate, BSD-2-Clause) on a copy box-downscaled by an integer factor to at most about 1 MP. Images with alpha are composited over white and over black, and the lower score counts.
2. Starting targets: JPEG fs2 ≥ 89.3, PNG fs2 ≥ 90. Phase 1 recalibrates them with the real search, which steps one quality point at a time. The grid stepped by 3, which cost about 3% in bytes against the Phase 0 sweep.
3. The search, per format:
   - JPEG: start at q66. Pick the next probe by interpolating between the two closest scores, with a bracketed search as the fallback. Stop when the bracket is 1 quality point wide or after 5 probes. Probes are encoded baseline. Only the final encode is progressive, which doesn't change pixels.
   - PNG: bracketed search over the palette size, 2 to 256 colors, scored on the remapped pixels. oxipng runs only on the final palette.
4. Speed budget, measured on `bench/` with a release build: the search, including the final encode, costs at most 4x one plain encode at the median, and no file goes over 10 s of CPU. On the grid data, 4 probes cost 5.6x one encode for JPEG and 2.4x for PNG. The baseline probes, the smarter start and the cheap decode in point 5 are how JPEG gets under 4x.
5. Things to try first if JPEG misses the budget: decode probes with libjpeg's DCT scaling (1/2, 1/4 or 1/8), which skips most of the decode and the downscale.
6. Large images. On the grid, the worst JPEG is the 40 MP `image (7).jpg`. At fs2 ≥ 89 it lands at 2.09x, and it alone pushes the JPEG target up to 89.3, which costs about 3% in bytes on every other file. Downscaled 7x, its artifacts don't show. Phase 1 tries a higher cap (2 to 4 MP) for images above about 16 MP. It keeps that cap only if it brings that file under 2x without breaking the speed budget, and then lowers the JPEG target to match.
7. `--effort fast` keeps the fixed settings from ADR-0024. `--effort max` may raise the probe limit. It doesn't change the target.

## Consequences

- `squish-core` gains `fast-ssim2` and a `search` module shared by the codecs. ADR-0020's tree gets `metric.rs` and `search.rs` next to `quant/`.
- fast-ssim2 needs Rust 1.89 or newer. The toolchain is 1.99.
- Score memory is bounded by the 1 MP copy, plus the reference precompute. It no longer grows with the input.
- Targets are numbers on fs2's scale, 0 to 100, and they show up in `--json` output and `/api/info`.

## Alternatives considered

- **butteraugli.** It tracks dssim about as well per call, but TinyPNG's scores on it are spread out, so a fixed target costs 8 to 10% more bytes. It's also slower.
- **Full-size or half-size scoring.** Slower, more memory, and much worse agreement with TinyPNG's choices.
- **image-compare.** Its SSIM isn't perceptual enough. It needs +75% bytes or more to meet NFR-1.
- **rust-av `ssimulacra2`.** Same scores, 10x the CPU.
