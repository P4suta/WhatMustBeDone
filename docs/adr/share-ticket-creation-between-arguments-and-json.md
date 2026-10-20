---
date: 2026-10-20
status: proposed
---
# Share ticket creation between arguments and JSON

## Status

Proposed.
The local implementation follows this decision.

## Context

Capturing a discovery must take little effort before attention returns to another repository.
JSON creation supports agents and structured requests but requires preparing a document for a title alone.
A second creation path must preserve the existing defaults, relation checks, transaction, and retry identity.

## Decision

Expose `capture TITLE` with optional body, repeated repositories, priority, parent, repeated dependencies, and request ID arguments.
Derive argument parsing on the existing `AddInput` request type and send that value directly to `Store::add`.
Keep JSON `add` available and use the same input, ticket response, and error schemas for discovery.
Require explicit backlog initialization and leave standard input unused by capture.

Use the existing add operation for replay records across both commands.
Matching decoded fields replay the original response, while changed fields conflict.
Preserve array order and duplicates in request identity before the existing stored-ticket normalization.
The [agent guide](../agents.md#capture-and-resume) specifies defaults and argument usage.
The [verification contract](../verification.md#persistence-and-process-boundaries) identifies the native parsing and persistence checks and the unchanged proved core.

A separate capture request or storage operation would duplicate defaults and split retry identities.
Constructing JSON text from arguments would add an unnecessary encoding boundary.
Automatic initialization would make a mistyped database path create a separate backlog.
Implicit repository inference would make requests depend on the working directory.

## Consequences

A title alone is enough to preserve a discovery in an initialized backlog.
Agents can switch between JSON and arguments without changing creation semantics or replay records.
The public CLI gains a command while the on-disk format and dependency set remain unchanged.
Argument spelling and shell quoting require native boundary checks alongside the existing JSON checks.
Creation defaults on the shared request type must remain consistent between Clap and Serde.
