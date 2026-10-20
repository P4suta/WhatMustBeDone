---
date: 2026-10-20
status: proposed
---
# Scope bottleneck counts to goal consumers

## Status

Proposed.
The local implementation follows this decision.

## Context

[Global bottleneck ranking](rank-shared-prerequisites-by-current-affected-work.md) reveals shared foundations across the backlog.
Choosing work for one large goal also requires seeing which external prerequisites affect that purpose.
Candidate repository and descendant filters exclude foundations outside the selected scope without expressing which dependent work matters.

## Decision

Add an optional `--for-goal ID` consumer scope to bottleneck discovery.
The scope includes the specified ticket and all tickets beneath it in the parent hierarchy.
Compute each candidate's impact across the complete dependency graph before counting the affected consumers inside this scope.
Keep candidate repository, unassigned, strict descendant, and readiness filters independent.
Remove zero scoped counts and order the remaining candidates using their scoped counts and the existing rank key.
Resolve the requested scope before candidate filtering, returning `not_found` for a missing ticket even when no candidate matches.
Omitting the flag preserves the existing global query contract.
The [agent guide](../agents.md#find-work) defines the two scope directions.

Use the existing root-inclusive parent selector and impact traversal.
Keep the final selection-and-membership decision in the pure core and the ID-set join in the adapter.
The [verification contract](../verification.md) states the scalar proof and native composition checks.

Filtering candidates to the goal would hide external prerequisites.
Restricting dependency traversal to the goal would lose paths through outside intermediate work.
Counting only direct dependencies would miss shared transitive foundations.
Requiring a parent-null ticket would prevent focusing on a nested subgoal or a single blocked task.

## Consequences

Agents can compare external prerequisites for a chosen purpose while preserving global dependency semantics.
Scoped counts can change the ranking and exclude otherwise influential candidates.
Parent movement changes consumer membership on the next snapshot query.
Completion remains explicit, and scoped counts preserve the existing closed dependency boundaries.
The command's response shape and on-disk schema stay unchanged.
