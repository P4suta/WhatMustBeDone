---
date: 2026-10-20
status: proposed
---
# Separate native checks from complete verification

## Status

Proposed.
The local implementation follows this decision.

## Context

Argument and persistence boundary checks need a short executable entry point on their actual platform.
The existing complete check also runs core proofs, negative controls, and security checks.
Running the native subset by hand duplicates the authoritative command sequence and makes its result easy to confuse with complete verification.
Elapsed-time proof limits also make an incomplete run depend on machine speed.

## Decision

Expose `native` through the Rust task runner and the thin just command surface.
Keep its formatting, strict Clippy, and workspace-test sequence shared with the complete `check` command.
Report native success only after every native command succeeds.
Keep all proof, negative-control, security, and source-bound receipt requirements in `check`.
Don't promote a native result or an earlier proof receipt into a complete verdict.

Use the production harnesses' fixed input domains and explicit unwinding to bound formal verification.
Remove elapsed-time verdict limits from proof and negative-control invocations.
Treat an external stop as inconclusive, so completion requires the missing evidence.
The [verification contract](../verification.md) specifies each result's scope.

Copying the native command list into shell recipes would create another implementation to maintain.
Skipping proof requirements after a native success would weaken the complete gate.
Changing a proof's logical domain after a timeout would establish a different claim.

## Consequences

Contributors have a maintained entry point for fast native feedback.
The complete check keeps its original required stages and rejects stale evidence.
Formal outcomes depend on the stated finite domains rather than the speed of a particular host.
Portable runner execution and immutable security inputs remain separate requirements for the complete verification environment.
