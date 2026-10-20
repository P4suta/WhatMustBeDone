use crate::api::*;
use crate::error::{Error, Result};
use crate::store::Store;
use clap::{Args, Parser, Subcommand, ValueEnum};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_PAGE_LIMIT: u32 = 50;
const DEFAULT_DETAIL_NOTES_LIMIT: u32 = 20;
const MAX_JSON_INPUT_BYTES: usize = 1024 * 1024;

#[derive(Parser)]
#[command(name = "wmbd", version, about = "A local backlog for coding agents")]
pub struct Cli {
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true, default_value_t = 5000, value_parser = clap::value_parser!(u64).range(0..=60_000))]
    lock_timeout_ms: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize the local backlog without replacing existing data.
    Init,
    /// Create a ticket from JSON.
    Add(InputArgs),
    /// Capture a ticket directly from command arguments.
    Capture(AddInput),
    /// Edit a ticket using its current revision.
    Update {
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        id: u64,
        #[command(flatten)]
        input: InputArgs,
    },
    /// Append a progress or discovery note.
    Note {
        #[arg(value_parser = clap::value_parser!(u64).range(1..))]
        id: u64,
        /// Save this literal body, or omit it to read JSON.
        #[arg(conflicts_with = "input")]
        body: Option<String>,
        /// Reuse this identity when retrying a literal body.
        #[arg(long, requires = "body", conflicts_with = "input")]
        request_id: Option<String>,
        #[command(flatten)]
        input: InputArgs,
    },
    /// Read recent notes across all tickets with their current context.
    Notes(NotesArgs),
    /// List ticket summaries.
    List(ListArgs),
    /// Summarize work by repository, including unassigned work.
    Repos(ReposArgs),
    /// Summarize top-level goals and all work beneath them.
    Goals(GoalsArgs),
    /// Trace active prerequisites across repositories.
    Blockers(BlockersArgs),
    /// Trace active work that relies on a shared prerequisite.
    Impact(ImpactArgs),
    /// Discover shared prerequisites by the work currently waiting on them.
    Bottlenecks(BottlenecksArgs),
    /// Read a ticket and optionally its working context.
    Show(ShowArgs),
    /// Search all ticket titles, bodies, and notes using literal text.
    Search(SearchArgs),
    /// Save a consistent database copy to a new file.
    Backup(BackupArgs),
    /// Restore a database copy into a new backlog file.
    Restore(RestoreArgs),
    /// Describe a command's JSON request and response types.
    Schema {
        #[arg(value_enum)]
        command: SchemaCommand,
    },
}

#[derive(Args)]
pub struct InputArgs {
    /// Read JSON from this file, or use - for standard input.
    #[arg(long, default_value = "-")]
    pub input: PathBuf,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupArgs {
    pub destination: PathBuf,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreArgs {
    pub source: PathBuf,
}

fn default_limit() -> u32 {
    DEFAULT_PAGE_LIMIT
}
fn default_notes_limit() -> u32 {
    DEFAULT_DETAIL_NOTES_LIMIT
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotesArgs {
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"anyOf":[{"required":["all","status"],"properties":{"all":{"const":true},"status":{"not":{"type":"null"}}}},{"required":["repo","unassigned"],"properties":{"repo":{"not":{"type":"null"}},"unassigned":{"const":true}}}]}))]
pub struct ListArgs {
    /// Select all descendants of this ticket, excluding the ticket itself.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub under: Option<u64>,
    #[arg(long)]
    pub repo: Option<String>,
    /// Select tickets with no repository membership.
    #[arg(long, conflicts_with = "repo", default_value_t = false)]
    #[serde(default)]
    pub unassigned: bool,
    #[arg(long, value_enum)]
    pub status: Option<Status>,
    #[arg(long, conflicts_with = "status", default_value_t = false)]
    #[serde(default)]
    pub all: bool,
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub ready: bool,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub id: u64,
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub context: bool,
    /// Return only notes containing this literal text.
    #[arg(long)]
    #[schemars(length(min = 1))]
    pub notes_query: Option<String>,
    #[arg(long, default_value_t = DEFAULT_DETAIL_NOTES_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_notes_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub notes_limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub notes_offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReposArgs {
    /// Select repository groups containing open or in-progress work.
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub active: bool,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalsArgs {
    /// Select goals with open or in-progress work in their trees.
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub active: bool,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockersArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub id: u64,
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub ready: bool,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImpactArgs {
    #[arg(value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub id: u64,
    #[arg(long)]
    pub repo: Option<String>,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"required":["repo","unassigned"],"properties":{"repo":{"not":{"type":"null"}},"unassigned":{"const":true}}}))]
