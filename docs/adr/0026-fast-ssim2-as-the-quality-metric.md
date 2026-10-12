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

The search's cost per probe was measured at the median, CPU time, release build. Phase 0 grid estimates first, then the Phase 1 harness measurements:

| | JPEG | PNG |
|---|---|---|
| encode a probe | 416 ms (mozjpeg, progressive) | 21 ms (quantizr) |
| encode a probe, `Profile::Fast` (no trellis) | 50-90 ms | |
| decode, or palette lookup | 45-80 ms full; DCT 1/4 decode 146 ms on a 32 MP file | in the above |
| downscale to ~1 MP | 12 ms | 4 ms |
| fs2 at ~1 MP | 63 ms | 30 ms |
| fs2 at ~1 MP, measured in Phase 1 (fast-ssim2 0.9) | ~120 ms per compare on synthetic data; ~50 ms floor on small images | in the above |
| final step, once | none | 186 ms (oxipng) |

## Decision

1. The metric is fast-ssim2 (`fast-ssim2` crate, BSD-2-Clause) on a copy box-downscaled by an integer factor to at most about 1 MP. Images with alpha are composited over white and over black, and the lower score counts.
2. Starting targets: JPEG fs2 ≥ 89.3, PNG fs2 ≥ 90. **Recalibrated in Phase 1:** JPEG fs2 ≥ 88.7, PNG fs2 ≥ 90 unchanged. The recalibration measures what TinyPNG's own outputs score on this metric over the bench corpus (median 89.4) and aims just under it. At 88.7 JPEG takes −1.3% of TinyPNG's total bytes with a median dssim ratio of 0.82 and a worst file of 1.83; at 89.3 the same search cost +7.5% bytes for quality no file needed (median dssim ratio 0.70). TinyPNG's own per-file scores still spread from 80 (`image (18).jpg`) to 92.7, and the fixed target plus the size guard handle that spread better than a stricter one did.
3. The search, per format:
   - JPEG: two stages. Stage 1: cheap `Profile::Fast` (baseline) probes from q74, interpolated with slope moves, at most 3 probes, stopping within 1.0 point of the target. Stage 2: mozjpeg trellis probes within three quality points of stage 1's landing (trellis at the same quality lands within ~1 point, but costs about 15× a baseline encode), at most 2 probes (3 for `max`), stopping within 0.7. Stage 2 never spends a probe on q95 on a miss: the best fitting encode below the size guard takes over instead, or the size guard keeps the input.
   - PNG: bracketed search over the palette index, top first, scored on the quantized remap. oxipng runs only on the final palette.
4. Speed budget, measured on `bench/` with a release build: the search, including the final encode, costs at most 4x one plain encode at the median, and no file goes over 10 s of CPU. The two-stage search lands on **3.7x** median (slowest 3.4 s) for JPEG and 1.8x for PNG (runs of 2026-10-12, `bench/results/phase1-final.csv`).
5. Things to try first if JPEG misses the budget: decode probes with libjpeg's DCT scaling (1/2, 1/4 or 1/8), which skips most of the decode and the downscale. **Tried in Phase 1 and shelved.** The DCT decode saved only about 30% of the decode (146 ms at 1/4 against 208 ms full on a 32 MP file). Scaled copies also score differently: TinyPNG's median shifted from 89.3 to 88.4 on them, and the fixed target then left the worst files far off (dssim ratio 3.2-3.4 on `image (1).jpg` across targets 88.9 to 89.5, over the 2.0 limit). Box-averaged scoring stayed; the probe caps in point 3 got JPEG under the budget instead.
6. Large images. On the grid, the worst JPEG is the 40 MP `image (7).jpg`, at fs2 ≥ 89 it lands at 2.09x. Under the recalibrated target it lands at 1.61 (Phase 1 final run), so the 2 to 4 MP scoring cap from the grid plan is not needed.
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
