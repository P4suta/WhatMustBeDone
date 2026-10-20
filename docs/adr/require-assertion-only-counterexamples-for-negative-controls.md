---
date: 2026-10-21
status: proposed
---
# Require assertion-only counterexamples for negative controls

## Status

Proposed.
The local implementation follows this decision.
Current formal execution remains required.

## Context

A failed verifier process can mean an assertion counterexample or an incomplete analysis.
Kani 0.68 can preserve an assertion failure alongside a solver error.
Ignored quantifiers can also produce spurious assertions while Kani classifies the analysis as an error.
The [verification implementation](https://github.com/model-checking/kani/blob/kani-0.68.0/kani-driver/src/call_cbmc.rs) distinguishes those cases.
Looking only for one failed assertion can mistake invalid analysis for successful detection of a broken implementation.

## Decision

Require a completed single-harness report, the expected identity and target, exact failure counts, and a failed process.
Validate the attributed [error details](https://github.com/model-checking/kani/blob/kani-0.68.0/kani-driver/src/frontend/schema_utils.rs) as an ordinary assertion-only failure with measured properties.
Reject solver errors, unsupported semantics, undetermined checks, non-assertion failures, incomplete work, and unknown report values.
Keep the acceptance decision in the production `CounterexampleEvidence` core type and use an exhaustive check-status and category classification.
Require source-bound proofs of those decisions and a production mutation that bypasses required error classification.

A mutation can make an original cover unsatisfiable or unreachable without invalidating its assertion counterexample.
Accept those cover outcomes only in negative controls.
Successful production proofs still require satisfied covers.
The [verification contract](../verification.md#evidence-and-negative-controls) states the input domain and native report fixtures.

Accepting any nonzero exit confuses tool failure with a counterexample.
Accepting any failed assertion ignores analysis errors retained in the same report.
Comparing a single-harness report with the complete proof inventory establishes only that the inventories differ.

## Consequences

Negative controls distinguish assertion-only detection from missing or unsound evidence.
When the pinned Kani report schema changes, maintainers must update the adapter.
The original successful-proof requirements remain intact.
Run new decision proofs and the corresponding negative control in the documented verification environment before verification can confirm the change.
