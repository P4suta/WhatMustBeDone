# Verification contract

Run `mise x -- just check` for formatting, strict Clippy, workspace tests, source-bound proofs, negative controls, secret scanning, and the pinned dependency advisory check.
Dependency checks read the RustSec commit fixed in `verification.json` from `target/verification/advisory-db`.
Use `--advisory-repo PATH` to select another repository containing that committed object.
The source repository remains read-only, and its working tree, untracked drafts, current branch, and replacement objects don't supply advisory contents.
Neither mode fetches advisories or evaluates freshness.
`check --offline` remains a compatible spelling of the same fixed-input check.

Use `mise x -- just native` for the same formatting, strict Clippy, and workspace tests without starting the proof or security stages.
Its success message establishes only native checks on the executing platform.
`check` calls this same command implementation before every remaining required stage.
A native result never replaces current source-bound proof evidence or dependency and secret checks.

The supported initial target is Apple Silicon macOS.
Rust 1.99.0 and the versions checked by the verification task define the toolchain.
Kani 0.68.0 uses its bundled compiler and model checker.
Proof domains use fixed input bounds and explicit harness unwinding rather than elapsed-time limits.
The proof and negative-control commands impose no wall-clock verdict limit.
An external interruption leaves verification inconclusive and supplies no passing evidence or accepted counterexample.

## Hosted verification

The [manual verification workflow](../.github/workflows/verification.yml) documents the supported hosted environment: the Apple Silicon `macos-26` runner.
It calls the same complete `just check` command and adds no scheduled or automatic hosted runs.
It uses immutable action revisions, mise 2026.10.11, project-pinned Rust and just, locked verification-tool installs, and the public advisory commit.
Kani and Gitleaks archives must match the [checked-in checksums](../verification/tool-assets.sha256) before extraction or setup.
The Kani bundle supplies its own fixed nightly compiler and model checker.
The source-bound evidence includes the workflow and checksum manifest.
The hosted runner image can receive OS updates.
Its label documents an execution platform rather than an immutable image digest.
Local native and security checks supply scoped feedback while hosted execution and current source-bound proof results remain required evidence.
Preparing the workflow or checking its syntax doesn't establish that the hosted setup or checks have executed successfully.

## Security inputs

