# ADR-0028: Lossy WebP uses a fixed q71, and the per-file search waits for a bigger corpus

- Status: Accepted. The codec PR's harness run confirms q71 on real encodes. The search in point 4 of Context is the candidate to re-test when the corpus grows.
- Date: 2026-10-13
- Amends: ADR-0024 for WebP, which made a per-file search the default for every format. ADR-0010 for WebP metadata. PRD sections 4.3 and 4.4, which said fixed quality ~75.

## Context

PRD section 4.3 gives lossy WebP libwebp quality ~75 at method 4. ADR-0024 made a per-file search the default for every format and said WebP gets "the same treatment" in Phase 2. This ADR chooses between the two and sets the metadata policy.

**Corpus.** 16 WebP files are in `bench/images/`, 13 of them unique: `sample1.webp` is there four times, so three copies have no TinyPNG output and every tool skips them. 12 are lossy, 1 is lossless, and none is lossy with alpha. They run from 0.1 to 10.5 MP. Two are 138 and 420 bytes, and TinyPNG returned both untouched, so they only exercise the size guard. Eleven files carry the calibration. Read every number below as a first measurement, not a calibration.

`bench/spike/src/bin/curves.rs` encoded each lossy file with libwebp at method 4, from q25 to q95 in steps of 5, and scored every result with dssim and with fs2 at ~1 MP. A second run covered q66 to q76 in steps of 1 (`CURVES_WEBP_GRID=66:76:1`). `bench/scripts/analyse-curves.py --guard` turns them into `bench/results/curves-webp-analysis.txt` and `curves-webp-fine-analysis.txt`, and models the size guard of ADR-0011: a candidate that isn't smaller than the input counts as the input. Sections 2b and 2c of those files hold the tables below.

What the curves show:

1. **TinyPNG runs libwebp at a fixed q70.** On all 11 lossy files big enough to tell, its output is within 2 bytes of our q70 encode. JPEG was the opposite, with a quality that moved per file. So TinyPNG's WebP outputs spread from fs2 69 to 90 (median 75.5), because one quality means different things on different images.
2. **The cheapest fixed quality that passes NFR-1 is q71.**

   | fixed quality | bytes vs TinyPNG | median dssim ratio | worst file |
   |---|---|---|---|
   | q65 | −31.2% | 1.20 | 1.38x |
   | q69 | −9.5% | 1.04 | 1.21x |
   | q70 | +0.0% | 1.0035 | 1.13x |
   | q71 | +1.3% | 0.95 | 1.11x |
   | q72 | +7.6% | 0.92 | 1.05x |
   | q75 | +12.3% | 0.84 | 1.01x |

   q70 is TinyPNG's own setting, and its median ratio lands just above 1.0, so it fails clause (b) of ADR-0025 by 0.35%. Our libwebp and TinyPNG's give the same sizes but not the same bytes. q72 and up break clause (a). q71 passes with margin, and it holds when any one file is left out: bytes +0.6% to +2.4%, median at most 0.96, worst at most 1.11x.
3. **A search over the whole quality range fails.** The worst file is always `sample1.webp`, a 10 MP photo that scores fs2 90 at TinyPNG's q70. Its ~1 MP copy is smooth enough that a low quality still passes a moderate target, and at full size dssim is 4.5x TinyPNG's. That is the risk ADR-0026 and PRD section 11 named for large photos, now showing up on WebP.
4. **A search with a floor at q60 passes, but on one file.** With the search held to q60 and above, targets 78 and 80 pass all three clauses.

   | range | fs2 target | bytes vs TinyPNG | median ratio | worst file | kept as original, of 12 | NFR-1 |
   |---|---|---|---|---|---|---|
   | q25 to q95 | 78 | −52.3% | 0.53 | 4.48x | 3 | fails (c) |
   | q25 to q95 | 84 | +8.4% | 0.00 | 2.79x | 7 | fails (a), (c) |
   | q25 to q95 | 88 | +25.6% | 0.00 | 1.76x | 8 | fails (a) |
   | q60 to q95 | 75 | −38.7% | 1.02 | 1.52x | 2 | fails (b) |
   | q60 to q95 | 78 | −30.9% | 0.53 | 1.52x | 4 | passes |
   | q60 to q95 | 80 | −30.8% | 0.49 | 1.52x | 5 | passes |
   | q60 to q95 | 82 | +5.3% | 0.00 | 1.52x | 8 | fails (a) by 0.3 |

   A median near 0 means more than half the files were kept as originals, which count as dssim 0. Floors of q55 and q65 pass at 78 and 80 too, with worst files of 1.76x and 1.28x.

   The saving comes from `sample-photo-3840x2160.webp`, which is 259 KB at q60 against TinyPNG's 917 KB. Without it, q60+ at target 78 comes to +10.4% bytes on the other 11 files and fails (a). Between targets 80 and 82 the total moves 36 points, because two large photos sit on a steep part of their curves.
