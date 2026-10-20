---
date: 2026-10-20
status: proposed
---
# Rank shared prerequisites by current affected work

## Status

Proposed.
The local implementation follows this decision.

## Context

An impact query explains a known prerequisite but requires remembering its ID.
Shared foundations can affect work across many repositories and goals.
Comparing them needs an observable measure that doesn't require estimates or another set of maintained records.

## Decision

Derive bottleneck candidates from non-done tickets with a nonempty existing impact selection.
Count each affected active consumer once, using the same transitive traversal and closed-branch boundaries as `impact`.
Include cancelled candidates when active work still relies on them.
Offer global readiness as an optional candidate restriction.
Apply repository and strict descendant filters only to candidates, keeping the counted dependency graph global.
Order by affected count descending, manual priority descending, and ticket ID ascending before pagination.
The [agent guide](../agents.md#find-work) defines candidate scope, navigation, and count semantics.

Keep the positive count and exact ordering in a private core key.
Reuse an immutable checked dependency view across candidate traversals while keeping the app's existing snapshot validation and storage protocol.
The [verification contract](../verification.md) separates complete scalar ordering and constructor input domains from finite graph proofs and native adapter checks.

Priority-first ordering would preserve ordinary list behavior but hide broad dependencies under narrower high-priority work.
Counting only direct consumers would miss prerequisites of shared intermediate work.
Filtering the traversal to a repository or goal would undercount cross-project effects.
Effort-weighted scores would require estimates the backlog doesn't contain.

## Consequences

Agents can find shared prerequisites before selecting an ID for a detailed impact query.
Candidate counts are transparent and reproducible from existing backlog state.
Counts can overlap across candidates, and completing a candidate can leave other blockers unresolved.
Users keep the decision about purpose, effort, and timing.
Repeated impact queries reuse validation while traversing overlapping subgraphs independently.
The on-disk schema remains unchanged.
