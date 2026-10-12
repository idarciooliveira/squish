# Architecture decision records

Every decision that shapes squish's code, license or workflow is recorded here (see [ADR-0001](0001-record-architecture-decisions.md)). To change a decision, add a new ADR and mark the old one `Superseded by ADR-XXXX`. Don't rewrite history.

| ADR | Decision | Status | Date |
|-----|----------|--------|------|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted | 2026-10-11 |
| [0002](0002-rust-as-the-language.md) | Rust as the only language | Accepted | 2026-10-11 |
| [0003](0003-libraries-for-codecs-build-the-rest.md) | Use libraries for codecs, build everything around them | Accepted | 2026-10-11 |
| [0004](0004-permissive-license-and-png-quantizer.md) | Permissive license, quantette as the PNG quantizer | Quantizer superseded by ADR-0022 | 2026-10-11 |
| [0005](0005-mozjpeg-for-jpeg.md) | mozjpeg for JPEG encoding | Accepted | 2026-10-11 |
| [0006](0006-workspace-and-single-binary.md) | One Cargo workspace, one binary | Accepted | 2026-10-11 |
| [0007](0007-stateless-server-side-compression.md) | Compress on the server, keep the server stateless | Accepted | 2026-10-11 |
| [0008](0008-download-all-zip-in-browser.md) | "Download all" builds a STORE zip in the browser | Accepted | 2026-10-11 |
| [0009](0009-tinypng-style-defaults.md) | No knobs by default, settings for those who want them | Partly superseded by ADR-0024 | 2026-10-11 |
| [0010](0010-metadata-policy.md) | Strip metadata except the color profile and orientation | Accepted | 2026-10-11 |
| [0011](0011-never-return-a-larger-file.md) | Never return a larger file | Accepted | 2026-10-11 |
| [0012](0012-avif-deferred.md) | AVIF only in Phase 2, and only if needed | Accepted | 2026-10-11 |
| [0013](0013-vanilla-frontend-embedded.md) | Vanilla HTML and JS for the web UI, embedded in the binary | Accepted | 2026-10-11 |
| [0014](0014-local-first-deployment.md) | Local first, ready to harden for public use | Accepted | 2026-10-11 |
| [0015](0015-commit-policy.md) | One-line commits, owner as the only author, no AI attribution | Accepted | 2026-10-11 |
| [0016](0016-benchmark-before-building.md) | Benchmark against TinyPNG before building the product | Accepted | 2026-10-11 |
| [0017](0017-shared-options-and-errors.md) | One options grammar and one error list, owned by the core | Accepted | 2026-10-12 |
| [0018](0018-http-api.md) | HTTP API design | Accepted | 2026-10-12 |
| [0019](0019-cli-design.md) | CLI design | Accepted | 2026-10-12 |
| [0020](0020-repository-layout.md) | Repository layout | Accepted | 2026-10-12 |
| [0021](0021-tinify-compatible-api.md) | Optional Tinify-compatible API | Deferred | 2026-10-12 |
| [0022](0022-quantizr-as-the-png-quantizer.md) | quantizr as the PNG quantizer | Accepted | 2026-10-12 |
| [0023](0023-mozjpeg-confirmed-over-jpegli.md) | mozjpeg confirmed over jpegli | Accepted | 2026-10-12 |
| [0024](0024-per-file-quality-search-by-default.md) | Per-file quality search is the default | Accepted | 2026-10-12 |
| [0025](0025-nfr-1-against-a-fixed-target.md) | NFR-1 judges the median and the worst file, not every file | Accepted | 2026-10-12 |
| [0026](0026-fast-ssim2-as-the-quality-metric.md) | fast-ssim2 at about 1 MP as the quality metric | Accepted | 2026-10-12 |

## Template

```markdown
# ADR-NNNN: Title in sentence case

- Status: Proposed | Accepted | Superseded by ADR-XXXX
- Date: YYYY-MM-DD

## Context
What forces us to decide, and what we know.

## Decision
What we'll do.

## Consequences
What gets easier, what gets harder.

## Alternatives considered
What we rejected and why.
```
