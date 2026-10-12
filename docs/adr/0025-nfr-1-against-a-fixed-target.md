# ADR-0025: NFR-1 judges the median and the worst file, not every file

- Status: Accepted
- Date: 2026-10-12
- Supersedes: the per-file quality rule in NFR-1, as written on 2026-10-12 and used in ADR-0022 and ADR-0024

## Context

NFR-1 said: total bytes within ~5% of TinyPNG's on `bench/`, and each file's dssim at or below TinyPNG's for that file, with differences under 0.0002 counting as equal.

The Phase 0 sweep met that rule for JPEG by giving every file its own target: TinyPNG's dssim for that file. squish can't do that at runtime, because it doesn't have TinyPNG's output. It has to search against one target per format.

TinyPNG's dssim on `bench/` spans 0.00032 to 0.00488 for JPEG, a 15x range. `bench/scripts/analyse-curves.py` over `bench/results/curves.csv` (20 JPEGs, 41 PNGs, `image (20).jpg` left out because at 66 MP it's over the 50 MP pixel limit) tested what one fixed target can do:

| JPEG, one target for every file | bytes vs TinyPNG | files at or below TinyPNG's dssim | worst file |
|---|---|---|---|
| dssim ≤ 0.0003, the loosest that passes every file | +146% | 20/20 | 0.91x |
| dssim ≤ 0.0025, about TinyPNG's median | +13% | 8/20 | 7.0x |
| fs2 at ~1 MP ≥ 89.3 (ADR-0026) | +3.5% | 17/20 | 1.94x |

No fixed target meets the per-file rule within the byte budget. That includes dssim, the metric the rule is written in. So the old rule mostly measured how far TinyPNG's per-file choices are from any consistent target, rather than squish's quality.

## Decision

1. NFR-1 is met for a format when, over that format's files in `bench/`, with default settings:
   - (a) total bytes are at most 5% above TinyPNG's,
   - (b) the median per-file dssim ratio is at most 1.0,
   - (c) no file's ratio is above 2.0.

   The ratio is our dssim divided by TinyPNG's for the same file, and TinyPNG's value is floored at 0.0002.
2. dssim stays the referee. It's AGPL, so it runs in `bench/` only and never ships (ADR-0024).
3. For PNG, until ADR-0022 is revisited, (c) doesn't count files already at 256 colors. Those misses come from quantizr's limit, and no target can fix them.
4. The benchmark report lists the per-file ratios, so outliers stay visible even when the rule passes.

## Consequences

- JPEG passes on the grid data at fs2 ≥ 89.3: +3.5% bytes, median ratio 0.79, worst 1.94x. The worst file, `image (7).jpg`, is 40 MP, and it's what sets the target. ADR-0026 lists the fix to try for it.
- PNG passes at fs2 ≥ 90: −2.1% bytes, median ratio 0.87, and every miss is at 256 colors. The worst is `2_webp_a.png` at 2.01x.
- 3 of 20 JPEGs come out lossier than TinyPNG made them, by up to 2x in dssim. In exchange, quality is consistent across files, and total bytes stay inside the 5% budget.
- The rule works from aggregates, so a new corpus could pass with a few bad files. Clause (c) bounds how bad any one file can be.

## Alternatives considered

- **Keep the per-file rule.** It costs +146% bytes on JPEG, which breaks the byte half of NFR-1.
- **A tighter per-file tolerance, such as every file within 1.5x.** The cheapest fs2 target that does it, 89.9, costs +12% bytes. A tighter band costs more.
- **Judge with the shipped metric instead of dssim.** That's circular. The referee has to be independent of the thing being tuned.
- **Estimate TinyPNG's per-file choice at runtime.** TinyPNG's outputs sit in a narrow band on fs2 (80 to 90.5 on JPEG). A fixed fs2 target already captures most of what it does, so there's little left for a predictor to win.
