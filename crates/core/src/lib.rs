#![no_std]
#![deny(clippy::panic, clippy::indexing_slicing)]

#[cfg(test)]
extern crate alloc;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TicketId(u64);

impl TicketId {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkPriority {
    Low,
    Normal,
    High,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BottleneckRank {
    affected: core::cmp::Reverse<core::num::NonZeroU64>,
    priority: core::cmp::Reverse<WorkPriority>,
    id: TicketId,
}

impl BottleneckRank {
    pub fn new(affected: u64, priority: WorkPriority, id: TicketId) -> Option<Self> {
        Some(Self {
            affected: core::cmp::Reverse(core::num::NonZeroU64::new(affected)?),
            priority: core::cmp::Reverse(priority),
            id,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Edge {
    pub from: TicketId,
    pub to: TicketId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelationError {
    MissingEndpoint,
    SelfReference,
    Cycle,
    InvalidWorkspace,
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphMark {
    Pending(usize),
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParentLink {
    pub id: TicketId,
    pub parent: Option<TicketId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescendantMark {
    Outside,
    Pending,
    Included,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyState {
    Active,
    Done,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencyNode {
    pub id: TicketId,
    pub state: DependencyState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyMark {
    Outside,
    Pending,
    Included,
}

impl DependencyMark {
    pub fn included_in(self, member: bool) -> bool {
        self == Self::Included && member
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyQuery {
    Blockers,
    Impact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkState {
    Open,
    InProgress,
    Done,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkCounts {
    pub total: u64,
    pub open: u64,
    pub in_progress: u64,
    pub done: u64,
    pub cancelled: u64,
    pub ready: u64,
    pub blocked: u64,
}

impl WorkCounts {
    pub fn record(self, state: WorkState, dependencies_done: bool) -> Option<Self> {
        let open = state == WorkState::Open;
        let active = matches!(state, WorkState::Open | WorkState::InProgress);
        Some(Self {
            total: self.total.checked_add(1)?,
            open: self.open.checked_add(u64::from(open))?,
            in_progress: self
                .in_progress
                .checked_add(u64::from(state == WorkState::InProgress))?,
            done: self.done.checked_add(u64::from(state == WorkState::Done))?,
            cancelled: self
                .cancelled
                .checked_add(u64::from(state == WorkState::Cancelled))?,
            ready: self
                .ready
                .checked_add(u64::from(ready(open, dependencies_done)))?,
            blocked: self
                .blocked
                .checked_add(u64::from(active && !dependencies_done))?,
        })
    }
}

#[derive(Debug)]
pub struct DependencyRelations<'a> {
    nodes: &'a [DependencyNode],
    edges: &'a [Edge],
}

impl<'a> DependencyRelations<'a> {
    pub fn new(nodes: &'a [DependencyNode], edges: &'a [Edge]) -> Result<Self, RelationError> {
        Self::validate(nodes, edges, None)
    }

    fn validate(
        nodes: &'a [DependencyNode],
        edges: &'a [Edge],
        root: Option<TicketId>,
    ) -> Result<Self, RelationError> {
        if nodes
            .iter()
            .any(|node| nodes.iter().filter(|other| other.id == node.id).count() != 1)
        {
            return Err(RelationError::InvalidWorkspace);
        }
        if root.is_some_and(|root| !nodes.iter().any(|node| node.id == root)) {
            return Err(RelationError::MissingEndpoint);
        }
        for edge in edges {
            if !nodes.iter().any(|node| node.id == edge.from)
                || !nodes.iter().any(|node| node.id == edge.to)
            {
                return Err(RelationError::MissingEndpoint);
            }
            if edge.from == edge.to {
                return Err(RelationError::SelfReference);
            }
        }
        Ok(Self { nodes, edges })
    }

    pub fn mark(
        &self,
        query: DependencyQuery,
        root: TicketId,
        marks: &mut [DependencyMark],
    ) -> Result<(), RelationError> {
        let nodes = self.nodes;
        let edges = self.edges;
        if marks.len() != nodes.len() {
            return Err(RelationError::InvalidWorkspace);
        }
        let root = nodes
            .iter()
            .find(|node| node.id == root)
            .ok_or(RelationError::MissingEndpoint)?;
        marks.fill(DependencyMark::Outside);
        let empty = match query {
            DependencyQuery::Blockers => root.state != DependencyState::Active,
            DependencyQuery::Impact => root.state == DependencyState::Done,
        };
        if empty {
            return Ok(());
        }
        for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
            if node.id == root.id {
                *mark = DependencyMark::Pending;
            }
        }
        while let Some(next) = nodes
            .iter()
            .zip(marks.iter())
            .find_map(|(node, mark)| (*mark == DependencyMark::Pending).then_some(*node))
        {
            for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
                if node.id == next.id {
                    *mark = DependencyMark::Included;
                }
            }
            let expand = match query {
                DependencyQuery::Blockers => next.state == DependencyState::Active,
                DependencyQuery::Impact => true,
            };
            if expand {
                for edge in edges {
                    let candidate = match query {
                        DependencyQuery::Blockers if edge.from == next.id => edge.to,
                        DependencyQuery::Impact if edge.to == next.id => edge.from,
                        DependencyQuery::Blockers | DependencyQuery::Impact => continue,
                    };
                    for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
                        let include = match query {
                            DependencyQuery::Blockers => node.state != DependencyState::Done,
                            DependencyQuery::Impact => node.state == DependencyState::Active,
                        };
                        if node.id == candidate && include && *mark == DependencyMark::Outside {
                            *mark = DependencyMark::Pending;
                        }
                    }
                }
            }
        }
        for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
            if node.id == root.id {
                *mark = DependencyMark::Outside;
            }
        }
        Ok(())
    }
}

pub fn mark_dependencies(
    query: DependencyQuery,
    root: TicketId,
    nodes: &[DependencyNode],
    edges: &[Edge],
    marks: &mut [DependencyMark],
) -> Result<(), RelationError> {
    if marks.len() != nodes.len() {
        return Err(RelationError::InvalidWorkspace);
    }
    DependencyRelations::validate(nodes, edges, Some(root))?.mark(query, root, marks)
}

#[derive(Debug)]
pub struct ParentRelations<'a> {
    nodes: &'a [ParentLink],
}

impl<'a> ParentRelations<'a> {
    pub fn new(nodes: &'a [ParentLink]) -> Result<Self, RelationError> {
        Self::validate(nodes, None)
    }

    fn validate(nodes: &'a [ParentLink], root: Option<TicketId>) -> Result<Self, RelationError> {
        if nodes
            .iter()
            .any(|node| nodes.iter().filter(|other| other.id == node.id).count() != 1)
        {
            return Err(RelationError::InvalidWorkspace);
        }
        if root.is_some_and(|root| !nodes.iter().any(|node| node.id == root)) {
            return Err(RelationError::MissingEndpoint);
        }
        for node in nodes {
            if let Some(parent) = node.parent {
                if parent == node.id {
                    return Err(RelationError::SelfReference);
                }
                if !nodes.iter().any(|node| node.id == parent) {
                    return Err(RelationError::MissingEndpoint);
                }
            }
        }
        Ok(Self { nodes })
    }

    pub fn mark_subtree(
        &self,
        root: TicketId,
        marks: &mut [DescendantMark],
    ) -> Result<(), RelationError> {
        let nodes = self.nodes;
        if marks.len() != nodes.len() {
            return Err(RelationError::InvalidWorkspace);
        }
        if !nodes.iter().any(|node| node.id == root) {
            return Err(RelationError::MissingEndpoint);
        }
        marks.fill(DescendantMark::Outside);
        for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
            if node.id == root {
                *mark = DescendantMark::Pending;
            }
        }
        while let Some(next) = nodes
            .iter()
            .zip(marks.iter())
            .find_map(|(node, mark)| (*mark == DescendantMark::Pending).then_some(node.id))
        {
            for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
                if node.id == next {
                    *mark = DescendantMark::Included;
                } else if node.parent == Some(next) && *mark == DescendantMark::Outside {
                    *mark = DescendantMark::Pending;
                }
            }
        }
        Ok(())
    }
}

pub fn mark_subtree(
    root: TicketId,
    nodes: &[ParentLink],
    marks: &mut [DescendantMark],
) -> Result<(), RelationError> {
    if marks.len() != nodes.len() {
        return Err(RelationError::InvalidWorkspace);
    }
    ParentRelations::validate(nodes, Some(root))?.mark_subtree(root, marks)
}

pub fn mark_descendants(
    root: TicketId,
    nodes: &[ParentLink],
    marks: &mut [DescendantMark],
) -> Result<(), RelationError> {
    mark_subtree(root, nodes, marks)?;
    for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
        if node.id == root {
            *mark = DescendantMark::Outside;
        }
    }
    Ok(())
}

pub fn ready(open: bool, dependencies_done: bool) -> bool {
    open && dependencies_done
}

pub const fn revision_matches(expected: u64, current: u64) -> bool {
    expected != 0 && expected == current
}

#[derive(Debug, Eq, PartialEq)]
pub struct MatchedRevision(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionChange {
    Unchanged,
    Changed(u64),
}

impl MatchedRevision {
    pub const fn new(expected: u64, current: u64) -> Option<Self> {
        if revision_matches(expected, current) {
            Some(Self(current))
        } else {
            None
        }
    }

    pub const fn update(self, changed: bool) -> Option<RevisionChange> {
        if !changed {
            Some(RevisionChange::Unchanged)
        } else {
            match next_sequence(self.0) {
                Some(revision) => Some(RevisionChange::Changed(revision)),
                None => None,
            }
        }
    }
}

pub const fn next_sequence(current: u64) -> Option<u64> {
    current.checked_add(1)
}

#[derive(Clone, Copy, Debug)]
pub struct ProofCounts {
    pub expected: usize,
    pub total: usize,
    pub executed: usize,
    pub successful: usize,
    pub failed: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct CounterexampleEvidence {
    pub counts: ProofCounts,
    pub process_failed: bool,
    pub report_completed: bool,
    pub harness_matches: bool,
    pub assertion_failed: bool,
    pub checks_valid: bool,
    pub error_is_assertion: bool,
}

impl CounterexampleEvidence {
    pub const fn accepted(self) -> bool {
        self.counts.expected == 1
            && self.counts.total == 1
            && self.counts.executed == 1
            && self.counts.successful == 0
            && self.counts.failed == 1
            && self.process_failed
            && self.report_completed
            && self.harness_matches
            && self.assertion_failed
            && self.checks_valid
            && self.error_is_assertion
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AuditCoverage {
    pub advisories: u64,
    pub dependencies: u64,
    pub vulnerabilities: u64,
    pub settings_complete: bool,
    pub findings_empty: bool,
}

impl AuditCoverage {
    pub const fn accepted(self) -> bool {
        self.advisories > 0
            && self.dependencies > 0
            && self.vulnerabilities == 0
            && self.settings_complete
            && self.findings_empty
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofCheckResult {
    Success,
    Satisfied,
    Unreachable,
    Invalid,
    Failure,
    Unsatisfiable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofCheckKind {
    Assertion,
    Cover,
    Other,
}

impl ProofCheckResult {
    pub const fn accepted(self, cover: bool) -> bool {
        match self {
            Self::Satisfied => true,
            Self::Success | Self::Unreachable => !cover,
            Self::Invalid | Self::Failure | Self::Unsatisfiable => false,
        }
    }

    pub const fn supports_counterexample(self, kind: ProofCheckKind) -> bool {
        match kind {
            ProofCheckKind::Assertion => match self {
                Self::Success | Self::Unreachable | Self::Failure => true,
                Self::Satisfied | Self::Invalid | Self::Unsatisfiable => false,
            },
            ProofCheckKind::Cover => match self {
                Self::Satisfied | Self::Unreachable | Self::Unsatisfiable => true,
                Self::Success | Self::Invalid | Self::Failure => false,
            },
            ProofCheckKind::Other => match self {
                Self::Success | Self::Unreachable => true,
                Self::Satisfied | Self::Invalid | Self::Failure | Self::Unsatisfiable => false,
            },
        }
    }
}

impl ProofCounts {
    pub const fn complete(self) -> bool {
        self.expected > 0
            && self.total == self.expected
            && self.executed == self.expected
            && self.successful == self.expected
            && self.failed == 0
    }
}

pub fn validate_graph(
    nodes: &[TicketId],
    edges: &[Edge],
    marks: &mut [GraphMark],
) -> Result<(), RelationError> {
    if marks.len() != nodes.len() {
        return Err(RelationError::InvalidWorkspace);
    }
    marks.fill(GraphMark::Pending(0));
    for edge in edges {
        if !nodes.contains(&edge.from) || !nodes.contains(&edge.to) {
            return Err(RelationError::MissingEndpoint);
        }
        if edge.from == edge.to {
            return Err(RelationError::SelfReference);
        }
        for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
            if *node == edge.to
                && let GraphMark::Pending(degree) = mark
            {
                *degree = degree.checked_add(1).ok_or(RelationError::Overflow)?;
            }
        }
    }
    loop {
        let next = nodes
            .iter()
            .zip(marks.iter())
            .find_map(|(node, mark)| (*mark == GraphMark::Pending(0)).then_some(*node));
        let Some(next) = next else {
            return if marks.iter().all(|m| *m == GraphMark::Removed) {
                Ok(())
            } else {
                Err(RelationError::Cycle)
            };
        };
        for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
            if *node == next {
                *mark = GraphMark::Removed;
            }
        }
        for edge in edges.iter().filter(|e| e.from == next) {
            for (node, mark) in nodes.iter().zip(marks.iter_mut()) {
                if *node == edge.to
                    && let GraphMark::Pending(degree) = mark
                {
                    *degree = degree
                        .checked_sub(1)
                        .ok_or(RelationError::InvalidWorkspace)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counterexamples_require_one_attributed_assertion_failure_without_tool_errors() {
        use super::*;
        let valid = CounterexampleEvidence {
            counts: ProofCounts {
                expected: 1,
                total: 1,
                executed: 1,
                successful: 0,
                failed: 1,
            },
            process_failed: true,
            report_completed: true,
            harness_matches: true,
            assertion_failed: true,
            checks_valid: true,
            error_is_assertion: true,
        };
        assert!(valid.accepted());
        for counts in [
            ProofCounts {
                expected: 0,
                ..valid.counts
            },
            ProofCounts {
                total: 0,
                ..valid.counts
            },
            ProofCounts {
                executed: 0,
                ..valid.counts
            },
            ProofCounts {
                successful: 1,
                ..valid.counts
            },
            ProofCounts {
                failed: 0,
                ..valid.counts
            },
            ProofCounts {
                expected: usize::MAX,
                total: usize::MAX,
                executed: usize::MAX,
                successful: 0,
                failed: usize::MAX,
            },
        ] {
            assert!(!CounterexampleEvidence { counts, ..valid }.accepted());
        }
        for refused in [
            CounterexampleEvidence {
                process_failed: false,
                ..valid
            },
            CounterexampleEvidence {
                report_completed: false,
                ..valid
            },
            CounterexampleEvidence {
                harness_matches: false,
                ..valid
            },
            CounterexampleEvidence {
                assertion_failed: false,
                ..valid
            },
            CounterexampleEvidence {
                checks_valid: false,
                ..valid
            },
            CounterexampleEvidence {
                error_is_assertion: false,
                ..valid
            },
        ] {
            assert!(!refused.accepted(), "{refused:?}");
        }
    }

    #[test]
    fn counterexample_checks_match_their_exact_category() {
        use super::{ProofCheckKind::*, ProofCheckResult::*};
        for (result, expected, positive) in [
            (Success, [true, false, true], [true, false]),
            (Satisfied, [false, true, false], [true, true]),
            (Unreachable, [true, true, true], [true, false]),
            (Invalid, [false, false, false], [false, false]),
            (Failure, [true, false, false], [false, false]),
            (Unsatisfiable, [false, true, false], [false, false]),
        ] {
            for (kind, expected) in [Assertion, Cover, Other].into_iter().zip(expected) {
                assert_eq!(
                    result.supports_counterexample(kind),
                    expected,
                    "{result:?} / {kind:?}"
                );
            }
            for (cover, expected) in [false, true].into_iter().zip(positive) {
                assert_eq!(result.accepted(cover), expected);
            }
        }
    }

    #[test]
    fn audit_coverage_rejects_empty_filtered_and_unclean_reports() {
        let clean = super::AuditCoverage {
            advisories: 1,
            dependencies: 1,
            vulnerabilities: 0,
            settings_complete: true,
            findings_empty: true,
        };
        assert!(clean.accepted());
        for refused in [
            super::AuditCoverage {
                advisories: 0,
                ..clean
            },
            super::AuditCoverage {
                dependencies: 0,
                ..clean
            },
            super::AuditCoverage {
                vulnerabilities: 1,
                ..clean
            },
            super::AuditCoverage {
                settings_complete: false,
                ..clean
            },
            super::AuditCoverage {
                findings_empty: false,
                ..clean
            },
        ] {
            assert!(!refused.accepted(), "{refused:?}");
        }
    }

    #[test]
    fn scoped_dependencies_require_both_selection_and_membership() {
        use super::DependencyMark;
        for (mark, expected) in [
            (DependencyMark::Outside, [false, false]),
            (DependencyMark::Pending, [false, false]),
            (DependencyMark::Included, [false, true]),
        ] {
            for (member, expected) in [false, true].into_iter().zip(expected) {
                assert_eq!(
                    mark.included_in(member),
                    expected,
                    "{mark:?}, member={member}"
                );
            }
        }
    }

    #[test]
    fn bottlenecks_rank_affected_work_before_priority_and_id() {
        use super::*;
        let candidates = [
            (0, WorkPriority::High, TicketId(1)),
            (1, WorkPriority::High, TicketId(2)),
            (u64::MAX, WorkPriority::Low, TicketId(3)),
            (1, WorkPriority::Normal, TicketId(4)),
            (1, WorkPriority::Normal, TicketId(1)),
        ];
        let mut ranks: alloc::vec::Vec<_> = candidates
            .into_iter()
            .filter_map(|(affected, priority, id)| BottleneckRank::new(affected, priority, id))
            .collect();
        ranks.sort();
        assert_eq!(
            ranks
                .into_iter()
                .map(|rank| rank.id.get())
                .collect::<alloc::vec::Vec<_>>(),
            [3, 2, 1, 4]
        );
    }

    #[test]
    fn dependency_queries_match_active_edge_transitive_closure() {
        use super::*;
        let ids = [TicketId(1), TicketId(2), TicketId(3)];
        for states in 0..27_u8 {
            let mut encoded = states;
            let mut nodes = alloc::vec::Vec::new();
            for id in ids {
                let state = match encoded % 3 {
                    0 => DependencyState::Active,
                    1 => DependencyState::Done,
                    _ => DependencyState::Cancelled,
                };
                encoded /= 3;
                nodes.push(DependencyNode { id, state });
            }
            for graph in 0..64_u8 {
                let mut bit = 0;
                let mut edges = alloc::vec::Vec::new();
                let mut closure = alloc::collections::BTreeSet::new();
                for from in &nodes {
                    for to in nodes.iter().filter(|to| to.id != from.id) {
                        if graph & (1 << bit) != 0 {
                            edges.push(Edge {
                                from: from.id,
                                to: to.id,
                            });
                            if from.state == DependencyState::Active
                                && to.state != DependencyState::Done
                            {
                                closure.insert((from.id, to.id));
                            }
                        }
                        bit += 1;
                    }
                }
                for pivot in ids {
                    for from in ids {
                        for to in ids {
                            if closure.contains(&(from, pivot)) && closure.contains(&(pivot, to)) {
                                closure.insert((from, to));
                            }
                        }
                    }
                }
                let relations = DependencyRelations::new(&nodes, &edges);
                assert!(relations.is_ok());
                if let Ok(relations) = relations {
                    for root in &nodes {
                        for query in [DependencyQuery::Blockers, DependencyQuery::Impact] {
                            let mut marks = [DependencyMark::Included; 3];
                            assert_eq!(relations.mark(query, root.id, &mut marks), Ok(()));
                            for (node, mark) in nodes.iter().zip(marks) {
                                let expected = node.id != root.id
                                    && match query {
                                        DependencyQuery::Blockers => {
                                            root.state == DependencyState::Active
                                                && closure.contains(&(root.id, node.id))
                                        }
                                        DependencyQuery::Impact => {
                                            node.state == DependencyState::Active
                                                && closure.contains(&(node.id, root.id))
                                        }
                                    };
                                assert_eq!(
                                    mark == DependencyMark::Included,
                                    expected,
                                    "query {query:?}, states {states}, graph {graph}, root {root:?}, node {node:?}"
                                );
                                assert_ne!(mark, DependencyMark::Pending);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn checked_dependency_relations_reject_invalid_inputs_and_reuse_marks() {
        use super::*;
        let nodes = [
            DependencyNode {
                id: TicketId(1),
                state: DependencyState::Active,
            },
            DependencyNode {
                id: TicketId(2),
                state: DependencyState::Active,
            },
        ];
        let edges = [Edge {
            from: TicketId(2),
            to: TicketId(1),
        }];
        assert!(matches!(
            DependencyRelations::new(&[nodes[0], nodes[0]], &[]),
            Err(RelationError::InvalidWorkspace)
        ));
        assert!(matches!(
            DependencyRelations::new(&nodes[..1], &edges),
            Err(RelationError::MissingEndpoint)
        ));
        assert!(matches!(
            DependencyRelations::new(
                &nodes,
                &[Edge {
                    from: TicketId(1),
                    to: TicketId(1)
                }]
            ),
            Err(RelationError::SelfReference)
        ));
        let relations = DependencyRelations::new(&nodes, &edges);
        assert!(relations.is_ok());
        if let Ok(relations) = relations {
            let mut marks = [DependencyMark::Pending; 2];
            assert_eq!(
                relations.mark(DependencyQuery::Impact, TicketId(1), &mut []),
                Err(RelationError::InvalidWorkspace)
            );
            assert_eq!(
                relations.mark(DependencyQuery::Impact, TicketId(3), &mut marks),
                Err(RelationError::MissingEndpoint)
            );
            assert_eq!(marks, [DependencyMark::Pending; 2]);
            assert_eq!(
                relations.mark(DependencyQuery::Impact, TicketId(1), &mut marks),
                Ok(())
            );
            assert_eq!(marks, [DependencyMark::Outside, DependencyMark::Included]);
            assert_eq!(
                relations.mark(DependencyQuery::Blockers, TicketId(2), &mut marks),
                Ok(())
            );
            assert_eq!(marks, [DependencyMark::Included, DependencyMark::Outside]);
            assert_eq!(
                relations.mark(DependencyQuery::Impact, TicketId(2), &mut marks),
                Ok(())
            );
            assert_eq!(marks, [DependencyMark::Outside; 2]);
        }
        assert_eq!(
            mark_dependencies(
                DependencyQuery::Impact,
                TicketId(3),
                &[nodes[0], nodes[0]],
                &[],
                &mut [DependencyMark::Outside; 2]
            ),
            Err(RelationError::InvalidWorkspace)
        );
        assert_eq!(
            mark_dependencies(
                DependencyQuery::Impact,
                TicketId(3),
                &nodes,
                &[Edge {
                    from: TicketId(1),
                    to: TicketId(1)
                }],
                &mut [DependencyMark::Outside; 2]
            ),
            Err(RelationError::MissingEndpoint)
        );
    }

    #[test]
    fn dependency_queries_check_graph_boundaries_and_long_reverse_chains() {
        use super::*;
        let nodes = (1..=8)
            .map(|id| DependencyNode {
                id: TicketId(id),
                state: DependencyState::Active,
            })
            .collect::<alloc::vec::Vec<_>>();
        let edges = (2..=8)
            .map(|id| Edge {
                from: TicketId(id),
                to: TicketId(id - 1),
            })
            .collect::<alloc::vec::Vec<_>>();
        for query in [DependencyQuery::Blockers, DependencyQuery::Impact] {
            let (far, leaf) = match query {
                DependencyQuery::Blockers => (TicketId(8), TicketId(1)),
                DependencyQuery::Impact => (TicketId(1), TicketId(8)),
            };
            let mut marks = [DependencyMark::Included; 8];
            assert_eq!(
                mark_dependencies(query, far, &nodes, &edges, &mut marks),
                Ok(())
            );
            assert_eq!(
                marks
                    .iter()
                    .filter(|mark| **mark == DependencyMark::Included)
                    .count(),
                7
            );
            assert_eq!(
                mark_dependencies(query, leaf, &nodes, &edges, &mut marks),
                Ok(())
            );
            assert!(marks.iter().all(|mark| *mark == DependencyMark::Outside));
            assert_eq!(
                mark_dependencies(query, far, &nodes, &edges, &mut []),
                Err(RelationError::InvalidWorkspace)
            );
            assert_eq!(
                mark_dependencies(query, TicketId(9), &nodes, &edges, &mut marks),
                Err(RelationError::MissingEndpoint)
            );
            for (edge, error) in [
                (
                    Edge {
                        from: TicketId(9),
                        to: TicketId(1),
                    },
                    RelationError::MissingEndpoint,
                ),
                (
                    Edge {
                        from: TicketId(1),
                        to: TicketId(9),
                    },
                    RelationError::MissingEndpoint,
                ),
                (
                    Edge {
                        from: TicketId(1),
                        to: TicketId(1),
                    },
                    RelationError::SelfReference,
                ),
            ] {
                assert_eq!(
                    mark_dependencies(query, TicketId(1), &nodes, &[edge], &mut marks),
                    Err(error)
                );
            }
            let duplicate = DependencyNode {
                id: TicketId(1),
                state: DependencyState::Active,
            };
            assert_eq!(
                mark_dependencies(
                    query,
                    TicketId(1),
                    &[duplicate, duplicate],
                    &[],
                    &mut [DependencyMark::Outside; 2]
                ),
                Err(RelationError::InvalidWorkspace)
            );
        }
    }

    #[test]
    fn every_three_node_parent_map_matches_reachability() {
        use super::*;
        let ids = [TicketId(1), TicketId(2), TicketId(3)];
        for encoding in 0..64_u8 {
            let mut value = encoding;
            let mut nodes = alloc::vec::Vec::new();
            let mut closure = alloc::collections::BTreeSet::new();
            for id in ids {
                let parent = match value % 4 {
                    0 => None,
                    parent => Some(TicketId(u64::from(parent))),
                };
                value /= 4;
                nodes.push(ParentLink { id, parent });
                if let Some(parent) = parent {
                    closure.insert((parent, id));
                }
            }
            for pivot in ids {
                for from in ids {
                    for to in ids {
                        if closure.contains(&(from, pivot)) && closure.contains(&(pivot, to)) {
                            closure.insert((from, to));
                        }
                    }
                }
            }
            let relations = ParentRelations::new(&nodes);
            for root in ids {
                for include_root in [false, true] {
                    let mut marks = [DescendantMark::Included; 3];
                    let result = if include_root {
                        relations
                            .as_ref()
                            .map_err(|error| *error)
                            .and_then(|relations| relations.mark_subtree(root, &mut marks))
                    } else {
                        mark_descendants(root, &nodes, &mut marks)
                    };
                    if nodes.iter().any(|node| node.parent == Some(node.id)) {
                        assert_eq!(result, Err(RelationError::SelfReference));
                    } else {
                        assert_eq!(result, Ok(()));
                        for (node, mark) in nodes.iter().zip(marks) {
                            assert_eq!(
                                mark == DescendantMark::Included,
                                (include_root && node.id == root)
                                    || (node.id != root && closure.contains(&(root, node.id))),
                                "parent map {encoding}, root {root:?}, node {node:?}"
                            );
                            assert_ne!(mark, DescendantMark::Pending);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn checked_parent_relations_require_endpoints_and_matching_workspaces() {
        use super::*;
        let root = ParentLink {
            id: TicketId(2),
            parent: None,
        };
        let child = ParentLink {
            id: TicketId(1),
            parent: Some(root.id),
        };
        assert!(matches!(
            ParentRelations::new(&[root, root]),
            Err(RelationError::InvalidWorkspace)
        ));
        assert!(matches!(
            ParentRelations::new(&[child]),
            Err(RelationError::MissingEndpoint)
        ));
        assert!(matches!(
            ParentRelations::new(&[ParentLink {
                parent: Some(root.id),
                ..root
            }]),
            Err(RelationError::SelfReference)
        ));
        let nodes = [child, root];
        let relations = ParentRelations::new(&nodes);
        assert!(relations.is_ok());
        if let Ok(relations) = relations {
            let mut marks = [DescendantMark::Pending; 2];
            assert_eq!(
                relations.mark_subtree(root.id, &mut []),
                Err(RelationError::InvalidWorkspace)
            );
            assert_eq!(
                relations.mark_subtree(TicketId(3), &mut marks),
                Err(RelationError::MissingEndpoint)
            );
            assert_eq!(marks, [DescendantMark::Pending; 2]);
            assert_eq!(relations.mark_subtree(root.id, &mut marks), Ok(()));
            assert_eq!(marks, [DescendantMark::Included; 2]);
            assert_eq!(relations.mark_subtree(child.id, &mut marks), Ok(()));
            assert_eq!(marks, [DescendantMark::Included, DescendantMark::Outside]);
        }
        assert_eq!(
            mark_subtree(
                TicketId(3),
                &[root, root],
                &mut [DescendantMark::Outside; 2]
            ),
            Err(RelationError::InvalidWorkspace)
        );
        assert_eq!(
            mark_subtree(
                TicketId(3),
                &[ParentLink {
                    parent: Some(root.id),
                    ..root
                }],
                &mut [DescendantMark::Outside]
            ),
            Err(RelationError::MissingEndpoint)
        );
    }

    #[test]
    fn subtrees_include_roots_across_long_parent_chains() {
        use super::*;
        let nodes: alloc::vec::Vec<_> = (1..=8)
            .map(|id| ParentLink {
                id: TicketId(id),
                parent: (id < 8).then_some(TicketId(id + 1)),
            })
            .collect();
        let mut marks = [DescendantMark::Outside; 8];
        assert_eq!(mark_subtree(TicketId(8), &nodes, &mut marks), Ok(()));
        assert_eq!(marks, [DescendantMark::Included; 8]);
        assert_eq!(mark_subtree(TicketId(4), &nodes, &mut marks), Ok(()));
        for (node, mark) in nodes.iter().zip(marks) {
            assert_eq!(mark == DescendantMark::Included, node.id.get() <= 4);
        }
        assert_eq!(mark_descendants(TicketId(4), &nodes, &mut marks), Ok(()));
        for (node, mark) in nodes.iter().zip(marks) {
            assert_eq!(mark == DescendantMark::Included, node.id.get() < 4);
        }
    }

    #[test]
    fn descendant_selection_checks_inputs_and_reuses_scratch() {
        use super::*;
        let root = TicketId(99);
        let leaf = ParentLink {
            id: TicketId(1),
            parent: Some(root),
        };
        let nodes = [
            leaf,
            ParentLink {
                id: root,
                parent: None,
            },
        ];
        let mut marks = [DescendantMark::Included; 2];
        assert_eq!(
            mark_descendants(root, &nodes, &mut []),
            Err(RelationError::InvalidWorkspace)
        );
        assert_eq!(
            mark_descendants(root, &[leaf, leaf], &mut marks),
            Err(RelationError::InvalidWorkspace)
        );
        assert_eq!(
            mark_descendants(TicketId(2), &nodes, &mut marks),
            Err(RelationError::MissingEndpoint)
        );
        assert_eq!(
            mark_descendants(TicketId(1), &[leaf], &mut marks[..1]),
            Err(RelationError::MissingEndpoint)
        );
        assert_eq!(mark_descendants(root, &nodes, &mut marks), Ok(()));
        assert_eq!(marks, [DescendantMark::Included, DescendantMark::Outside]);
        assert_eq!(mark_descendants(TicketId(1), &nodes, &mut marks), Ok(()));
        assert_eq!(marks, [DescendantMark::Outside; 2]);
        let mut reversed = nodes;
        reversed.reverse();
        assert_eq!(mark_descendants(root, &reversed, &mut marks), Ok(()));
        assert_eq!(marks, [DescendantMark::Outside, DescendantMark::Included]);
    }

    #[test]
    fn only_open_work_with_completed_dependencies_is_ready() {
        assert!(super::ready(true, true));
        assert!(!super::ready(true, false));
        assert!(!super::ready(false, true));
        assert!(!super::ready(false, false));
    }

    #[test]
    fn graphs_refuse_missing_nodes_self_edges_and_long_cycles() {
        use super::{Edge, GraphMark, RelationError, TicketId, validate_graph};
        let a = TicketId(1);
        let b = TicketId(2);
        let c = TicketId(3);
        assert_eq!(
            validate_graph(
                &[a],
                &[Edge { from: a, to: b }],
                &mut [GraphMark::Pending(0)]
            ),
            Err(RelationError::MissingEndpoint)
        );
        assert_eq!(
            validate_graph(
                &[a],
                &[Edge { from: a, to: a }],
                &mut [GraphMark::Pending(0)]
            ),
            Err(RelationError::SelfReference)
        );
        let edges = [
            Edge { from: a, to: b },
            Edge { from: b, to: c },
            Edge { from: c, to: a },
        ];
        assert_eq!(
            validate_graph(&[a, b, c], &edges, &mut [GraphMark::Pending(0); 3]),
            Err(RelationError::Cycle)
        );
        assert_eq!(
            validate_graph(&[a, b, c], &edges[..2], &mut [GraphMark::Pending(0); 3]),
            Ok(())
        );
    }

    #[test]
    fn every_three_node_graph_matches_transitive_closure() {
        use super::*;
        let nodes = [TicketId(1), TicketId(2), TicketId(3)];
        for mask in 0..512_u16 {
            let mut edges = alloc::vec::Vec::new();
            let mut closure = alloc::collections::BTreeSet::new();
            for (i, from) in nodes.iter().enumerate() {
                for (j, to) in nodes.iter().enumerate() {
                    if mask & (1 << (i * 3 + j)) != 0 {
                        edges.push(Edge {
                            from: *from,
                            to: *to,
                        });
                        closure.insert((*from, *to));
                    }
                }
            }
            for pivot in nodes {
                for from in nodes {
                    for to in nodes {
                        if closure.contains(&(from, pivot)) && closure.contains(&(pivot, to)) {
                            closure.insert((from, to));
                        }
                    }
                }
            }
            let cyclic = nodes.iter().any(|n| closure.contains(&(*n, *n)));
            assert_eq!(
                validate_graph(&nodes, &edges, &mut [GraphMark::Pending(0); 3]).is_err(),
                cyclic,
                "mask {mask}"
            );
        }
    }

    #[test]
    fn graph_workspace_is_checked_and_reinitialized() {
        use super::*;
        let nodes = [TicketId(1), TicketId(2)];
        assert_eq!(
            validate_graph(&nodes, &[], &mut []),
            Err(RelationError::InvalidWorkspace)
        );
        let mut marks = [GraphMark::Removed; 2];
        assert_eq!(validate_graph(&nodes, &[], &mut marks), Ok(()));
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn counterexample_contract() {
        let evidence = CounterexampleEvidence {
            counts: ProofCounts {
                expected: kani::any(),
                total: kani::any(),
                executed: kani::any(),
                successful: kani::any(),
                failed: kani::any(),
            },
            process_failed: kani::any(),
            report_completed: kani::any(),
            harness_matches: kani::any(),
            assertion_failed: kani::any(),
            checks_valid: kani::any(),
            error_is_assertion: kani::any(),
        };
        let refused = evidence.counts.expected != 1
            || evidence.counts.total != 1
            || evidence.counts.executed != 1
            || evidence.counts.successful != 0
            || evidence.counts.failed != 1
            || !evidence.process_failed
            || !evidence.report_completed
            || !evidence.harness_matches
            || !evidence.assertion_failed
            || !evidence.checks_valid
            || !evidence.error_is_assertion;
        assert_eq!(evidence.accepted(), !refused);
        kani::cover!(evidence.accepted());
        kani::cover!(evidence.counts.expected != 1 && !evidence.accepted());
        kani::cover!(evidence.counts.total != 1 && !evidence.accepted());
        kani::cover!(evidence.counts.executed != 1 && !evidence.accepted());
        kani::cover!(evidence.counts.successful != 0 && !evidence.accepted());
        kani::cover!(evidence.counts.failed != 1 && !evidence.accepted());
        kani::cover!(!evidence.process_failed && !evidence.accepted());
        kani::cover!(!evidence.report_completed && !evidence.accepted());
        kani::cover!(!evidence.harness_matches && !evidence.accepted());
        kani::cover!(!evidence.assertion_failed && !evidence.accepted());
        kani::cover!(!evidence.checks_valid && !evidence.accepted());
        kani::cover!(!evidence.error_is_assertion && !evidence.accepted());
    }

    #[kani::proof]
    fn audit_coverage_contract() {
        let coverage = AuditCoverage {
            advisories: kani::any(),
            dependencies: kani::any(),
            vulnerabilities: kani::any(),
            settings_complete: kani::any(),
            findings_empty: kani::any(),
        };
        let refused = coverage.advisories == 0
            || coverage.dependencies == 0
            || coverage.vulnerabilities != 0
            || !coverage.settings_complete
            || !coverage.findings_empty;
        assert_eq!(coverage.accepted(), !refused);
        kani::cover!(coverage.accepted());
        kani::cover!(coverage.advisories == 0 && !coverage.accepted());
        kani::cover!(coverage.dependencies == 0 && !coverage.accepted());
        kani::cover!(coverage.vulnerabilities != 0 && !coverage.accepted());
        kani::cover!(!coverage.settings_complete && !coverage.accepted());
        kani::cover!(!coverage.findings_empty && !coverage.accepted());
        kani::cover!(
            coverage.advisories == u64::MAX
                && coverage.dependencies == u64::MAX
                && coverage.accepted()
        );
    }

    #[kani::proof]
    fn scoped_dependency_contract() {
        let choice: u8 = kani::any();
        kani::assume(choice < 3);
        let mark = match choice {
            0 => DependencyMark::Outside,
            1 => DependencyMark::Pending,
            _ => DependencyMark::Included,
        };
        let member: bool = kani::any();
        let included = mark.included_in(member);
        let expected = match (choice, member) {
            (2, true) => true,
            _ => false,
        };
        assert_eq!(included, expected);
        kani::cover!(choice == 0 && !member && !included);
        kani::cover!(choice == 0 && member && !included);
        kani::cover!(choice == 1 && !member && !included);
        kani::cover!(choice == 1 && member && !included);
        kani::cover!(choice == 2 && !member && !included);
        kani::cover!(choice == 2 && member && included);
    }

    fn symbolic_id() -> TicketId {
        let value: u64 = kani::any();
        kani::assume(value != 0);
        TicketId(value)
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn dependency_relations_contract() {
        let a = symbolic_id();
        let b = symbolic_id();
        let c = symbolic_id();
        let nodes = [a, b, c].map(|id| DependencyNode {
            id,
            state: DependencyState::Active,
        });
        let edges = [
            Edge {
                from: symbolic_id(),
                to: symbolic_id(),
            },
            Edge {
                from: symbolic_id(),
                to: symbolic_id(),
            },
        ];
        let endpoints = |id| id == a || id == b || id == c;
        let valid = a != b
            && a != c
            && b != c
            && edges
                .iter()
                .all(|edge| endpoints(edge.from) && endpoints(edge.to) && edge.from != edge.to);
        let result = DependencyRelations::new(&nodes, &edges);
        assert_eq!(result.is_ok(), valid);
        kani::cover!(result.is_ok());
        kani::cover!(matches!(result, Err(RelationError::InvalidWorkspace)));
        kani::cover!(matches!(result, Err(RelationError::MissingEndpoint)));
        kani::cover!(matches!(result, Err(RelationError::SelfReference)));
    }

    #[kani::proof]
    fn bottleneck_rank_contract() {
        let left_count: u64 = kani::any();
        let right_count: u64 = kani::any();
        let left_id: u64 = kani::any();
        let right_id: u64 = kani::any();
        kani::assume(left_id != 0 && right_id != 0);
        let left_priority: u8 = kani::any();
        let right_priority: u8 = kani::any();
        kani::assume(left_priority < 3 && right_priority < 3);
        let priority = |value| match value {
            0 => WorkPriority::Low,
            1 => WorkPriority::Normal,
            _ => WorkPriority::High,
        };
        let left = BottleneckRank::new(left_count, priority(left_priority), TicketId(left_id));
        let right = BottleneckRank::new(right_count, priority(right_priority), TicketId(right_id));
        assert_eq!(left.is_some(), left_count != 0);
        assert_eq!(right.is_some(), right_count != 0);
        if let (Some(left), Some(right)) = (left, right) {
            let before = left_count > right_count
                || (left_count == right_count && left_priority > right_priority)
                || (left_count == right_count
                    && left_priority == right_priority
                    && left_id < right_id);
            let equal =
                left_count == right_count && left_priority == right_priority && left_id == right_id;
            assert_eq!(left < right, before);
            assert_eq!(left == right, equal);
            assert_eq!(left > right, !before && !equal);
            kani::cover!(
                left_count > right_count && left_priority < right_priority && left < right
            );
            kani::cover!(
                left_count == right_count && left_priority > right_priority && left < right
            );
            kani::cover!(
                left_count == right_count
                    && left_priority == right_priority
                    && left_id < right_id
                    && left < right
            );
            kani::cover!(left == right);
            kani::cover!(left_count == u64::MAX && right_count < u64::MAX && left < right);
            kani::cover!(left > right);
        }
        kani::cover!(left.is_none() && right.is_some());
        kani::cover!(left.is_some() && right.is_none());
    }

    fn symbolic_dependency_state() -> DependencyState {
        let choice: u8 = kani::any();
        kani::assume(choice < 3);
        match choice {
            0 => DependencyState::Active,
            1 => DependencyState::Done,
            _ => DependencyState::Cancelled,
        }
    }

    fn check_dependency_query<
        const AB: bool,
        const BC: bool,
        const AC: bool,
        const IMPACT: bool,
    >() -> (DependencyState, bool, bool, DependencyState) {
        let a = DependencyNode {
            id: TicketId(1),
            state: symbolic_dependency_state(),
        };
        let b = DependencyNode {
            id: TicketId(2),
            state: symbolic_dependency_state(),
        };
        let c = DependencyNode {
            id: TicketId(3),
            state: symbolic_dependency_state(),
        };
        let x = Edge {
            from: a.id,
            to: b.id,
        };
        let y = Edge {
            from: b.id,
            to: c.id,
        };
        let z = Edge {
            from: a.id,
            to: c.id,
        };
        let mut marks = [DependencyMark::Included; 3];
        let edges: &[Edge] = match (AB, BC, AC) {
            (false, false, false) => &[],
            (true, false, false) => &[x],
            (false, true, false) => &[y],
            (false, false, true) => &[z],
            (true, true, false) => &[x, y],
            (true, false, true) => &[x, z],
            (false, true, true) => &[y, z],
            (true, true, true) => &[x, y, z],
        };
        let (query, root) = if IMPACT {
            (DependencyQuery::Impact, c)
        } else {
            (DependencyQuery::Blockers, a)
        };
        let result = mark_dependencies(query, root.id, &[c, b, a], edges, &mut marks);
        assert!(result.is_ok());
        let (selected_a, selected_b, selected_c) = if IMPACT {
            (
                c.state != DependencyState::Done
                    && a.state == DependencyState::Active
                    && (AC || (AB && BC && b.state == DependencyState::Active)),
                c.state != DependencyState::Done && BC && b.state == DependencyState::Active,
                false,
            )
        } else {
            (
                false,
                a.state == DependencyState::Active && AB && b.state != DependencyState::Done,
                a.state == DependencyState::Active
                    && c.state != DependencyState::Done
                    && (AC || (AB && b.state == DependencyState::Active && BC)),
            )
        };
        assert_eq!(
            marks,
            [
                if selected_c {
                    DependencyMark::Included
                } else {
                    DependencyMark::Outside
                },
                if selected_b {
                    DependencyMark::Included
                } else {
                    DependencyMark::Outside
                },
                if selected_a {
                    DependencyMark::Included
                } else {
                    DependencyMark::Outside
                },
            ]
        );
        kani::cover!(root.state == DependencyState::Active);
        kani::cover!(root.state == DependencyState::Done);
        kani::cover!(root.state == DependencyState::Cancelled);
        (
            b.state,
            selected_b,
            if IMPACT { selected_a } else { selected_c },
            root.state,
        )
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_empty_contract() {
        check_dependency_query::<false, false, false, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_direct_contract() {
        check_dependency_query::<true, false, false, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_unreachable_contract() {
        check_dependency_query::<false, true, false, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_skip_contract() {
        check_dependency_query::<false, false, true, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_chain_contract() {
        let (state, selected_b, selected_c, _) =
            check_dependency_query::<true, true, false, false>();
        kani::cover!(selected_c);
        kani::cover!(selected_b && state == DependencyState::Cancelled && !selected_c);
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_fork_contract() {
        check_dependency_query::<true, false, true, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_detached_contract() {
        check_dependency_query::<false, true, true, false>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn blocker_shared_contract() {
        let (state, _, selected_c, _) = check_dependency_query::<true, true, true, false>();
        kani::cover!(selected_c && state == DependencyState::Done);
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_empty_contract() {
        check_dependency_query::<false, false, false, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_direct_contract() {
        check_dependency_query::<false, true, false, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_unreachable_contract() {
        check_dependency_query::<true, false, false, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_skip_contract() {
        check_dependency_query::<false, false, true, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_chain_contract() {
        let (state, selected_b, selected_a, root) =
            check_dependency_query::<true, true, false, true>();
        kani::cover!(selected_a);
        kani::cover!(state == DependencyState::Cancelled && !selected_b && !selected_a);
        kani::cover!(state == DependencyState::Done && !selected_b && !selected_a);
        kani::cover!(root == DependencyState::Cancelled && selected_a);
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_fork_contract() {
        check_dependency_query::<false, true, true, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_detached_contract() {
        check_dependency_query::<true, false, true, true>();
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn impact_shared_contract() {
        let (state, _, selected_a, _) = check_dependency_query::<true, true, true, true>();
        kani::cover!(selected_a && state == DependencyState::Done);
        kani::cover!(selected_a && state == DependencyState::Cancelled);
    }

    fn symbolic_parent() -> Option<TicketId> {
        let choice: u8 = kani::any();
        kani::assume(choice < 4);
        match choice {
            0 => None,
            1 => Some(TicketId(1)),
            2 => Some(TicketId(2)),
            _ => Some(TicketId(3)),
        }
    }

    fn check_parent_scope<const INCLUDE_ROOT: bool>()
    -> (Result<(), RelationError>, [DescendantMark; 3]) {
        let nodes = [
            ParentLink {
                id: TicketId(1),
                parent: symbolic_parent(),
            },
            ParentLink {
                id: TicketId(2),
                parent: symbolic_parent(),
            },
            ParentLink {
                id: TicketId(3),
                parent: symbolic_parent(),
            },
        ];
        let choice: u8 = kani::any();
        kani::assume(choice < 3);
        let root = match choice {
            0 => TicketId(1),
            1 => TicketId(2),
            _ => TicketId(3),
        };
        let mut marks = [DescendantMark::Included; 3];
        let result = if INCLUDE_ROOT {
            ParentRelations::new(&nodes)
                .and_then(|relations| relations.mark_subtree(root, &mut marks))
        } else {
            mark_descendants(root, &nodes, &mut marks)
        };
        let self_reference = nodes.iter().any(|node| node.parent == Some(node.id));
        assert_eq!(result.is_err(), self_reference);
        if result.is_ok() {
            for (node, mark) in nodes.iter().zip(marks) {
                let reachable = node.parent == Some(root)
                    || nodes
                        .iter()
                        .any(|via| via.parent == Some(root) && node.parent == Some(via.id));
                assert_eq!(
                    mark == DescendantMark::Included,
                    (INCLUDE_ROOT && node.id == root) || (node.id != root && reachable)
                );
                assert_ne!(mark, DescendantMark::Pending);
            }
        }
        (result, marks)
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn descendant_selection_contract() {
        let (result, marks) = check_parent_scope::<false>();
        kani::cover!(result.is_err());
        kani::cover!(result.is_ok() && marks.iter().any(|mark| *mark == DescendantMark::Included));
        kani::cover!(result.is_ok() && marks.iter().all(|mark| *mark == DescendantMark::Outside));
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn subtree_selection_contract() {
        let (result, marks) = check_parent_scope::<true>();
        kani::cover!(result.is_err());
        kani::cover!(
            result.is_ok()
                && marks
                    .iter()
                    .filter(|mark| **mark == DescendantMark::Included)
                    .count()
                    == 1
        );
        kani::cover!(result.is_ok() && marks.iter().all(|mark| *mark == DescendantMark::Included));
    }

    #[kani::proof]
    fn proof_check_contract() {
        let choice: u8 = kani::any();
        kani::assume(choice < 6);
        let result = match choice {
            0 => ProofCheckResult::Success,
            1 => ProofCheckResult::Satisfied,
            2 => ProofCheckResult::Unreachable,
            3 => ProofCheckResult::Invalid,
            4 => ProofCheckResult::Failure,
            _ => ProofCheckResult::Unsatisfiable,
        };
        let cover: bool = kani::any();
        let accepted = result.accepted(cover);
        assert_eq!(accepted, if cover { choice == 1 } else { choice < 3 });
        kani::cover!(cover && choice == 1 && accepted);
        kani::cover!(cover && choice == 2 && !accepted);
        kani::cover!(!cover && choice == 2 && accepted);
        kani::cover!(choice == 3 && !accepted);
        let kind_choice: u8 = kani::any();
        kani::assume(kind_choice < 3);
        let kind = match kind_choice {
            0 => ProofCheckKind::Assertion,
            1 => ProofCheckKind::Cover,
            _ => ProofCheckKind::Other,
        };
        let supports = result.supports_counterexample(kind);
        let expected = match kind_choice {
            0 => choice == 0 || choice == 2 || choice == 4,
            1 => choice == 1 || choice == 2 || choice == 5,
            _ => choice == 0 || choice == 2,
        };
        assert_eq!(supports, expected);
        kani::cover!(kind_choice == 0 && choice == 4 && supports);
        kani::cover!(kind_choice == 1 && choice == 5 && supports);
        kani::cover!(kind_choice == 2 && choice == 0 && supports);
        kani::cover!(kind_choice == 2 && choice == 4 && !supports);
    }

    #[kani::proof]
    fn proof_completeness_contract() {
        let counts = ProofCounts {
            expected: kani::any(),
            total: kani::any(),
            executed: kani::any(),
            successful: kani::any(),
            failed: kani::any(),
        };
        assert_eq!(
            counts.complete(),
            counts.expected > 0
                && counts.total == counts.expected
                && counts.executed == counts.expected
                && counts.successful == counts.expected
                && counts.failed == 0
        );
        kani::cover!(counts.complete());
        kani::cover!(!counts.complete());
    }

    #[kani::proof]
    fn readiness_contract() {
        let open: bool = kani::any();
        let done: bool = kani::any();
        assert_eq!(ready(open, done), open && done);
        kani::cover!(ready(open, done));
        kani::cover!(!ready(open, done));
    }

    #[kani::proof]
    #[kani::unwind(9)]
    fn repo_count_contract() {
        let counts = WorkCounts {
            total: kani::any(),
            open: kani::any(),
            in_progress: kani::any(),
            done: kani::any(),
            cancelled: kani::any(),
            ready: kani::any(),
            blocked: kani::any(),
        };
        let choice: u8 = kani::any();
        kani::assume(choice < 4);
        let state = match choice {
            0 => WorkState::Open,
            1 => WorkState::InProgress,
            2 => WorkState::Done,
            _ => WorkState::Cancelled,
        };
        let dependencies_done: bool = kani::any();
        let increments: [u64; 7] = match (choice, dependencies_done) {
            (0, true) => [1, 1, 0, 0, 0, 1, 0],
            (0, false) => [1, 1, 0, 0, 0, 0, 1],
            (1, true) => [1, 0, 1, 0, 0, 0, 0],
            (1, false) => [1, 0, 1, 0, 0, 0, 1],
            (2, _) => [1, 0, 0, 1, 0, 0, 0],
            _ => [1, 0, 0, 0, 1, 0, 0],
        };
        let before = [
            counts.total,
            counts.open,
            counts.in_progress,
            counts.done,
            counts.cancelled,
            counts.ready,
            counts.blocked,
        ];
        let result = counts.record(state, dependencies_done);
        let fits = before
            .iter()
            .zip(increments)
            .all(|(value, delta)| u128::from(*value) + u128::from(delta) <= u128::from(u64::MAX));
        assert_eq!(result.is_some(), fits);
        if let Some(after) = result {
            let values = [
                after.total,
                after.open,
                after.in_progress,
                after.done,
                after.cancelled,
                after.ready,
                after.blocked,
            ];
            for ((value, previous), delta) in values.into_iter().zip(before).zip(increments) {
                assert_eq!(u128::from(value), u128::from(previous) + u128::from(delta));
            }
        }
        kani::cover!(fits && choice == 0 && dependencies_done);
        kani::cover!(fits && choice == 0 && !dependencies_done);
        kani::cover!(fits && choice == 1 && dependencies_done);
        kani::cover!(fits && choice == 1 && !dependencies_done);
        kani::cover!(fits && choice == 2 && dependencies_done);
        kani::cover!(fits && choice == 2 && !dependencies_done);
        kani::cover!(fits && choice == 3 && dependencies_done);
        kani::cover!(fits && choice == 3 && !dependencies_done);
        kani::cover!(!fits);
    }

    #[kani::proof]
    fn revision_change_contract() {
        let expected: u64 = kani::any();
        let current: u64 = kani::any();
        let changed: bool = kani::any();
        let matched = MatchedRevision::new(expected, current);
        assert_eq!(matched.is_some(), expected != 0 && expected == current);
        if let Some(matched) = matched {
            let result = matched.update(changed);
            match result {
                Some(RevisionChange::Unchanged) => assert!(!changed),
                Some(RevisionChange::Changed(revision)) => {
                    assert!(changed);
                    assert!(revision > current);
                    assert_eq!(u128::from(revision), u128::from(current) + 1);
                }
                None => assert!(changed && current == u64::MAX),
            }
            assert_eq!(result.is_none(), changed && current == u64::MAX);
            kani::cover!(result == Some(RevisionChange::Unchanged));
            kani::cover!(current == u64::MAX && result == Some(RevisionChange::Unchanged));
            kani::cover!(matches!(result, Some(RevisionChange::Changed(_))));
            kani::cover!(result.is_none());
        } else {
            kani::cover!(!changed && expected != 0 && expected != current);
            kani::cover!(expected == 0);
        }
    }

    #[kani::proof]
    fn identifiers_and_revisions() {
        let id: u64 = kani::any();
        assert_eq!(TicketId::new(id).is_some(), id != 0);
        let expected: u64 = kani::any();
        let current: u64 = kani::any();
        assert_eq!(
            revision_matches(expected, current),
            expected != 0 && expected == current
        );
        assert_eq!(next_sequence(current).is_none(), current == u64::MAX);
        kani::cover!(revision_matches(expected, current));
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn triangle_cycle_contract() {
        let a = TicketId(1);
        let b = TicketId(2);
        let c = TicketId(3);
        let ab: bool = kani::any();
        let bc: bool = kani::any();
        let ca: bool = kani::any();
        let x = Edge { from: a, to: b };
        let y = Edge { from: b, to: c };
        let z = Edge { from: c, to: a };
        let mut marks = [GraphMark::Pending(0); 3];
        let result = match (ab, bc, ca) {
            (false, false, false) => validate_graph(&[a, b, c], &[], &mut marks),
            (true, false, false) => validate_graph(&[a, b, c], &[x], &mut marks),
            (false, true, false) => validate_graph(&[a, b, c], &[y], &mut marks),
            (false, false, true) => validate_graph(&[a, b, c], &[z], &mut marks),
            (true, true, false) => validate_graph(&[a, b, c], &[x, y], &mut marks),
            (true, false, true) => validate_graph(&[a, b, c], &[x, z], &mut marks),
            (false, true, true) => validate_graph(&[a, b, c], &[y, z], &mut marks),
            (true, true, true) => validate_graph(&[a, b, c], &[x, y, z], &mut marks),
        };
        assert_eq!(result.is_err(), ab && bc && ca);
        kani::cover!(result.is_ok());
        kani::cover!(result.is_err());
    }
}
