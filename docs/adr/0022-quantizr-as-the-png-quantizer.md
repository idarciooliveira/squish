# ADR-0022: quantizr as the PNG quantizer

- Status: Accepted. The per-file quality rule it measures against is superseded by ADR-0025.
- Date: 2026-10-12
- Supersedes: the quantizer choice in ADR-0004 (points 2, 5 and 6). The license decision in ADR-0004 stands.

## Context

ADR-0004 picked `quantette` and made alpha support a hard requirement. Phase 0 measured it on 41 PNGs (25 with transparency) against TinyPNG's output for the same files. Results are in `bench/results/spike.csv` and `bench/results/sweep*.csv`.

- **quantette 0.6.0** is RGB-only, so it can't take 25 of the 41 files. On the other 16 it always used 256 colors and came out 17.5% bigger than TinyPNG. 0.6.0 is still the latest release. It fails ADR-0004 point 5.
- **imagequant** is the best quantizer we measured. With the palette size searched per file until its dssim matches TinyPNG's, it met TinyPNG's quality on all 41 files at 6.2% fewer bytes. It's GPL-3.0-or-later. The commercial license costs $950 a year for a company of 1 to 9 people, and a perpetual license is available.
- **quantizr 1.4.3** is MIT, takes RGBA, and libvips 8.13+ supports it as an alternative to imagequant. At a fixed 256 colors it came out 0.6% smaller than TinyPNG in total bytes, 1.2% smaller on the alpha files. Searched per file, it matched TinyPNG's quality on 22 of 41 files. On the other 19 it can't get there even at 256 colors. Eight of those are within 10% of TinyPNG's dssim. The worst are flat alpha illustrations (`1.png`, `3.png`, `5.png`, `2_webp_a.png`), at 1.5x to 2x TinyPNG's dssim. Lower dithering trades quality for bytes and didn't fix any miss. It also handled the two files where imagequant returned `QualityTooLow`.
- **NeuQuant** (`color_quant`, MIT) made small files at visibly lower quality: median 1.6x TinyPNG's dssim, worst 9.5x. It has no dithering.
- **exoquant** (MIT) came out 26.5% bigger and was the slowest. It hasn't changed since 2016.
- **libimagequant 2.4.1** is the last BSD-2-Clause release. The GPL relicense is commit `d7c9eca` (2015-12-05), first shipped in 2.5.0. It's 2014-era C that we'd have to vendor and maintain. Not benchmarked.

The quality rule used throughout is NFR-1 as decided on 2026-10-12: per file, our dssim is at or below TinyPNG's, and anything under 0.0002 counts as equal.

## Decision

1. Ship `quantizr` as the PNG quantizer, behind the `Quantizer` trait from ADR-0004.
2. Accept that, for now, about half of PNGs come out at lower quality than TinyPNG at about the same size. PNG doesn't meet NFR-1 yet. JPEG does.
3. `imagequant` stays out of every shipped crate. It remains a bench-only reference.
4. Pick the palette size per file with a quality search, like the Phase 0 sweep. `dssim` is AGPL and can't ship, so the core needs a permissive metric. Candidates: `ssimulacra2` (BSD-2-Clause), `butteraugli` (BSD-3-Clause), `image-compare` (MIT). Choosing one is a Phase 1 task, and its scores need calibrating against dssim on `bench/`.
5. Revisit this when any of these happens:
   - a user-visible quality complaint on PNG output,
   - squish gets paying users, which makes the imagequant commercial license affordable,
   - a permissive quantizer, or our own, matches TinyPNG's quality on `bench/`.

## Consequences

- squish stays MIT OR Apache-2.0 with no paid dependency.
- PNG transparency works from day one, so the lossless-only fallback for alpha in ADR-0004 point 6 isn't needed.
- ADR-0020 names `quant/quantette.rs` and the `quant-quantette` feature. Those become `quant/quantizr.rs` and `quant-quantizr`.
- quantizr has no quality target of its own. The quality search lives in our code, which costs a few quantize passes per file.
- Swapping in imagequant (commercial) or our own quantizer later is one new file behind the trait.

## Alternatives considered

- **Buy the imagequant commercial license now.** Best results measured, but it's a yearly cost before squish has users.
- **Fork libimagequant 2.4.1.** Permissive and it has alpha, but it's ten-year-old C that we'd own entirely.
- **Write our own quantizer now.** The ADR-0004 fallback. quantizr already does alpha and dithering, so ours would have to beat it to be worth writing.
- **Call an installed `pngquant` binary.** Running a GPL program as a separate process is usually treated as aggregation, not linking. It adds a runtime dependency and needs legal review, so not now.
- **NeuQuant, exoquant, quantette.** Measured and rejected above.
