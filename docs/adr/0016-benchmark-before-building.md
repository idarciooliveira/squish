# ADR-0016: Benchmark against TinyPNG before building the product

- Status: Accepted
- Date: 2026-10-11

## Context

Several decisions rest on numbers we haven't measured yet: quantette's quality compared to imagequant (ADR-0004), mozjpeg compared to jpegli (ADR-0005), and the default quality values (ADR-0009). If the core can't get close to TinyPNG, a polished CLI and web UI don't matter.

## Decision

Phase 0 is a time-boxed spike of 2 to 3 days before any product code:

- Build a test set of about 50 images in `bench/`: photos, screenshots, logos with transparency, illustrations, and files that are already optimized.
- Run them through TinyPNG once and keep its output as the baseline.
- Run throwaway programs over the same files with each candidate library.
- Record bytes, encode time and SSIM per file.

Phase 1 starts only when Phase 0 answers these questions:

1. Which quantizer ships, and does it handle PNGs with transparency?
2. Which JPEG encoder ships?
3. What are the default quality values?

## Consequences

- A few days before any visible product.
- Decisions get made from data, and the test set becomes the regression suite for every later phase.
- `dssim` (our SSIM tool) is AGPL-3.0. It's used only in benchmark scripts and never linked into squish.

## Alternatives considered

- **Start with the CLI and tune later.** Risks building on a core we'd have to replace.
