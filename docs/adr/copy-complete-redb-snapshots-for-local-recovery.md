---
date: 2026-10-20
status: proposed
---
# Copy complete redb snapshots for local recovery

## Status

Proposed.
The local implementation follows this decision.

## Context

The backlog contains long-lived work context across repositories.
A recoverable checkpoint must preserve both the visible work and the sequence and replay metadata that prevent duplicate captures.
Restoring a checkpoint must preserve any existing backlog at the chosen destination.

## Decision

Use the existing redb file format for copies, as established by [the storage decision](store-the-local-backlog-in-redb.md).
Read all four typed tables from one read-only transaction and write them into one new database transaction.
Share this operation between `backup DESTINATION` and `--db NEW_DESTINATION restore SOURCE`.
Preserve stored JSON and request replay responses without normalizing or regenerating them.
Reject incompatible source schemas and layouts through the normal snapshot boundary.

Prepare the database in a private temporary file on the destination filesystem.
Validate the closed copy through a read-only reopen, synchronize it, and publish without replacing an existing name.
Synchronize new directory links and the final destination directory before returning success.
Bind the prepared file to its destination through a private owned type.
See [the verification contract](../verification.md#archive-publication) for the assumptions and native checks.

A ticket-only export would lose sequence and replay identities.
A new JSON archive format would add another compatibility contract without improving local recovery.
Copying the file while redb has it open for writing would include the recovery state and require assumptions about concurrent writes.
Reading through the transaction API avoids those assumptions and ties the copy to one committed snapshot.
Replacing a live backlog would require a separate destructive operation and recovery protocol.
Create-only recovery lets the owner inspect a restored copy before choosing it for active work.

## Consequences

Backups preserve the app's retry and revision behavior and need no external account or service.
Source files can remain read-only, and existing destinations keep their data.
The copy uses the same app schema and redb format as its source.
It provides a checkpoint rather than synchronization or an automatic format upgrade.
Copying reads and writes every stored row and holds the source snapshot until publication finishes.
A process exit can leave a private staging file, but the requested destination appears only after preparation succeeds.
An error after publication can leave a complete destination whose directory synchronization didn't finish.
