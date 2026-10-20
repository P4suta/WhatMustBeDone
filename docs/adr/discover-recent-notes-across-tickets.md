---
date: 2026-10-20
status: proposed
---
# Discover recent notes across tickets

## Status

Proposed.
The local implementation follows this decision.

## Context

Progress and discoveries already persist as append-only ticket notes.
Reading a ticket's history requires its ID, and searching requires remembering some saved text.
After switching repositories or sessions, users can forget either clue.
A recovery view must expose the saved next action and identify the current work it belongs to.

## Decision

Add `notes` to read all saved notes in descending note ID order with ordinary limit and offset pagination.
Return the complete note and its ticket's current summary in each item.
Include every repository and ticket status without requiring a query or prior ticket ID.
Count notes rather than distinct tickets, and keep the original position when replaying a note request.
Describe append order through existing note IDs rather than adding timestamps or a separate activity log.
The [agent guide](../agents.md#capture-and-resume) specifies navigation and the page contract.

Reuse the [shared snapshot query protocol](read-healthy-backlogs-through-shared-snapshots.md) and its complete record validation.
Reverse the stored note-key order, paginate borrowed records, and hydrate the requested entries with current ticket summaries.
Keep ticket history and literal search available for deeper investigation.
The [verification contract](../verification.md#persistence-and-process-boundaries) records the native storage-order and join checks and their limits.

Returning only ticket summaries would hide the discovery that prompted the query.
Restricting the view to active tickets would hide useful findings attached to completed work.
Duplicating notes into an activity table would create another persistence and synchronization obligation.
Inferring time from IDs would claim information the backlog doesn't store.

## Consequences

Users and agents can recover recent next actions without remembering a ticket ID or search phrase.
Current ticket summaries give each saved note a route back to the full working context.
Notes from one ticket can share a page, and the view describes saved notes rather than all backlog activity.
Pagination narrows body cloning and summary hydration after full snapshot validation.
The storage format, dependencies, clock assumptions, and existing query responses remain unchanged.
