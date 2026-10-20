---
date: 2026-10-20
status: partly superseded
---
# Store the local backlog in redb

## Status

Partly superseded for query access by [shared read snapshots](read-healthy-backlogs-through-shared-snapshots.md).

## Context

Coding agents need durable work context shared across local repositories.
Short-lived CLI processes must preserve relationships and prevent lost updates and duplicate captures.
Initial use targets one Mac and requires no external service or account.

## Decision

Use redb 4.4.0 through its stable API and keep the app and storage engine in Rust.
Store versioned JSON ticket, note, and request records in typed tables.
Commit a mutation, its sequence allocation, and its replay record atomically with immediate durability.
Serialize independent CLI processes through the database lock with a bounded wait.
Check revisions inside the write transaction.
Keep graph decisions in the dependency-free, no-std core, with scratch storage supplied by the adapter.
Generate command schemas from the Serde request and response types using Schemars.

SQLite would provide mature SQL querying, but introduces a C storage implementation.
Turso provides Rust SQL.
Its cross-process write-ahead logging support remains experimental.
Fjall offers log-structured merge storage and background maintenance.
The backlog's small record set favors direct tables.
Native DB adds typed models and indexes over redb but has an unstable API.
Direct redb tables keep the required persistence contract small.

## Consequences

Close each database handle after the operation and before writing its response.
Independent processes execute serially through the standard redb lock.
Search scans the local records.
Add indexes only when measurements justify them.
App schema evolution remains an explicit responsibility even though the database file format is stable.
The storage boundary trusts redb and the local filesystem's documented durability behavior.
Native transaction and process tests cover that boundary.
Rust-core proofs don't prove filesystem durability.
