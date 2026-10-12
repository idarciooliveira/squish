# ADR-0027: Keep PNG color chunks, and try lossless when 256 colors barely helps

- Status: Accepted
- Date: 2026-10-12
- Amends: ADR-0010 (what counts as the color profile in a PNG) and PRD 4.1 step 4 (what happens when 256 colors misses the target)

## Context

A review of the Phase 1 core found two PNG problems.

**Color chunks.** ADR-0010 keeps "the ICC profile". In a PNG that is `iCCP`, and the code kept only that chunk under `strip-keep-icc`. But a PNG can describe its color space without an ICC profile, through `gAMA` and `cHRM`, `sRGB`, or `cICP` for HDR, which `mDCV` and `cLLI` complete. Those were dropped by both the lossy and the lossless path, so a file tagged with `gAMA` only could shift in browsers that honor it. On `bench/`, 21 of 41 PNGs carry `gAMA`, 11 of them with `sRGB` too. Keeping them costs 16 or 29 bytes per file.

**Dithering against the image.** When 256 colors misses the target, PRD 4.1 ships the 256-color file. The size guard only compares against the input, so it can't see that a lossless optimize of the same input would be smaller. A 300x200 smooth gradient shows the case. 256 colors with dithering gives 26,028 bytes at fs2 34, and oxipng alone gives 1,310 bytes with every pixel exact.

Always running the lossless optimize when the target is missed fixes that, but it's expensive. On `bench/`, lossless never won on any of the 25 misses. The median search cost rose from 1.8x to 2.8x one plain encode, and `image (1).png` went from 3.0 s to 11.0 s of CPU, over the 10 s limit in NFR-2.

On `bench/`, every miss lands at 20% to 49% of the input's size. On the gradient it was 90%. When quantizing barely shrinks the file, the dithering is fighting the image.

## Decision

1. Under `strip-keep-icc`, a PNG keeps `iCCP`, `sRGB`, `gAMA`, `cHRM`, `cICP`, `mDCV` and `cLLI`, in both the lossy and the lossless path. When `iCCP` and `sRGB` are both present, `sRGB` goes: the spec says a file shouldn't carry both, and PNG 3rd edition ranks `iCCP` higher. `strip-all` drops them all, and `keep-all` is unchanged.
2. The kept chunks go into the indexed PNG before oxipng runs, with the same keep set as the lossless path. oxipng then won't reduce to grayscale under an RGB profile, and both paths keep the same chunks. Leaving `sRGB` out of the keep set when `iCCP` is present also stops oxipng from dropping or rewriting the profile.
3. A PNG whose `cICP` names PQ or HLG only gets the lossless steps. An 8-bit palette can't hold HDR levels.
4. When the quantized result misses the target, or wasn't scored (`--effort fast`, `--quality N`), and it's more than half the size of either the input or a quick lossless encode, also run the lossless steps on the original, and ship them if they're no bigger. The quick encode is the `png` crate at its balanced level, so a poorly compressed input can't hide the case. The report then has no setting and a score of 100, since every pixel survives, and keeps the target and the probe count.
5. The lossless pass runs after the quantizer's buffers are freed, so it doesn't add to peak memory.

## Consequences

- PNG colors stay correct for files without an ICC profile, and the profile itself comes through byte for byte.
- The gradient case gets its exact pixels at a fraction of the size, at any effort or fixed quality, and however the input was compressed.
- On `bench/`, the default output is byte for byte unchanged, and so is the median search CPU (505 ms before, 508 ms after). The quick encode costs at most 0.45 s per file there. Every miss lands at 20% to 49% of both the input and the quick encode.
- The fixed-quality baseline in the bench harness now tries the lossless pass on the 10 small sprites, which doubles their baseline time. The NFR-2 ratio falls from 1.8x to 1.3x because of that, not because the search got cheaper. The 10 s limit is the number to watch.
- The 50% line is still a heuristic. Revisit if a user reports a file that dithers badly but still lands under it.

## Alternatives considered

- **Always try lossless on a miss.** Correct, but it fails NFR-2 on `bench/` and never wins there.
- **Gate on the fs2 score.** Real photos on `bench/` miss with scores as low as 56 and still lose to lossy, so no score threshold separates the cases.
- **Leave it.** The gradient case is rare, but when it happens squish returns a file that is both worse and 20 times bigger than it needs to be.
