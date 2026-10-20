# `WhatMustBeDone`

A local backlog for goals, shared prerequisites, and coding agent handoffs.

## Installation

```sh
mise install
mise x -- cargo install --path . --locked
```

## Usage

```sh
(
  set -eu
  wmbd_demo_db="$(mktemp -d)/backlog.redb"
  wmbd --db "$wmbd_demo_db" init
  printf '%s\n' '{"title":"Ship project A","body":"Replace duplicated verification setup with the shared runner","repos":["owner/project-a"],"priority":"high"}' | wmbd --db "$wmbd_demo_db" add
  printf '%s\n' '{"title":"Build the shared runner","repos":["owner/tooling"]}' | wmbd --db "$wmbd_demo_db" add
  printf '%s\n' '{"title":"Adopt the runner","parent":1,"depends_on":[2],"repos":["owner/project-a"]}' | wmbd --db "$wmbd_demo_db" add
  wmbd --db "$wmbd_demo_db" list --under 1
  wmbd --db "$wmbd_demo_db" blockers 3 --ready
  printf '%s\n' '{"revision":1,"status":"done"}' | wmbd --db "$wmbd_demo_db" update 2
  printf '%s\n' '{"revision":1,"status":"in_progress"}' | wmbd --db "$wmbd_demo_db" update 3
  printf '%s\n' '{"body":"Discovery: reuse existing checks in project B too. Next: connect the integration."}' | wmbd --db "$wmbd_demo_db" note 3
  wmbd --db "$wmbd_demo_db" show 3 --context
)
# Agent setup and the full command reference: docs/agents.md.
```

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [Massachusetts Institute of Technology (MIT) license](LICENSE-MIT), at your option.
You license each contribution you submit under the same two licenses, without other terms, unless you state otherwise.