Use `mise x -- just security --advisory-repo PATH` to run the same secret and dependency checks independently of native tests and proofs.
This command establishes only its stated security coverage and never makes a stale proof receipt current.
Provision an advisory repository explicitly from the [official RustSec database](https://github.com/rustsec/advisory-db).
The pinned [advisory commit](https://github.com/rustsec/advisory-db/commit/999ec89d5d3f10f20be14750ec8c2d8b44e0ac9b) is part of the reviewable project configuration.
Advancing the pin requires a separate review.
A successful check with that pin doesn't establish that no newer advisory exists.

The task exports regular committed blobs under `crates` and `rust` into an owned temporary directory.
It refuses malformed identities, paths, modes, sizes, and incomplete object responses.
Raw object reads avoid archive attributes, working-tree filters, and untracked files.
The task hashes the exported paths and contents and requires the digest to remain unchanged through the audit.
Cargo-audit runs with the fixed database, `--no-fetch`, and `--no-yanked`, avoiding mutable advisory and registry-index lookups.
Its report must include nonempty advisory and dependency counts, no vulnerabilities or warnings, no ignores, no architecture or OS filters, no severity threshold, and exactly the supported informational warning classes.
The production `AuditCoverage` decision rejects incomplete settings, empty work, and findings.
The complete gate requires its scalar Kani harness and production negative control.
A native test alone doesn't discharge that proof obligation.
The security receipt records the source digest, tool versions, advisory revision, snapshot digest, and unevaluated freshness.
Secret scanning selects `.gitleaks.toml` explicitly and doesn't inherit `GITLEAKS_CONFIG` or `GITLEAKS_CONFIG_TOML`.
The source digest includes that configuration and any root `.gitleaksignore` file, binding allowlists and ignore fingerprints to the same evidence.
Generated build outputs and Git metadata remain excluded by the project configuration.
The native adapter fixtures cover raw Git object extraction and report decoding under trusted Git, cargo-audit, and filesystem semantics.
These fixtures don't prove those external programs correct or replace execution on the documented verification environment.

## Deterministic core

`crates/core` uses neither the standard library, external dependencies, nor heap allocation.
The adapter supplies scratch storage for Kahn's topological elimination algorithm.
ID construction rejects zero, sequence increments reject overflow, and revision comparisons require a nonzero exact match.
Readiness requires open work with every prerequisite completed.
`WorkCounts::record` consumes counts and returns exact checked additions for one of four work states and both prerequisite outcomes.
Only open tickets with completed prerequisites increment ready counts.
Only open or in-progress tickets with unresolved prerequisites increment blocked counts.
The repository-count harness quantifies all seven `u64` counters, all four states, and both prerequisite outcomes.
An independent increment table and widened arithmetic check every resulting field and exact overflow refusal.
All eight state and prerequisite combinations and overflow have required reachable covers.
Repeated successful transitions from zero preserve the status partition and keep ready and blocked counts within active work.
`MatchedRevision` constructs evidence of a positive exact revision match before the adapter applies a patch.
Its consuming transition preserves unchanged revisions and advances changed revisions once, rejecting overflow.
The revision-change harness checks all `u64` expected and current revisions and both change states.
It verifies constructor refusal, exact increments, unchanged updates at `u64::MAX`, and overflow for real changes.
Relationship validation rejects missing endpoints, self-reference, and cycles independently for parent and dependency edges.

The Kani harnesses import production functions directly.
Identifier, sequence, revision, and readiness harnesses quantify their complete admitted scalar input domains.
The proof-completeness harness also checks every combination of the reported scalar counts.
Report statuses map to a closed result enum, with unknown values rejected.
Covers require satisfied results, while checks in other categories can be unreachable.
The result harness checks all six variants, both values of the cover flag, and all three check categories used by negative controls.
The report accepts failure only for assertions and unsatisfiable coverage only for covers in a negative-control report.
Unknown and solver-error statuses map to the invalid variant and can't support either verdict.
The graph harness explores all edge subsets of a three-node directed triangle with unwinding assertions enabled.
The graph proof covers three-node triangle subgraphs.
Native tests use transitive closure to check all 512 three-node directed graphs and exercise the app's graph boundary.
The graph proof assumes one scratch entry per node.
Native tests cover refusal of incorrect scratch sizes.
Descendant selection uses an iterative frontier with one scratch mark per node and no recursion.
Each node moves from outside to pending to included at most once.
The selector excludes the root from the final selection.
The selector checks unique IDs, root and parent endpoints, self-reference, and scratch length.
The app validates acyclic parent relationships before calling it.
Its Kani harness explores all 64 parent maps on three fixed IDs and each possible root, checking self-reference refusal and strict reachability with unwinding assertions enabled.
The proof covers that finite domain, including non-self cycles, rather than arbitrary graph sizes.
An independent native transitive-closure oracle checks the same parent maps and roots for both strict descendant and root-inclusive subtree selection.
Native boundary cases also exercise unsorted IDs, missing endpoints, duplicate IDs, and reused scratch storage.

`ParentRelations` owns an immutable borrow of checked parent links, validating unique IDs, endpoints, and self-reference once.
It permits non-self cycles, while the app's snapshot validation requires an acyclic parent forest.
Its root-inclusive subtree method checks root existence and scratch length on every call and resets the scratch marks.
`mark_descendants` calls the same traversal and excludes the root after success, preserving the free wrapper's error precedence.
The subtree Kani harness calls the same checked constructor and method as goal grouping.
It checks all 64 three-node parent maps and each root using an independent one-or-two-edge reachability formula plus root membership.
The proof covers that finite domain, including non-self cycles, with required covers for rejection, singleton trees, and complete trees.
Native checks reuse one checked view across roots and test constructor errors, scratch refusal, root errors, wrapper error precedence, and an eight-node reverse parent chain.

Blocker and impact selection share an iterative frontier over dependency edges with an explicit query variant.
Blocker selection follows prerequisite edges.
Completed nodes stop traversal and stay outside the result.
Cancelled nodes remain visible but stop traversal, and closed targets return empty selections.
The production core checks unique IDs, endpoints, self-reference, and scratch size.
The app validates acyclic dependency relationships before calling it.
Eight blocker Kani harnesses partition all edge subsets of a fixed three-node directed acyclic graph.
Each harness checks all 27 combinations of active, done, and cancelled states for its fixed edge subset.
The expected result uses an independent formula for paths with one or two edges.
The harnesses use reversed node order and unwinding assertions.
Cover checks require transitive reachability, alternate paths past completed branches, terminal cancellation, and closed targets.
This proof covers that finite graph and state domain.
Native tests compare all 64 self-edge-free three-node directed graphs, 27 state assignments, and each root to independent active-edge transitive closure.
Boundary tests also cover an eight-node reverse chain, reused scratch, missing endpoints, duplicate IDs, and self-reference.

Impact selection follows the edges in reverse and includes only active dependents.
Done and cancelled dependents stop that path and stay outside the result.
A done root returns an empty selection, while a cancelled root can still affect active dependents.
The shared selector retains the same input checks and root exclusion.
Eight impact Kani harnesses partition the same fixed three-node directed acyclic graph and all 27 state assignments per edge subset.
They check an independent formula for reverse paths with one or two edges.
The harnesses reverse node order and enable unwinding assertions.
Cover checks require transitive impact, stopped done and cancelled branches, alternative paths, and active consumers of a cancelled root.
This proof covers that finite graph and state domain.
The independent native active-edge closure oracle checks both directions for all 64 self-edge-free three-node directed graphs, 27 state assignments, and each root.
Both query variants also run the native input-boundary and eight-node chain checks.

`DependencyRelations` holds immutable borrows of the checked dependency nodes and edges.
It validates unique IDs, endpoints, and self-reference once, while each query checks its root and scratch length and resets scratch marks.
The free selector preserves its error precedence and calls the same method used by ordinary queries and bottleneck discovery.
The existing finite blocker and impact harnesses remain bound to this production method.
The constructor harness quantifies all seven positive `u64` IDs in three nodes and two edges, checking acceptance exactly when IDs are unique and all edges have known, distinct endpoints.
Its four covers require successful construction and each refusal variant.
The native closure oracle reuses a checked view across roots and query variants for every existing three-node graph and state assignment.
Boundary checks cover constructor errors, wrong scratch length, missing roots, scratch reuse, and free-selector error precedence.
Reusing validation keeps each candidate's impact traversal independent.
Affected subgraphs can overlap across candidates.

`BottleneckRank` admits only positive affected counts and stores reverse affected count, reverse exhaustive priority, and positive ticket ID in a private ordering key.
Its harness quantifies both complete `u64` affected-count domains, both positive ID domains, and all three priorities for each key.
It checks zero refusal and exact ordering using independent boolean formulas for each tie level.
Eight covers require zero refusal, equality, each ordering level, a saturated count, and reversed order.

`DependencyMark::included_in` counts only included impact marks whose consumer belongs to the selected goal scope.
Its harness checks every combination of the three mark variants and both membership values, with six required covers.
Without a consumer scope, the adapter supplies true for membership and preserves existing counts.
The adapter constructs an ID set through the existing root-inclusive subtree selector and joins it to final global impact marks.
The parent, impact, and rank proofs keep their declared domains.

No proof assumes that an operation succeeds.
Cover properties require reachable outcomes for the decisions each harness checks.

## Persistence and process boundaries

Native CLI tests use real redb files and separate processes.
Output fixtures connect standard output and standard error to pipes whose readers close before the child starts.
Argument errors preserve status 2 when diagnostics succeed, while a failed diagnostic write returns status 1.
Operation and response-output failures also return status 1 without a panic or an attempted write to another stream.
These checks trust native descriptor and Rust output semantics outside the core verifier's model.
Reassess this boundary before changing output buffering or supporting another platform.
Argument capture and JSON creation share the production `AddInput` and `Store::add` entry point.
Native fixtures check capture defaults, literal Unicode and multiline arguments, repeated memberships and dependencies, relation validation, rollback, and schema parity with `add`.
They exercise cross-command request replay in both directions, original responses after updates, and conflicts for changed fields or raw array order and duplicates.
Argument failures precede database access, while canonical storage validation handles blank text and invalid relationships inside the existing mutation transaction.
Fixtures also check ignored standard input, explicit initialization, and bounded writer contention.
Clap parsing, Serde request representation, and redb transactions form a native adapter boundary beyond the core verifier's model.
Capture introduces no new core decision.
Existing source-bound identifier, revision, and graph proofs keep their stated domains.
Reassess this boundary if capture gains separate normalization, validation, or mutation rules.
Literal note arguments construct the existing `NoteInput` and call the same `Store::note` mutation as JSON input.
Native fixtures preserve Unicode, quotes, newlines, leading dashes, and JSON-looking bodies and verify that literal input ignores a prewritten standard-input file.
They check JSON stdin, explicit `--input -`, and file compatibility, cross-form request replays, target-ticket and operation conflicts, blank text rollback, sequence allocation, and writer contention.
Clap rejects conflicting input sources and a request-ID flag without a literal body before storage access.
The published note schema remains the canonical request type rather than the CLI source-selection wrapper.
Source selection stays within the existing native Clap, Serde, and redb adapter boundary and adds no core state transition.
Reassess that boundary if either note input form gains separate normalization or mutation rules.
The suite covers restart and idempotent initialization, shared repository context, revision conflicts, replay identities, relation rollback, note pagination, literal Unicode search, process locks, and preservation of unknown files.
Layout checks include normal and multimap tables and refuse foreign databases without adding backlog tables.
Generated schemas validate real command responses and errors.
Update requests carry a nonzero revision type, so JSON decoding rejects zero before storage access.
Native stdin and file fixtures verify the field error, byte preservation, missing directories, writer locks, and valid and stale positive revisions.
They check the schema's positive range, including `u64::MAX`.
The existing revision proof continues to check the production comparison over both complete `u64` input domains.
Unchanged-update fixtures check empty and nullable patches, normalized repository and dependency arrays, every mutable field, stale revisions, parent-flag validation, and saturated revisions.
They run unchanged updates before meaningful updates from the same revision to verify that repeated saves don't create conflicts.
The storage adapter uses derived ticket equality after normalization and skips ticket writes for unchanged results.
Native tests cover string and array equality, normalization, and redb transactions beyond the scalar proof's domain.
Unchanged updates use the writer lock and the normal snapshot validation and commit protocol.
The contract preserves app records and revisions rather than native file bytes.
Native schema tests cover positive IDs, page size limits, argument conflicts, and nullable options.
They check direct dependent summaries across repositories and statuses.
Separate fixtures exclude unrelated children and indirect dependents.
Note-query fixtures place discoveries outside the default notes page and check ticket scope, Unicode literals, case sensitivity, filtered counts, and pagination.
Empty and whitespace-only note queries fail before the database opens.
Goal-query fixtures reparent earlier IDs into a four-level hierarchy and check strict descendant scope before filtering and pagination.
They cover closed roots and intermediate parents, unrelated dependency edges, external blockers, repository and status intersections, note search, and refreshed scope after reparenting.
Blocker fixtures check shared prerequisites across repositories, completed and cancelled traversal boundaries, alternative active paths, reopening, and closed targets.
Readiness uses the full snapshot.
They check final filtered pagination and the generated command schema.
Impact fixtures check transitive consumers across repositories, shared-path deduplication, independent parent links, other blockers, terminal branches, reopening, and cancelled and done roots.
They check exact repository filtering after traversal, final pagination, extreme offsets, and schema discovery without storage.
Repository fixtures check null and case-sensitive named groups, distinct membership, global dependencies, all work states, active-group filtering, refreshed membership, and final pagination.
They include valid raw duplicate repository membership that bypasses write normalization.
Unassigned list and search fixtures check repository conflicts, schemas, and intersections with existing selectors.
String grouping, membership deduplication, ordering, and joins over a full snapshot are native adapter checks outside the scalar counter proof.
Goal fixtures check root-inclusive deep trees across repositories and closed intermediate parents, closed roots with active descendants, standalone leaves, and root-priority ordering before pagination.
They check global prerequisite updates, repeated reparenting, splitting and merging goals, filtered counts, extreme offsets, and schema discovery before storage access.
Unfiltered goal totals must partition every fixture ticket after each parent change.
The goal adapter uses the existing proved checked counters and the root-inclusive selector.
Native checks cover status mapping, global joins, root selection, record alignment with scratch marks, ordering, filtering, and JSON/schema compatibility.
Repository summary fields stay flat while goal counts sit inside `work`.
Bottleneck fixtures compare each affected count with the existing unfiltered impact result.
They also check shared-path deduplication, completed and cancelled cuts, alternative paths, cancelled roots, in-progress roots, zero-impact exclusion, and readiness.
Further fixtures check count-first ranking, priority and ID ties, candidate-only repository and strict descendant filters, and unassigned work.
They also check missing roots under empty candidate filters, global consumers, pagination, and schema validity of real responses.
These native checks cover status and priority mapping, candidate joins, count conversion, filtering, and sorting integration beyond the scalar ordering proof and finite graph proofs.
Goal-focused bottleneck fixtures check external prerequisites, scoped re-ranking and zero exclusion, and paths through outside intermediate work.
They also check root-inclusive nested and leaf scopes, closed parent membership, and existing dependency cuts.
They check independent candidate filters, eager missing-goal errors, refreshed parent membership, nullable positive argument schemas, unchanged default results, and filtered pagination.
A separate 51-consumer fixture verifies complete counts beyond the default impact page and with a one-item candidate page.
The native fixtures establish the consumer ID-set join and count/sort composition across the scalar and finite graph proof boundaries.
Recent-note fixtures interleave notes across repositories, closed tickets, and priorities and recover complete note bodies with current ticket summaries and global blockers.
They check persisted note ID order, original note replays, later ticket updates, empty results, complete counts beyond the default page, offsets, and generated argument and response schemas.
The adapter reverses the validated snapshot's redb note-key order using the same stored-order assumption as ticket detail.
It uses the existing page helper before cloning and joining the requested entries, preserving full-note counts and offsets.
Snapshot validation checks every note's key, identifier, and ticket reference before pagination.
Native checks exercise redb iteration, append transactions, Serde values, and snapshot joins outside the supported core verifier's model.
This query adds no core state transition or custom ordering algorithm.
The supported core proofs keep their declared domains.
Reassess the ordering and pagination boundary if notes become mutable, IDs stop expressing append order, or the storage engine changes.
Query tests hold a separate native reader while exercising all nine read commands and refusing a writer.
They verify read-only file access and source-byte preservation for clean databases.
Clean foreign layouts, unsupported app schemas, and corrupt records fail validation without switching to a writer.
The abrupt-exit fixture first requires immutable archive reads to return `recovery_required` and preserve the dirty source.
An ordinary query then recovers the committed snapshot and permits further writes.
These checks exercise the Serde, Schemars, and redb adapter together.
The storage adapter trusts redb 4.4.0's transaction and immediate-durability contracts and the native filesystem.
Tests establish observable native behavior under the declared filesystem assumptions.
They don't prove filesystem or hardware correctness.

`store::query_snapshot` selects a shared reader or native recovery before app validation.
Only redb's `RepairAborted` result permits the recovery path.
Both open attempts share one contention retry budget.
The timeout bounds waiting for a native lock and doesn't limit recovery or filesystem I/O.
Native recovery precedes layout validation and can update an unclean foreign redb file before the app rejects it.
The byte-preservation contract applies to clean query sources and to archive sources that never enter recovery.

## Archive publication

`backup` and `restore` share `store::copy_archive`.
A read-only source transaction supplies all four typed tables.
The tables include sequence metadata and replay records.
The existing snapshot checks validate the app records and layout before staging.
They don't certify the contents of every historical replay response.
The copy preserves those response records without reinterpreting them.

`PreparedArchive` owns the checked file and its absolute destination.
Only successful preparation constructs this private type.
Preparation commits the new database with immediate durability, closes it, reopens it read-only for snapshot validation, and synchronizes the file.
New parent directories synchronize their contents and their links in the parent before preparation continues.
Publication uses `tempfile` 3.27.0's no-clobber operation and then synchronizes the destination directory.
An error after publication reports that the destination already exists.

Native tests verify restoration of relationships, revisions, notes, replay responses, and future sequence allocation.
They check source-byte preservation, read-only source files, private file permissions, invalid schemas, corruption, writer locks, and competing publishers.
A deterministic collision fixture creates the destination between preparation and publication.
A child process exits without cleanup after staging to check that the final destination stays absent.
A synchronization callback injects a directory flush failure after publication and verifies the phase-specific error and complete destination.
The tests exercise production redb and filesystem operations on the supported Mac target.
These operations cross native filesystem and redb semantics that the Rust-core Kani harnesses don't model.
The supported core retains its source-bound proofs.
Archive checks rely on redb snapshot isolation, successful file and directory synchronization, and no-clobber publication in a stable destination directory.
They establish process-level behavior and protocol ordering, not a proof of power-loss behavior or hostile filesystem changes.
Reassess the native checks and durability protocol before supporting another target or changing the storage engine.

## Evidence and negative controls

Long checks inherit standard output and standard error through Rust's [process API](https://doc.rust-lang.org/std/process/struct.Command.html#method.status).
The runner prints the command before starting it and preserves its failure status or launch error.
Standard input stays closed to prevent interactive prompts.
Version and dependency-audit commands keep captured output for parsing and display their diagnostics after exit.
Expected-failure proof controls also keep their output and announce the harness and mutation before starting.
The runner doesn't impose an ordering between standard output and standard error or flush a child's own buffers.

Native runner tests hold a child at an owned release-file predicate until both flushed output markers reach the observer.
Only then does the test release the child, require its nonzero exit to propagate, and check both raw output tails.
Timeouts diagnose a stuck handshake rather than establish its ordering.
The test releases and waits for the child and joins both stream observers before reporting a probe failure.
Other fixtures check successful commands, captured bytes, nonzero exits, and missing executables.
These checks exercise native process and filesystem semantics on the supported Mac target, outside the core verifier's model.
Reassess this boundary before supporting another target or adding custom output forwarding.

Native report tests append invalid covers to an otherwise valid proof report and require rejection.
They preserve acceptance of unreachable checks in other categories and reject unknown statuses.

Proof discovery must match the complete expected harness inventory.
Every expected harness must execute successfully and include actual checks.
Verification rejects empty, incomplete, timed-out, and failed reports.
The proof receipt binds the result to the current source, manifests, lockfile, configuration, and verifier version.
Both successful proof reports and negative controls must identify the `aarch64-apple-darwin` target fixed in `verification.json`.
Missing or different target metadata can't supply the required evidence.
Source hashing prefixes each relative UTF-8 path and content with its byte length, keeping file boundaries unambiguous for binary inputs.
Native fixtures distinguish an embedded NUL-delimited path from another file and require scan configuration and ignore-file creation, edits, and deletion to change the digest.
`just verify-evidence` rejects stale receipts.
Negative controls mutate production decisions in isolated copies and require assertion counterexamples from the corresponding real harnesses.
Each control requires one selected and executed harness, one failure, no successes, a completed report, and the expected harness identity and target.
The matching error entry must identify an ordinary assertion failure with measured failed properties rather than a solver, resource, or unsupported-semantics error.
An assertion failure mixed with an error can't establish a counterexample, including spurious failures after ignored quantifiers.
`CounterexampleEvidence` is the production acceptance predicate for these counts and facts.
Its harness quantifies all five `usize` counts and all six Boolean facts, while the check-result harness covers every status and category combination.
Native report fixtures exercise mixed solver errors, unsupported-quantifier error details, incomplete and duplicated records, mismatched identities, missing fields, unknown statuses, and wrong check categories.
They preserve valid negative controls whose changed implementation makes an original cover unsatisfiable or unreachable.
Those negative-control covers supply no replacement for the satisfied covers required from the original production proofs.
Mutations cover blocker completion and cancellation, impact root cancellation, consumer states, and edge direction.
They also cover proof result acceptance, descendant root exclusion, readiness, revision, and cycle decisions.
Revision-change controls bypass the match constructor and force unchanged content to advance.
Repository-count controls misclassify closed work as ready and omit blocked in-progress work.
The subtree control removes root initialization from the actual traversal.
Dependency-constructor and bottleneck-rank controls accept self-reference and rank priority before affected work.
The scoped-dependency control ignores consumer membership in the actual production predicate.
The counterexample control bypasses production error classification, so its contract harness must reject it.
The original checkout is never mutated by those controls.
