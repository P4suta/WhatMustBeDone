---
date: 2026-10-20
status: proposed
---
# Derive goal overviews from top-level ticket trees

## Status

Proposed.
The local implementation follows this decision.

## Context

Repository inventories don't show how unfinished work contributes to a larger purpose.
Top-level tickets already capture goals, and parent links decompose work across repositories.
A completed root can still contain unfinished descendants because completion is explicit.
Inspecting only active roots hides that work.

## Decision

Derive `goals` groups from each parent-null ticket and its complete subtree, including the root itself.
Treat standalone tickets as one-ticket goals rather than requiring another classification or registry.
Compute work counts using the same checked core transition as repository overviews and direct prerequisites from the complete snapshot.
Keep goal identity and status in a root summary and the combined counts in a nested `work` object.
Filter whole groups for `--active` after counting, then order by root priority and ID and paginate.
The [agent guide](../agents.md#find-work) defines the response and navigation contract.

Share the parent traversal between root-inclusive subtrees and strict descendants.
An immutable checked view validates the parent links once and supports repeated root queries with reusable scratch storage.
The app's existing snapshot validation supplies acyclicity.
The [verification contract](../verification.md) states the selector's finite proof domain and native adapter checks.

Explicit goal records would require maintaining a second classification and deciding where ordinary root work belongs.
Grouping only roots with children would hide standalone work.
Filtering roots by their own status would hide active descendants beneath closed roots.
Inferring root completion from descendants would change the existing explicit status contract.

## Consequences

Agents can discover large goals without remembering their IDs and then inspect their ready descendants.
Every ticket belongs to exactly one top-level tree in a valid backlog.
Moving or clearing a parent changes the derived grouping immediately.
Counts include historical work but don't estimate effort or completion percentages.
The on-disk schema and shared query protocol stay unchanged.
