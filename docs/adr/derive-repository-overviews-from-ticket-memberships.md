---
date: 2026-10-20
status: proposed
---
# Derive repository overviews from ticket memberships

## Status

Proposed.
The local implementation follows this decision.

## Context

Repository-specific ticket queries require remembering which repositories contain unfinished work.
Work can span many repositories, depend on work elsewhere, or have no repository yet.
A separate repository registry would require maintaining another set of records and deciding how stale entries behave.

## Decision

Derive `repos` groups from the current validated ticket snapshot without adding persisted repository records.
Count each ticket once per distinct exact repository identifier and place tickets without membership in a null group.
Keep historical status counts alongside readiness and blocking counts computed using global direct dependencies.
Filter active groups only after aggregation, retaining their closed counts, then paginate null-first lexical group order.
Provide `list --unassigned` and `search --unassigned` for inspecting the null group, mutually exclusive with an exact repository filter.
The [agent guide](../agents.md#find-work) defines the response and navigation contract.

Keep four-state classification and checked counter transitions in the dependency-free core.
Keep string grouping, deduplication, and snapshot joins in the adapter.
The [verification contract](../verification.md) distinguishes complete scalar proofs from native adapter checks.

A persisted registry could include empty repositories but would create another lifecycle to manage.
Filtering tickets to active states before aggregation would hide useful historical counts in active groups.
Restricting prerequisite checks to each repository would incorrectly report cross-repository work as ready.

## Consequences

Agents can discover repositories and unassigned work from the same local backlog without GitHub access.
Removing a group's final ticket membership removes the group.
Group totals can overlap, and counts don't estimate effort or project completion.
An in-progress ticket with completed prerequisites is active but neither ready nor blocked.
The on-disk schema and existing query snapshot protocol stay unchanged.
