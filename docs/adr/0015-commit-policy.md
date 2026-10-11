# ADR-0015: One-line commits, owner as the only author, no AI attribution

- Status: Accepted
- Date: 2026-10-11

## Context

AI coding tools help write parts of this project, and some of them add attribution to commits by default, such as `Co-authored-by` trailers or "Generated with" lines. The owner wants the history to show only their name, with short one-line messages.

## Decision

Every commit must:

1. Have a message of exactly one line.
2. Not mention Claude or Anthropic, and not contain `Co-authored-by`, "Generated with" or `noreply@` addresses.
3. Have `idarciooliveira <idarciooliveira@gmail.com>` as both author and committer.

Two git hooks in `.githooks/` enforce this:

- `commit-msg` rejects commits that break the rules. It strips comment lines and writes back the single line it checked, so what gets committed is exactly what was validated.
- `pre-push` checks every commit about to be pushed again. That catches commits made with `--no-verify`, by a rebase, or on a machine without the hooks.

Turn them on once per clone:

```sh
git config core.hooksPath .githooks
```

## Consequences

- A clean history with one author.
- Multi-line messages aren't possible, so the reasoning behind changes lives in ADRs and PRs, not in commit bodies.
- Hooks are local. Someone who never runs the `core.hooksPath` command, or who pushes with `--no-verify`, can bypass both. A CI check can close that gap later.
- A one-line message that starts with `#` gets stripped as a comment and rejected as empty.

## Alternatives considered

- **A server-side check only (CI).** Catches problems after the push, when they're already public.
- **Rewrite messages silently instead of rejecting them.** Hides problems instead of surfacing them.
