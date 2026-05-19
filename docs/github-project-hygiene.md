# GitHub Project Hygiene

agent007 uses GitHub Projects as a lightweight planning mirror, not as the source of truth for runtime state.

## Goals

```text
GitHub Project hygiene
├─ keep milestone cards small and actionable
├─ link PRs/issues back to repo docs
├─ avoid stale cards after PR merge
└─ make automation safe to run repeatedly
```

## Recommended states

| State | Meaning | Exit action |
|---|---|---|
| Backlog | idea exists, not yet scoped | attach milestone/doc link |
| Ready | accepted next slice | create branch/PR |
| In progress | branch or PR exists | link PR |
| Review | PR checks/reviews pending | resolve comments/checks |
| Done | merged or explicitly closed | archive or move to Done |

## Hygiene rules

1. Every implementation PR should reference one milestone/card.
2. A merged PR should move the linked card to `Done` or be archived.
3. Closed/unmerged PRs should keep the card open only if work remains.
4. Large roadmap items should be split into small cards before coding.
5. Project automation must be idempotent: running it twice should not duplicate cards.

## Current milestone cards to track

```text
M4 cards
├─ repository/catalog skill import
├─ artifact versioning on save/import/edit
├─ runtime session messages
├─ memory lifecycle UX
├─ provider onboarding UX
├─ TUI operator controls
└─ .a7bundle v2 container
```

## Future automation contract

A future `agent007 project sync` command should:

```text
project sync
├─ read local milestone docs
├─ query GitHub Project items
├─ create missing cards only when explicitly requested
├─ update status for merged PR links
├─ never delete cards automatically
└─ print a dry-run diff by default
```