5. **The search costs more than NFR-2 allows.** On the grid, one encode costs a median 8 ms of CPU (590 ms at the 10 MP p95), and four probes with fs2 cost 76 ms, about 9x one encode against NFR-2's limit of 4x. No file is near the 10 s limit: the slowest is 3.9 s at four probes.

fs2 rises 0.35 to 0.6 points per quality step between q60 and q90 (median per file).

## Decision

1. **Lossy WebP: fixed libwebp quality 71, no search, at every effort.** Method 2 at `fast`, 4 at default and 6 at max (PRD section 4.4). Methods 2 and 6 are unmeasured. NFR-1 is judged at the default.
2. **No per-file search for WebP yet (amends ADR-0024).** The one search that passes, point 4, gets its saving from a single 4K photo and fails NFR-1(a) without it. It also misses NFR-2. A fixed q71 passes NFR-1 on every leave-one-out subset and costs one encode.
3. **`--quality N` stays a fixed setting**, as for the other formats.
4. **Lossless sources stay lossless** (PRD section 4.3): libwebp lossless at method 4, 2 for fast and 6 for max. `--quality N` doesn't turn a lossless file lossy. This is unmeasured, because the corpus's one lossless file is 138 bytes. Near-lossless isn't evaluated and needs a sweep on real lossless files. For the same reason, lossy files with alpha keep libwebp's defaults (`alpha_quality` 100, `exact` off, so RGB under fully transparent pixels may change) until the corpus has one.
5. **Metadata (amends ADR-0010).** WebP is a RIFF container, so the policy runs on chunks without touching pixel data, as `metadata.rs` does for JPEG segments and PNG chunks.
   - `strip-keep-icc`, the default: keep `ICCP`. Keep the orientation by writing a minimal `EXIF` chunk (`metadata::minimal_exif`). Drop every other `EXIF` field, `XMP ` and unknown chunks.
   - `strip-all`: drop `ICCP` and `EXIF` as well.
   - `keep-all`: carry `ICCP`, `EXIF` and `XMP ` through unchanged.
   - The `webp` crate encodes bitstreams only, so the container is assembled in `squish-core`. A file that needs nothing extra is a plain `VP8 ` or `VP8L` file. Otherwise it gets a `VP8X` header with the flags and canvas size set, `ICCP` first, `ALPH` before `VP8 `, and `EXIF` last.
6. **Size guard** as in ADR-0011, on whole files.
7. **Harness.** `bench/harness` takes `--only webp`, `--webp-target` and reports lossless sources as `webp-lossless`. Its plain encode is q71, so its search-cost ratio is 1x for WebP until a search exists. `--webp-target` is there for re-testing the search.

## Consequences

- WebP's exit criterion, NFR-1, is judged by the harness once `webp.rs` lands. The grid says +1.3% bytes, median ratio 0.95 and worst file 1.11x on 12 files.
- squish's lossy WebP will be about the size of TinyPNG's, not smaller. The floor-q60 search would save ~31% on this corpus, and that saving is what's given up until a larger corpus shows it doesn't hang on one file.
- Quality varies across files as much as TinyPNG's does: fs2 69 to 90 at one setting.
- NFR-2 holds for WebP, since there is no search.
- Re-test the search when the corpus has more large lossy photos and lossy files with alpha. Adopt it in a new ADR if a target passes NFR-1 with every single file left out in turn, and passes NFR-2.
- PRD sections 4.3 and 4.4 change to match. `bench/results/curves-webp.csv`, `curves-webp-fine.csv` and their analysis files are the evidence.

## Alternatives considered

- **Fixed q75, the PRD's value.** +12.3% bytes, so it fails NFR-1(a).
- **Fixed q70, TinyPNG's setting.** Same bytes, but the median ratio is 1.0035, which fails (b). q71 costs 1.3% more and clears it.
- **A per-file search from q60 to q95 at fs2 ≥ 78.** −30.9% bytes on this corpus, +10.4% without its largest photo, and about 9x one encode. Point 4 of Context. Held back, not rejected.
- **Search the whole range with no floor.** Fails clause (c) at every target that stays inside the byte budget.
- **Near-lossless for lossless inputs.** Not measured.
