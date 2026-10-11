# ADR-0001: Record architecture decisions

- Status: Accepted
- Date: 2026-10-11

## Context

squish starts as a plan with many choices made up front: language, codecs, license, how the web version works. Six months from now, nobody will remember why we picked mozjpeg over jpegli or why the server doesn't store files. Without that history, people reopen settled questions or undo a choice without knowing what it protected.

## Decision

Every decision that shapes the code, the license or the workflow gets an ADR in `docs/adr/`, numbered in order, using this format: status, date, context, decision, consequences, alternatives considered.

ADRs are never edited to change their meaning. To change a decision, write a new ADR and set the old one to `Superseded by ADR-XXXX`. Fixing typos is fine.

The index lives in [`docs/adr/README.md`](README.md). The PRD links to it.

## Consequences

- A few minutes of writing per decision.
- New decisions have to say which old ones they replace, which forces us to read them.

## Alternatives considered

- **Decisions only in the PRD.** The PRD describes the current plan and gets rewritten. It loses the history, which is the point.
- **Decisions in commit messages.** Our commit messages are one line (ADR-0015), so there's no room.
