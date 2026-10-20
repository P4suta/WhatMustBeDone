# Using the shared backlog

`wmbd` keeps goals, implementation work, prerequisites, and discoveries in one local redb database.
Repository names are ticket memberships, so a shared prerequisite can connect work across repositories.
Parent relationships describe a goal's work tree, while `depends_on` describes what must finish first.

## Start a session

Run `wmbd init` once to create the default per-user backlog.
Running it again preserves existing data.
Use `--db PATH` on every command when choosing a separate backlog.
Backlog queries and mutations require an existing initialized database.
`schema` needs no database, and `restore` creates a new destination.

```sh
wmbd goals --active
wmbd repos --active
wmbd notes --limit 10
wmbd bottlenecks --ready
```

Goals are top-level tickets.
Goal summaries count the goal and all descendants.
Repository overviews include work without a repository membership.
Recent notes include their full bodies and show the ticket's current status and blockers.
Bottlenecks rank shared prerequisites by the number of active consumers waiting on them, then priority and ticket ID.
`--ready` selects open tickets whose direct prerequisites are all done.

## Capture and resume

Create the purpose before breaking it into work.
Use the returned ticket IDs rather than assuming a particular sequence.

```sh
wmbd capture "Ship project A" --repo owner/project-a --priority high
wmbd capture "Build the shared runner" --repo owner/tooling --repo owner/project-b
wmbd capture "Adopt the runner" --parent 1 --depends-on 2 --repo owner/project-a
wmbd note 2 "Project C can reuse this too. Next: connect its existing checks."
```

The example IDs assume a fresh backlog whose first two captures returned IDs 1 and 2.
Repeat `--repo` or `--depends-on` to add more memberships or prerequisites.
`capture` accepts a literal title and optional `--body`, `--priority`, `--parent`, and `--request-id` arguments.
New tickets default to normal priority and an empty body.
`note ID BODY` preserves literal newlines and quotes in the body.
Use `note ID -- BODY` for a body that begins with a dash.

`add`, `update`, and a `note` without a literal body read JSON from standard input.
Pass `--input PATH` to read a file instead.
JSON input accepts at most 1 MiB and rejects unknown fields, trailing data, and invalid values.
Literal note bodies conflict with an explicit `--input` option.

Reuse a `request_id` when retrying the same capture or note.
An identical request returns the original response without allocating another ticket or note.
Reusing that identity for a different payload, ticket, or operation fails with `request_conflict`.

## Find work

```sh
wmbd show 3 --context
wmbd list --under 1 --ready
wmbd blockers 3
wmbd impact 2
wmbd bottlenecks --for-goal 1
wmbd search "runner" --under 1
```

`show --context` includes ancestors and direct prerequisites alongside notes and related ticket summaries.
`blockers` follows active prerequisites across repositories.
Completed tickets stop traversal, while cancelled prerequisites remain visible and stop their branch.
`impact` follows active dependents in the opposite direction.
`--under ID` excludes the named ticket and selects its descendants.
`bottlenecks --for-goal ID` counts consumers within that goal and its descendants while retaining prerequisites from other repositories.
Repository filters select displayed candidates without discarding their global dependency context.
Search matches literal, case-sensitive text in titles, bodies, and notes.

List commands return a page with `items`, `total`, and `next_offset`.
Use `--limit` and `--offset` to read later pages.
Queries compute full counts before pagination.
The default page size is 50, and `show` returns 20 notes by default.
Use `show --notes-query TEXT` to find a discovery within one ticket's notes.

## Record progress

Read the current revision before an update.
The update below assumes ticket 3 still has revision 1.

```sh
printf '%s\n' '{"revision":1,"status":"in_progress"}' | wmbd update 3
wmbd note 3 "Implemented the adapter. Next: run the integration checks."
```

Updates require a positive revision and reject a stale one with `conflict` and `current_revision`.
Read the ticket again before retrying a stale update.
An unchanged update preserves the revision.
Statuses are `open`, `in_progress`, `done`, and `cancelled`.
Only `done` satisfies a prerequisite.
Use an empty `depends_on` array to remove prerequisites or `clear_parent: true` to remove a parent.
Parent and dependency relationships reject missing tickets, self-reference, and cycles.

## Preserve the backlog

```sh
wmbd backup /path/to/new-backup.redb
wmbd --db /path/to/new-backlog.redb restore /path/to/new-backup.redb
```

Backups preserve tickets, notes, revisions, request replay records, and sequence allocation.
Both commands require a new destination and never replace an existing file.
Inspect the restored backlog before removing the original database.

## Command contracts

Use `wmbd COMMAND --help` for the complete arguments and `wmbd schema COMMAND` for its JSON request, response, and error schemas.
Successful commands write `{"api_version":1,"data":...}` to standard output.
Failures write `{"api_version":1,"error":...}` to standard error.
Exit status 2 identifies invalid command arguments.
Status 1 identifies an operation or output failure.
Help and version output use plain text.
`--lock-timeout-ms` bounds database-lock contention, defaults to 5000, and accepts 0 through 60000.
It doesn't bound filesystem operations or recovery.

The supported initial platform is Apple Silicon macOS.
See [verification](verification.md) for the native checks, supported proof domains, fixed advisory inputs, and complete hosted verification command.
