---
date: 2026-10-20
status: proposed
---
# Accept literal note bodies alongside JSON

## Status

Proposed.
The local implementation follows this decision.

## Context

Direct ticket capture and recent-note discovery reduce the effort needed to preserve and recover work.
Appending the next action still requires preparing JSON even when one sentence suffices.
Existing agents already depend on note requests from standard input and files, including their retry behavior.

## Decision

Accept an optional literal body after the ticket ID in `note`.
When present, construct the existing `NoteInput` from that body and the optional `--request-id` flag and leave standard input unused.
When absent, keep the existing JSON input path and its default standard input.
Reject a literal body combined with an explicit `--input`, including `-`, and require a literal body for the request-ID flag.
Pass both forms through the same `Store::note` operation, text validation, sequence allocation, and atomic replay record.
Keep schema discovery tied to the canonical `NoteInput` and saved `Note` response.
The [agent guide](../agents.md#capture-and-resume) defines quoting, input selection, and retry usage.

A second write command would add a name without another domain operation.
Treating JSON-looking bodies as structured input would make literal content ambiguous.
Accepting both a body and a file would require choosing which explicit input to discard.
A separate mutation or request identity would make retries depend on the input form.

## Consequences

Users can record a next action directly and recover it through `notes` in another session.
Existing JSON invocations keep their syntax and request schema.
Retries can switch between JSON and literal arguments while preserving the same saved note.
The [verification contract](../verification.md#persistence-and-process-boundaries) covers source selection at the native parser and storage boundary.
