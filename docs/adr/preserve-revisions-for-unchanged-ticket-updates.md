---
date: 2026-10-20
status: proposed
---
# Preserve revisions for unchanged ticket updates

## Status

Proposed.
The local implementation follows this decision.

## Context

Agents can save an existing ticket without changing its content.
Advancing its revision creates conflicts for another agent whose snapshot still describes the current content.
Repository and dependency arrays can also differ in order or duplication while identifying the same relations.
Updates must still reject stale snapshots and preserve transaction validation.

## Decision

Check the expected revision inside the existing write transaction before applying the patch.
Apply supplied fields and normalize repository and dependency arrays before comparing the candidate with the stored ticket.
Use derived ticket equality so every stored field participates.
Return the current ticket without advancing its revision or rewriting its ticket row when the candidate is identical.
Advance the revision once for a real change and reject overflow.
An unchanged update at `u64::MAX` succeeds.
Stale updates still conflict even when their requested content equals the current content.

The pure core constructs a matched revision and consumes it to select an unchanged or advancing result.
The storage adapter supplies candidate equality and keeps snapshot validation and the normal commit protocol.
The [verification contract](../verification.md) separates scalar proofs from native normalization and persistence checks.

Keeping a revision for every save attempt would preserve write-count semantics but cause avoidable conflicts.
Comparing before the revision check would accept stale requests that require rereading current context.
Comparing raw input would treat reordered relations as content changes.

## Consequences

Repeated saves of current content preserve the revision another agent has already read.
Actual changes preserve optimistic concurrency and atomic persistence.
Clients must treat revisions as content versions rather than successful update counts.
Unchanged updates still require the writer lock and a valid database snapshot.
This decision preserves app records and revisions and makes no byte-preservation promise for native storage.
