---
date: 2026-10-21
status: proposed
---
# Run complete verification on a hosted macOS runner

## Status

Proposed.
The project prepared the workflow locally.
Hosted execution remains unverified.

## Context

Persistence and process checks need the supported native filesystem and process semantics.
The initial app target is Apple Silicon macOS.
Required verification must be reproducible without an owner's private tools, caches, or machines.
Local development feedback alone doesn't establish execution in a shared contributor environment.

## Decision

Document the standard Apple Silicon `macos-26` hosted runner as the complete verification environment.
Use a manual workflow that invokes the authoritative `just check` implementation.
Pin actions by full commit, installation versions, tool archive checksums, and advisory data.
Verify archives before extraction or execution and fetch locked dependencies before offline native checks.
Require proof reports and counterexamples to identify the configured `aarch64-apple-darwin` target.
Include the workflow and archive manifest in source-bound evidence.
Keep local native and security results distinct from hosted execution and current formal results.

The hosted runner label receives platform updates and isn't an immutable machine image.
The [verification contract](../verification.md#hosted-verification) states that boundary.
Owner-machine execution can't replace the public environment rule.
A Linux proof result alone would not exercise the supported macOS persistence boundary.
Adding automatic or scheduled complete runs would spend hosted capacity independently of affected contracts.

## Consequences

Contributors have a public setup for the supported target using the project's own commands.
Preparing or linting the workflow supplies no hosted success result.
Publication and an authorized hosted execution are still required before treating the current change as verified.
Tool bootstrap and platform compatibility remain subject to the first actual hosted run.
Further supported targets require their own native evidence and explicit proof-target policy.
