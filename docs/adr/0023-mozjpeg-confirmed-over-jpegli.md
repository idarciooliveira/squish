# ADR-0023: mozjpeg confirmed over jpegli

- Status: Accepted
- Date: 2026-10-12
- Confirms: ADR-0005

## Context

ADR-0005 chose mozjpeg and asked Phase 0 to try jpegli if its bindings built. The `jpegli` crate (0.1.0, BSD-3-Clause, wraps libjxl's jpegli 0.10.2) built in the spike in 33 seconds with no extra setup.

Both encoders went through the same quality-matched sweep over the 21 JPEGs in `bench/`. For each file, the sweep found the lowest quality setting whose dssim is at or below TinyPNG's, then compared bytes. Results are in `bench/results/sweep-jpeg.csv` and `bench/results/sweep-jpeg-jpegli420.csv`.

| encoder | met TinyPNG's quality | bytes vs TinyPNG | smaller than mozjpeg |
|---|---|---|---|
| mozjpeg, progressive, 4:2:0 | 21/21 | −4.4% | |
| jpegli, progressive, default chroma | 21/21 | +3.9% | 0/21 |
| jpegli, progressive, 4:2:0 | 21/21 | +1.4% | 2/21 |

## Decision

Keep mozjpeg as the only JPEG encoder. Don't add jpegli.

## Consequences

- JPEG meets NFR-1 with mozjpeg once quality is picked per file (ADR-0024).
- One C encoder to build, not two. jpegli would also have pulled in cmake and C++.
- The stale-crate risk in the PRD (mozjpeg's last release was Feb 2025) stays open. jpegli is the measured fallback, about 6% behind on bytes.

## Alternatives considered

- **jpegli.** Measured above. It's tuned for high quality, and at the qualities TinyPNG targets (dssim around 0.002) it loses.
- **Both, picked per file.** jpegli was smaller on only 2 of 21 files, which isn't worth a second encoder.
