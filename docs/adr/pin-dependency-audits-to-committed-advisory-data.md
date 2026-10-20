---
date: 2026-10-20
status: proposed
---
# Pin dependency audits to committed advisory data

## Status

Proposed.
The local implementation follows this decision.

## Context

A dependency verdict must be reproducible from the same project inputs.
An implicitly selected owner cache can contain different advisory revisions, untracked drafts, or filtering settings.
Refreshing that cache during verification changes the input and confuses reproducibility with advisory freshness.

## Decision

Pin a public RustSec commit in `verification.json`.
Read it from an explicitly provisioned repository and export its raw committed advisory blobs into an owned temporary directory.
Validate object identities, file modes, normalized paths, sizes, and complete responses before auditing.
Bind the audit to that snapshot's content digest and reject mutation during the check.
Prefix paths and contents with their byte lengths in both source and snapshot hashes so binary data can't imitate file boundaries.
Never fetch objects or consult the source working tree during verification.

Run the pinned cargo-audit version without advisory fetching or registry-index yanked checks.
Reject incomplete reports, empty work, findings, ignored advisories, and narrowed effective settings.
Use the production core's `AuditCoverage` contract for the final acceptance decision and require its proof and negative control.
Keep advisory freshness unevaluated and advance the public pin through a separate reviewed change.
The [verification contract](../verification.md#security-inputs) defines the command and external trust boundary.

Copying the cache's working tree admits local drafts and filters into a contributor verdict.
Git archives can apply export attributes rather than preserve raw committed bytes.
Fetching the current database inside the check prevents identical inputs from producing the same verdict.
Treating a successful process exit as enough coverage can accept ignored or filtered reports.

## Consequences

Contributors with the same committed database object can reproduce the advisory input without network access.
The project adds no runtime dependency or database service.
An absent pinned object fails the check and requires explicit provisioning.
Snapshot export and effective-report validation add native adapter code and fixtures.
Current-advisory freshness and mutable registry yanked status remain separate from this fixed-input dependency verdict.
