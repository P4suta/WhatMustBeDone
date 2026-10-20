use crate::error::{Code, Error, Result};
use clap::{Args, ValueEnum};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use wmbd_core::{
    BottleneckRank, DependencyMark, DependencyNode, DependencyQuery, DependencyRelations,
    DependencyState, DescendantMark, Edge, ParentLink, ParentRelations, TicketId, WorkCounts,
    WorkPriority, WorkState,
};

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, ValueEnum,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Open,
    InProgress,
    Done,
    Cancelled,
}

impl Status {
    fn work_state(self) -> WorkState {
        match self {
            Self::Open => WorkState::Open,
            Self::InProgress => WorkState::InProgress,
            Self::Done => WorkState::Done,
            Self::Cancelled => WorkState::Cancelled,
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Serialize,
    JsonSchema,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    fn work_priority(self) -> WorkPriority {
        match self {
            Self::Low => WorkPriority::Low,
            Self::Normal => WorkPriority::Normal,
            Self::High => WorkPriority::High,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    #[schemars(range(min = 1))]
    pub id: u64,
    #[schemars(range(min = 1))]
    pub revision: u64,
    pub title: String,
    pub body: String,
    pub status: Status,
    pub repos: Vec<String>,
    pub priority: Priority,
    #[schemars(range(min = 1))]
    pub parent: Option<u64>,
    #[schemars(inner(range(min = 1)))]
    pub depends_on: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Note {
    #[schemars(range(min = 1))]
    pub id: u64,
    #[schemars(range(min = 1))]
    pub ticket_id: u64,
    pub body: String,
}

#[derive(Args, Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInput {
    pub title: String,
    #[arg(long, default_value = "")]
    #[serde(default)]
    pub body: String,
    #[arg(long = "repo", value_name = "REPO")]
    #[serde(default)]
    pub repos: Vec<String>,
    #[arg(long, value_enum, default_value_t = Priority::Normal)]
    #[serde(default)]
    pub priority: Priority,
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub parent: Option<u64>,
    #[arg(long, value_name = "ID", value_parser = clap::value_parser!(u64).range(1..))]
    #[serde(default)]
    #[schemars(inner(range(min = 1)))]
    pub depends_on: Vec<u64>,
    #[arg(long)]
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"required":["parent","clear_parent"],"properties":{"parent":{"not":{"type":"null"}},"clear_parent":{"const":true}}}))]
pub struct UpdateInput {
    pub revision: NonZeroU64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<Status>,
    pub repos: Option<Vec<String>>,
    pub priority: Option<Priority>,
    #[schemars(range(min = 1))]
    pub parent: Option<u64>,
    #[serde(default)]
    pub clear_parent: bool,
    #[schemars(inner(range(min = 1)))]
    pub depends_on: Option<Vec<u64>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    pub body: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Envelope<T> {
    pub api_version: u32,
    pub data: T,
}

impl<T> Envelope<T> {
    pub fn new(data: T) -> Self {
        Self {
            api_version: 1,
            data,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ErrorEnvelope {
    pub api_version: u32,
    pub error: Error,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InitOutput {
    pub schema_version: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArchiveOutput {
    pub schema_version: u32,
    pub tickets: u64,
    pub notes: u64,
    pub requests: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Summary {
    #[schemars(range(min = 1))]
    pub id: u64,
    #[schemars(range(min = 1))]
    pub revision: u64,
    pub title: String,
    pub status: Status,
    pub repos: Vec<String>,
    pub priority: Priority,
    #[schemars(range(min = 1))]
    pub parent: Option<u64>,
    #[schemars(inner(range(min = 1)))]
    pub blocked_by: Vec<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecentNote {
    pub note: Note,
    pub ticket: Summary,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RepoSummary {
    pub repo: Option<String>,
    #[serde(flatten)]
    pub work: WorkSummary,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GoalSummary {
    pub goal: Summary,
    pub work: WorkSummary,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BottleneckSummary {
    pub ticket: Summary,
    #[schemars(range(min = 1))]
    pub affected: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WorkSummary {
    pub total: u64,
    pub open: u64,
    pub in_progress: u64,
    pub done: u64,
    pub cancelled: u64,
    pub ready: u64,
    pub blocked: u64,
}

impl From<WorkCounts> for WorkSummary {
    fn from(counts: WorkCounts) -> Self {
        Self {
            total: counts.total,
            open: counts.open,
            in_progress: counts.in_progress,
            done: counts.done,
            cancelled: counts.cancelled,
            ready: counts.ready,
            blocked: counts.blocked,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

pub fn page<T>(items: Vec<T>, offset: usize, limit: usize) -> Page<T> {
    let total = items.len();
    let items: Vec<T> = items.into_iter().skip(offset).take(limit).collect();
    let end = offset.saturating_add(items.len());
    Page {
        items,
        total,
        next_offset: (end < total).then_some(end),
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Context {
    pub ancestors: Vec<Ticket>,
    pub dependencies: Vec<Ticket>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Detail {
    pub ticket: Ticket,
    pub notes: Page<Note>,
    pub parent: Option<Summary>,
    pub children: Vec<Summary>,
    pub dependencies: Vec<Summary>,
    pub dependents: Vec<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    pub tickets: Vec<Ticket>,
    pub notes: Vec<Note>,
}

struct DependencyRecords {
    nodes: Vec<DependencyNode>,
    edges: Vec<Edge>,
}

impl Ticket {
    pub fn validate(&self) -> Result<()> {
        checked_id(self.id, "id")?;
        if self.revision == 0 {
            return Err(Error::input("revision", "Revision must be positive"));
        }
        text(&self.title, "title")?;
        for repo in &self.repos {
            text(repo, "repos")?;
        }
        for id in &self.depends_on {
            checked_id(*id, "depends_on")?;
        }
        if let Some(id) = self.parent {
            checked_id(id, "parent")?;
        }
        Ok(())
    }
}

pub fn text(value: &str, path: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(Error::input(path, "Text must not be empty"))
    } else {
        Ok(())
    }
}

pub fn checked_id(value: u64, path: &str) -> Result<TicketId> {
    TicketId::new(value).ok_or_else(|| Error::input(path, "Ticket ID must be positive"))
}

impl Snapshot {
    fn done_ids(&self) -> BTreeSet<u64> {
        self.tickets
            .iter()
            .filter(|ticket| ticket.status == Status::Done)
            .map(|ticket| ticket.id)
            .collect()
    }

    pub fn repos(&self) -> Result<Vec<RepoSummary>> {
        let mut groups = BTreeMap::<Option<&str>, WorkCounts>::new();
        let done = self.done_ids();
        for ticket in &self.tickets {
            let state = ticket.status.work_state();
            let dependencies_done = ticket.depends_on.iter().all(|id| done.contains(id));
            let memberships: BTreeSet<_> = if ticket.repos.is_empty() {
                BTreeSet::from([None])
            } else {
                ticket
                    .repos
                    .iter()
                    .map(|repo| Some(repo.as_str()))
                    .collect()
            };
            for repo in memberships {
                let counts = groups.entry(repo).or_default();
                *counts = counts
                    .record(state, dependencies_done)
                    .ok_or_else(|| Error::new(Code::Exhausted, "Repository counts exhausted"))?;
            }
        }
        Ok(groups
            .into_iter()
            .map(|(repo, counts)| RepoSummary {
                repo: repo.map(str::to_owned),
                work: counts.into(),
            })
            .collect())
    }

    pub fn goals(&self) -> Result<Vec<GoalSummary>> {
        let nodes = self.parent_links()?;
        let relations = ParentRelations::new(&nodes)
            .map_err(|error| Error::corrupt(format!("Invalid parent scope: {error:?}")))?;
        let mut marks = vec![DescendantMark::Outside; nodes.len()];
        let done = self.done_ids();
        let mut items = Vec::new();
        for root in self.tickets.iter().filter(|ticket| ticket.parent.is_none()) {
            relations
                .mark_subtree(checked_id(root.id, "id")?, &mut marks)
                .map_err(|error| Error::corrupt(format!("Invalid parent scope: {error:?}")))?;
            let mut counts = WorkCounts::default();
            for (ticket, mark) in self.tickets.iter().zip(&marks) {
                if *mark == DescendantMark::Included {
                    counts = counts
                        .record(
                            ticket.status.work_state(),
                            ticket.depends_on.iter().all(|id| done.contains(id)),
                        )
                        .ok_or_else(|| Error::new(Code::Exhausted, "Goal counts exhausted"))?;
                }
            }
            items.push(GoalSummary {
                goal: self.summary(root),
                work: counts.into(),
            });
        }
        items.sort_by(|a, b| {
            b.goal
                .priority
                .cmp(&a.goal.priority)
                .then_with(|| a.goal.id.cmp(&b.goal.id))
        });
        Ok(items)
    }

    pub fn dependency_scope(&self, root: u64, query: DependencyQuery) -> Result<BTreeSet<u64>> {
        self.ticket(root)?;
        let records = self.dependency_records()?;
        let relations = DependencyRelations::new(&records.nodes, &records.edges)
            .map_err(|error| Error::corrupt(format!("Invalid dependency scope: {error:?}")))?;
        let mut marks = vec![DependencyMark::Outside; records.nodes.len()];
        relations
            .mark(query, checked_id(root, "id")?, &mut marks)
            .map_err(|error| Error::corrupt(format!("Invalid dependency scope: {error:?}")))?;
        Ok(records
            .nodes
            .iter()
            .zip(marks)
            .filter_map(|(node, mark)| (mark == DependencyMark::Included).then_some(node.id.get()))
            .collect())
    }

    pub fn bottlenecks(
        &self,
        ready: bool,
        consumer_scope: Option<&BTreeSet<u64>>,
        select: impl Fn(&Ticket) -> bool,
    ) -> Result<Vec<BottleneckSummary>> {
        let records = self.dependency_records()?;
        let relations = DependencyRelations::new(&records.nodes, &records.edges)
            .map_err(|error| Error::corrupt(format!("Invalid dependency scope: {error:?}")))?;
        let mut marks = vec![DependencyMark::Outside; records.nodes.len()];
        let done = self.done_ids();
        let mut items = Vec::new();
        for ticket in &self.tickets {
            if !select(ticket)
                || ticket.status == Status::Done
                || (ready
                    && !wmbd_core::ready(
                        ticket.status == Status::Open,
                        ticket.depends_on.iter().all(|id| done.contains(id)),
                    ))
            {
                continue;
            }
            let id = checked_id(ticket.id, "id")?;
            relations
                .mark(DependencyQuery::Impact, id, &mut marks)
                .map_err(|error| Error::corrupt(format!("Invalid dependency scope: {error:?}")))?;
            let affected = u64::try_from(
                records
                    .nodes
                    .iter()
                    .zip(&marks)
                    .filter(|(node, mark)| {
                        mark.included_in(
                            consumer_scope.is_none_or(|scope| scope.contains(&node.id.get())),
                        )
                    })
                    .count(),
            )
            .map_err(|_| Error::new(Code::Exhausted, "Affected work count exhausted"))?;
            if let Some(rank) = BottleneckRank::new(affected, ticket.priority.work_priority(), id) {
                items.push((
                    rank,
                    BottleneckSummary {
                        ticket: self.summary(ticket),
                        affected,
                    },
                ));
            }
        }
        items.sort_by_key(|(rank, _)| *rank);
        Ok(items.into_iter().map(|(_, summary)| summary).collect())
    }

    fn dependency_records(&self) -> Result<DependencyRecords> {
        let nodes = self
            .tickets
            .iter()
            .map(|ticket| {
                Ok(DependencyNode {
                    id: checked_id(ticket.id, "id")?,
                    state: match ticket.status {
                        Status::Open | Status::InProgress => DependencyState::Active,
                        Status::Done => DependencyState::Done,
                        Status::Cancelled => DependencyState::Cancelled,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut edges = Vec::new();
        for ticket in &self.tickets {
            for dependency in &ticket.depends_on {
                edges.push(Edge {
                    from: checked_id(ticket.id, "id")?,
                    to: checked_id(*dependency, "depends_on")?,
                });
            }
        }
        Ok(DependencyRecords { nodes, edges })
    }

    pub fn descendants(&self, root: u64) -> Result<BTreeSet<u64>> {
        self.ticket(root)?;
        let nodes = self.parent_links()?;
        let mut marks = vec![DescendantMark::Outside; nodes.len()];
        wmbd_core::mark_descendants(checked_id(root, "under")?, &nodes, &mut marks)
            .map_err(|error| Error::corrupt(format!("Invalid parent scope: {error:?}")))?;
        Ok(nodes
            .iter()
            .zip(marks)
            .filter_map(|(node, mark)| (mark == DescendantMark::Included).then_some(node.id.get()))
            .collect())
    }

    pub fn subtree(&self, root: u64) -> Result<BTreeSet<u64>> {
        self.ticket(root)?;
        let nodes = self.parent_links()?;
        let mut marks = vec![DescendantMark::Outside; nodes.len()];
        wmbd_core::mark_subtree(checked_id(root, "for_goal")?, &nodes, &mut marks)
            .map_err(|error| Error::corrupt(format!("Invalid parent scope: {error:?}")))?;
        Ok(nodes
            .iter()
            .zip(marks)
            .filter_map(|(node, mark)| (mark == DescendantMark::Included).then_some(node.id.get()))
            .collect())
    }

    fn parent_links(&self) -> Result<Vec<ParentLink>> {
        self.tickets
            .iter()
            .map(|ticket| {
                Ok(ParentLink {
                    id: checked_id(ticket.id, "id")?,
                    parent: ticket
                        .parent
                        .map(|id| checked_id(id, "parent"))
                        .transpose()?,
                })
            })
            .collect()
    }

    pub fn ticket(&self, id: u64) -> Result<&Ticket> {
        self.tickets
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| Error::new(Code::NotFound, format!("Ticket {id} does not exist")))
    }

    pub fn validate(&self) -> Result<()> {
        let nodes = self
            .tickets
            .iter()
            .map(|t| checked_id(t.id, "id"))
            .collect::<Result<Vec<_>>>()?;
        let mut parents = Vec::new();
        let mut dependencies = Vec::new();
        for ticket in &self.tickets {
            ticket.validate()?;
            let from = checked_id(ticket.id, "id")?;
            if let Some(parent) = ticket.parent {
                parents.push(Edge {
                    from,
                    to: checked_id(parent, "parent")?,
                });
            }
            for id in &ticket.depends_on {
                dependencies.push(Edge {
                    from,
                    to: checked_id(*id, "depends_on")?,
                });
            }
        }
        for (path, edges) in [("parent", parents), ("depends_on", dependencies)] {
            let mut marks = vec![wmbd_core::GraphMark::Pending(0); nodes.len()];
            wmbd_core::validate_graph(&nodes, &edges, &mut marks)
                .map_err(|error| Error::input(path, format!("Invalid relation: {error:?}")))?;
        }
        for note in &self.notes {
            checked_id(note.id, "note.id")?;
            self.ticket(note.ticket_id)?;
            text(&note.body, "note.body")?;
        }
        Ok(())
    }

    pub fn summary(&self, ticket: &Ticket) -> Summary {
        Summary {
            id: ticket.id,
            revision: ticket.revision,
            title: ticket.title.clone(),
            status: ticket.status,
            repos: ticket.repos.clone(),
            priority: ticket.priority,
            parent: ticket.parent,
            blocked_by: ticket
                .depends_on
                .iter()
                .copied()
                .filter(|id| {
                    !self
                        .tickets
                        .iter()
                        .any(|t| t.id == *id && t.status == Status::Done)
                })
                .collect(),
        }
    }

    pub fn recent_notes(&self, offset: usize, limit: usize) -> Result<Page<RecentNote>> {
        let notes = page(self.notes.iter().rev().collect(), offset, limit);
        let items = notes
            .items
            .into_iter()
            .map(|note| {
                Ok(RecentNote {
                    note: note.clone(),
                    ticket: self.summary(self.ticket(note.ticket_id)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Page {
            items,
            total: notes.total,
            next_offset: notes.next_offset,
        })
    }

    pub fn detail(
        &self,
        id: u64,
        context: bool,
        offset: usize,
        limit: usize,
        notes_query: Option<&str>,
    ) -> Result<Detail> {
        let ticket = self.ticket(id)?.clone();
        let mut ancestors = Vec::new();
        let mut next = ticket.parent;
        while let Some(id) = next {
            let ancestor = self.ticket(id)?;
            next = ancestor.parent;
            ancestors.push(ancestor.clone());
        }
        ancestors.reverse();
        let dependencies = ticket
            .depends_on
            .iter()
            .map(|id| self.ticket(*id).cloned())
            .collect::<Result<Vec<_>>>()?;
        let mut notes: Vec<_> = self
            .notes
            .iter()
            .filter(|n| n.ticket_id == id && notes_query.is_none_or(|query| n.body.contains(query)))
            .cloned()
            .collect();
        notes.reverse();
        Ok(Detail {
            parent: ticket
                .parent
                .map(|id| self.ticket(id).map(|t| self.summary(t)))
                .transpose()?,
            children: self
                .tickets
                .iter()
                .filter(|t| t.parent == Some(id))
                .map(|t| self.summary(t))
                .collect(),
            dependencies: dependencies.iter().map(|t| self.summary(t)).collect(),
            dependents: self
                .tickets
                .iter()
                .filter(|t| t.depends_on.contains(&id))
                .map(|t| self.summary(t))
                .collect(),
            context: context.then_some(Context {
                ancestors,
                dependencies,
            }),
            ticket,
            notes: page(notes, offset, limit),
        })
    }
}