pub struct SearchArgs {
    pub query: String,
    /// Select all descendants of this ticket, excluding the ticket itself.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub under: Option<u64>,
    #[arg(long)]
    pub repo: Option<String>,
    /// Select tickets with no repository membership.
    #[arg(long, conflicts_with = "repo", default_value_t = false)]
    #[serde(default)]
    pub unassigned: bool,
    #[arg(long, value_enum)]
    pub status: Option<Status>,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Args, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"required":["repo","unassigned"],"properties":{"repo":{"not":{"type":"null"}},"unassigned":{"const":true}}}))]
pub struct BottlenecksArgs {
    /// Count affected consumers in this ticket and all its descendants.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub for_goal: Option<u64>,
    /// Select candidates below this ticket, excluding the ticket itself.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[schemars(range(min = 1))]
    pub under: Option<u64>,
    #[arg(long)]
    pub repo: Option<String>,
    /// Select candidates with no repository membership.
    #[arg(long, conflicts_with = "repo", default_value_t = false)]
    #[serde(default)]
    pub unassigned: bool,
    /// Select open candidates whose direct prerequisites are all done.
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub ready: bool,
    #[arg(long, default_value_t = DEFAULT_PAGE_LIMIT, value_parser = clap::value_parser!(u32).range(1..=500))]
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 500))]
    pub limit: u32,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SchemaCommand {
    Init,
    Add,
    Capture,
    Update,
    Note,
    Notes,
    List,
    Repos,
    Goals,
    Blockers,
    Impact,
    Bottlenecks,
    Show,
    Search,
    Backup,
    Restore,
}

#[derive(Serialize, JsonSchema)]
pub struct SchemaOutput {
    pub command: SchemaCommand,
    pub input: Value,
    pub output: Value,
    pub error: Value,
}

