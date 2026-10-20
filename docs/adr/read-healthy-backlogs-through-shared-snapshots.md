---
date: 2026-10-20
status: proposed
---
# Read healthy backlogs through shared snapshots

## Status

Proposed.
The local implementation follows this decision.
It partly supersedes the query-access choice in [the storage decision](store-the-local-backlog-in-redb.md).

## Context

Queries need committed work context without requiring permission to change a healthy database.
A writable handle for every command prevents access to protected files and excludes other readers.
Ordinary queries already recover databases after an interrupted writer.
Archive operations promise to keep their source unchanged.

## Decision

Open `list`, `show`, and `search` through redb's `ReadOnlyDatabase` first.
Use shared native read locks and validate the snapshot through the existing layout and record boundary.
Only the native `RepairAborted` result permits a writable open for automatic recovery.
Keep both open attempts inside the same bounded lock-retry loop.
Permission or format errors from the reader never trigger a writable open.
App validation errors also return directly.

Keep `backup` and `restore` strict read-only operations for their sources.
Return `recovery_required` and recommend an ordinary query on the writable source when it needs recovery.
Private database variants distinguish shared access from native recovery in the query adapter.
The adapter returns owned records and closes native handles before CLI output.

Requiring manual recovery for every ordinary query would add a step after interrupted writes.
Always opening a writer would preserve that behavior but continue to reject healthy read-only sources.
Trying a writer after any failure would escalate access for unrelated errors.
The exact native recovery condition preserves the existing recovery behavior while minimizing ordinary query access.

## Consequences

Clean query sources keep their bytes, support read-only files, and share access with other readers.
Readers exclude writers until the snapshot closes.
Recovery still needs write permission.
The native recovery condition establishes no app ownership.
Recovery can update an unclean foreign redb file before app validation rejects its layout.
Immutable archive operations refuse that recovery and leave their source unchanged.
The [verification contract](../verification.md#persistence-and-process-boundaries) records the native scenarios and trust boundaries.