pub fn execute(cli: Cli) -> Result<Value> {
    if let Command::Schema { command } = cli.command {
        return encode(schema(command));
    }
    let path = match cli.db {
        Some(path) => path,
        None => directories::ProjectDirs::from("", "", "WhatMustBeDone")
            .map(|d| d.data_local_dir().join("backlog.redb"))
            .ok_or_else(|| {
                Error::input("db", "Cannot locate the user data directory; pass --db")
            })?,
    };
    let wait = Duration::from_millis(cli.lock_timeout_ms);
    match cli.command {
        Command::Init => {
            let result = Store::open(&path, wait, true)?.initialize()?;
            encode(result)
        }
        Command::Add(args) => {
            let input: AddInput = read_input(args)?;
            let result = Store::open(&path, wait, false)?.add(input)?;
            encode(result)
        }
        Command::Capture(input) => {
            let result = Store::open(&path, wait, false)?.add(input)?;
            encode(result)
        }
        Command::Update { id, input } => {
            let input: UpdateInput = read_input(input)?;
            let result = Store::open(&path, wait, false)?.update(id, input)?;
            encode(result)
        }
        Command::Note {
            id,
            body,
            request_id,
            input,
        } => {
            let input = match body {
                Some(body) => NoteInput { body, request_id },
                None => read_input(input)?,
            };
            let result = Store::open(&path, wait, false)?.note(id, input)?;
            encode(result)
        }
        Command::Notes(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            encode(state.recent_notes(args.offset, args.limit as usize)?)
        }
        Command::List(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let items = summaries(
                &state,
                RepoScope::new(args.repo.as_deref(), args.unassigned),
                args.status,
                args.all,
                args.ready,
                None,
                args.under,
            )?;
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Repos(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let items = state
                .repos()?
                .into_iter()
                .filter(|group| !args.active || group.work.open > 0 || group.work.in_progress > 0)
                .collect();
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Goals(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let items = state
                .goals()?
                .into_iter()
                .filter(|group| !args.active || group.work.open > 0 || group.work.in_progress > 0)
                .collect();
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Blockers(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let scope = state.dependency_scope(args.id, wmbd_core::DependencyQuery::Blockers)?;
            let items = selected_summaries(&state, args.ready, |ticket| scope.contains(&ticket.id));
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Impact(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let scope = state.dependency_scope(args.id, wmbd_core::DependencyQuery::Impact)?;
            let items = selected_summaries(&state, false, |ticket| {
                scope.contains(&ticket.id)
                    && args
                        .repo
                        .as_ref()
                        .is_none_or(|repo| ticket.repos.contains(repo))
            });
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Bottlenecks(args) => {
            let state = crate::store::query_snapshot(&path, wait)?;
            let scope = args.under.map(|id| state.descendants(id)).transpose()?;
            let consumers = args.for_goal.map(|id| state.subtree(id)).transpose()?;
            let repo = RepoScope::new(args.repo.as_deref(), args.unassigned);
            let items = state.bottlenecks(args.ready, consumers.as_ref(), |ticket| {
                repo.includes(ticket) && scope.as_ref().is_none_or(|ids| ids.contains(&ticket.id))
            })?;
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Search(args) => {
            text(&args.query, "query")?;
            let state = crate::store::query_snapshot(&path, wait)?;
            let items = summaries(
                &state,
                RepoScope::new(args.repo.as_deref(), args.unassigned),
                args.status,
                true,
                false,
                Some(&args.query),
                args.under,
            )?;
            encode(page(items, args.offset, args.limit as usize))
        }
        Command::Show(args) => {
            if let Some(query) = &args.notes_query {
                text(query, "notes_query")?;
            }
            let state = crate::store::query_snapshot(&path, wait)?;
            encode(state.detail(
                args.id,
                args.context,
                args.notes_offset,
                args.notes_limit as usize,
                args.notes_query.as_deref(),
            )?)
        }
        Command::Backup(args) => {
            encode(crate::store::copy_archive(&path, &args.destination, wait)?)
        }
        Command::Restore(args) => encode(crate::store::copy_archive(&args.source, &path, wait)?),
        Command::Schema { command } => encode(schema(command)),
    }
}

enum RepoScope<'a> {
    All,
    Named(&'a str),
    Unassigned,
}

impl<'a> RepoScope<'a> {
    fn new(repo: Option<&'a str>, unassigned: bool) -> Self {
        match (repo, unassigned) {
            (_, true) => Self::Unassigned,
            (Some(repo), false) => Self::Named(repo),
            (None, false) => Self::All,
        }
    }

    fn includes(&self, ticket: &Ticket) -> bool {
        match self {
            Self::All => true,
            Self::Named(repo) => ticket.repos.iter().any(|stored| stored == repo),
            Self::Unassigned => ticket.repos.is_empty(),
        }
    }
}

fn summaries(
    state: &Snapshot,
    repo: RepoScope<'_>,
    status: Option<Status>,
    all: bool,
    ready: bool,
    query: Option<&str>,
    under: Option<u64>,
) -> Result<Vec<Summary>> {
    let scope = under.map(|id| state.descendants(id)).transpose()?;
    Ok(selected_summaries(state, ready, |ticket| {
        if scope.as_ref().is_some_and(|ids| !ids.contains(&ticket.id)) {
            return false;
        }
        let status_matches = match status {
            Some(status) => ticket.status == status,
            None => all || matches!(ticket.status, Status::Open | Status::InProgress),
        };
        let repo_matches = repo.includes(ticket);
        let text_matches = query.is_none_or(|query| {
            ticket.title.contains(query)
                || ticket.body.contains(query)
                || state
                    .notes
                    .iter()
                    .any(|note| note.ticket_id == ticket.id && note.body.contains(query))
        });
        status_matches && repo_matches && text_matches
    }))
}

fn selected_summaries(
    state: &Snapshot,
    ready: bool,
    select: impl Fn(&Ticket) -> bool,
) -> Vec<Summary> {
    let mut items: Vec<_> = state
        .tickets
        .iter()
        .filter(|ticket| select(ticket))
        .map(|ticket| state.summary(ticket))
        .filter(|ticket| {
            !ready || wmbd_core::ready(ticket.status == Status::Open, ticket.blocked_by.is_empty())
        })
        .collect();
    items.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    items
}

fn read_input<T: DeserializeOwned>(args: InputArgs) -> Result<T> {
    let source: Box<dyn Read> = if args.input == std::path::Path::new("-") {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(args.input).map_err(Error::storage)?)
    };
    let mut bytes = Vec::new();
    source
        .take(u64::try_from(MAX_JSON_INPUT_BYTES + 1).map_err(Error::storage)?)
        .read_to_end(&mut bytes)
        .map_err(Error::storage)?;
    if bytes.len() > MAX_JSON_INPUT_BYTES {
        return Err(Error::input("input", "JSON input exceeds 1 MiB"));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(&bytes);
    let input = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| Error::input(&error.path().to_string(), error.inner().to_string()))?;
    deserializer
        .end()
        .map_err(|error| Error::input("input", error.to_string()))?;
    Ok(input)
}

fn encode(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(Envelope::new(value)).map_err(Error::storage)
}

fn schema(command: SchemaCommand) -> SchemaOutput {
    let (input, output) = match command {
        SchemaCommand::Init => (schema_for!(()), schema_for!(Envelope<InitOutput>)),
        SchemaCommand::Add | SchemaCommand::Capture => {
            (schema_for!(AddInput), schema_for!(Envelope<Ticket>))
        }
        SchemaCommand::Update => (schema_for!(UpdateInput), schema_for!(Envelope<Ticket>)),
        SchemaCommand::Note => (schema_for!(NoteInput), schema_for!(Envelope<Note>)),
        SchemaCommand::Notes => (
            schema_for!(NotesArgs),
            schema_for!(Envelope<Page<RecentNote>>),
        ),
        SchemaCommand::List => (schema_for!(ListArgs), schema_for!(Envelope<Page<Summary>>)),
        SchemaCommand::Repos => (
            schema_for!(ReposArgs),
            schema_for!(Envelope<Page<RepoSummary>>),
        ),
        SchemaCommand::Goals => (
            schema_for!(GoalsArgs),
            schema_for!(Envelope<Page<GoalSummary>>),
        ),
        SchemaCommand::Blockers => (
            schema_for!(BlockersArgs),
            schema_for!(Envelope<Page<Summary>>),
        ),
        SchemaCommand::Impact => (
            schema_for!(ImpactArgs),
            schema_for!(Envelope<Page<Summary>>),
        ),
        SchemaCommand::Bottlenecks => (
            schema_for!(BottlenecksArgs),
            schema_for!(Envelope<Page<BottleneckSummary>>),
        ),
        SchemaCommand::Show => (schema_for!(ShowArgs), schema_for!(Envelope<Detail>)),
        SchemaCommand::Search => (
            schema_for!(SearchArgs),
            schema_for!(Envelope<Page<Summary>>),
        ),
        SchemaCommand::Backup => (
            schema_for!(BackupArgs),
            schema_for!(Envelope<ArchiveOutput>),
        ),
        SchemaCommand::Restore => (
            schema_for!(RestoreArgs),
            schema_for!(Envelope<ArchiveOutput>),
        ),
    };
    SchemaOutput {
        command,
        input: input.to_value(),
        output: output.to_value(),
        error: schema_for!(ErrorEnvelope).to_value(),
    }
}
