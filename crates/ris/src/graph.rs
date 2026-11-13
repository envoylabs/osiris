//! The graph `ris summary` draws: items, open decisions and open person
//! actions as nodes, and the links between them as edges, drawn as a
//! tree.
//!
//! For the open phase it is the phase's picture: every item of the
//! phase in whatever state, each unmerged item from an earlier phase
//! that one of them waits on (and what that waits on in turn), and the
//! open decisions and person actions attached to any of those, whatever
//! phase raised them. Every node is drawn, linked or not. With `--all`
//! it is the work in motion in every phase: open decisions, open person
//! actions and unmerged items, where a node with no edge is counted,
//! not drawn.
//!
//! Edges point from what must happen first to what waits on it or is
//! shaped by it:
//!
//! - a decision to each item it was raised about, and a person action
//!   to each item it blocks or informs: `blocks` when the item is set
//!   aside on it, `informs` otherwise;
//! - an item to each item that depends on it: `blocks` while the
//!   dependency is unmerged (it must merge first), `met` once it has
//!   merged.
//!
//! The links come from the reverse index, and an item stands for the
//! items it replaced. A node with two parents is drawn under the first one reached and shown as
//! `(above)` under the others, so every edge appears exactly once.
//! Lines are cut at [`WIDTH`] columns and the drawing at [`MAX_LINES`].

use std::collections::{BTreeMap, BTreeSet};

use ris_core::ids::{ActionId, DecisionId, ItemId};
use ris_core::primitives::{ActionState, Blocker, DecisionState, ItemState};

use crate::project::Loaded;
use crate::render::Scope;

/// The widest a drawn line may be, in characters.
pub const WIDTH: usize = 80;
/// The most lines the drawing takes; the rest is counted.
pub const MAX_LINES: usize = 40;

/// A node: decisions, then person actions, then items, each by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    Decision(DecisionId),
    Action(ActionId),
    Item(ItemId),
}

impl std::fmt::Display for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decision(d) => d.fmt(f),
            Self::Action(a) => a.fmt(f),
            Self::Item(i) => i.fmt(f),
        }
    }
}

/// How hard a link is: the target cannot proceed until the source
/// resolves (`Blocks`), proceeds and is shaped by it (`Informs`), or
/// waited on a dependency that has since merged (`Met`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Blocks,
    Informs,
    Met,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::Informs => "informs",
            Self::Met => "met",
        }
    }
}

/// The nodes and the edges between them.
pub struct Graph {
    /// Every node, linked or not.
    pub nodes: BTreeSet<Node>,
    /// `from → [(to, kind)]`, children in order.
    pub edges: BTreeMap<Node, Vec<(Node, Kind)>>,
    /// The open phase's picture: a node with no edge is drawn and
    /// listed, where the graph of every phase only counts it.
    pub whole: bool,
}

impl Graph {
    pub fn of(l: &Loaded) -> Self {
        Self::of_scope(l, Scope::All)
    }

    /// The graph for `scope`: the open phase's picture, or the work in
    /// motion in every phase.
    pub fn of_scope(l: &Loaded, scope: Scope) -> Self {
        let s = &l.state;
        let whole = scope != Scope::All;
        let nodes = if whole {
            phase_nodes(l, scope)
        } else {
            let mut nodes = BTreeSet::new();
            nodes.extend(s.open_decisions().map(|d| Node::Decision(d.id)));
            nodes.extend(
                s.actions
                    .values()
                    .filter(|a| a.state == ActionState::Open)
                    .map(|a| Node::Action(a.id)),
            );
            nodes.extend(
                s.items
                    .values()
                    .filter(|i| {
                        !matches!(
                            i.state,
                            ItemState::Merged { .. } | ItemState::Superseded { .. }
                        )
                    })
                    .map(|i| Node::Item(i.id)),
            );
            nodes
        };
        let mut edges: BTreeMap<Node, Vec<(Node, Kind)>> = BTreeMap::new();
        for &n in &nodes {
            let Node::Item(item) = n else { continue };
            // What this item stands for: itself and what it replaced.
            for source in s.lineage(item) {
                for a in s.actions_about(source) {
                    if !nodes.contains(&Node::Action(*a)) {
                        continue;
                    }
                    let blocked = matches!(
                        &s.items[&item].state,
                        ItemState::Blocked { on } if on.contains(&Blocker::Action(*a))
                    );
                    let kind = if blocked { Kind::Blocks } else { Kind::Informs };
                    edges.entry(Node::Action(*a)).or_default().push((n, kind));
                }
                for d in s.decisions_about(source) {
                    if !nodes.contains(&Node::Decision(*d)) {
                        continue;
                    }
                    let blocked = matches!(
                        &s.items[&item].state,
                        ItemState::Blocked { on } if on.contains(&Blocker::Decision(*d))
                    );
                    let kind = if blocked { Kind::Blocks } else { Kind::Informs };
                    edges.entry(Node::Decision(*d)).or_default().push((n, kind));
                }
            }
            for dep in &s.items[&item].depends_on {
                let dep = s.current(*dep);
                let from = Node::Item(dep);
                if nodes.contains(&from) {
                    let kind = match s.items[&dep].state {
                        ItemState::Merged { .. } => Kind::Met,
                        _ => Kind::Blocks,
                    };
                    edges.entry(from).or_default().push((n, kind));
                }
            }
        }
        for children in edges.values_mut() {
            children.sort();
            children.dedup();
        }
        Self {
            nodes,
            edges,
            whole,
        }
    }

    /// The nodes that are drawn and listed: every one in the phase's
    /// picture, otherwise those with an edge.
    fn shown(&self) -> BTreeSet<Node> {
        if self.whole {
            self.nodes.clone()
        } else {
            self.linked()
        }
    }

    fn linked(&self) -> BTreeSet<Node> {
        let mut out: BTreeSet<Node> = self.edges.keys().copied().collect();
        for children in self.edges.values() {
            out.extend(children.iter().map(|(c, _)| *c));
        }
        out
    }

    /// The tree as lines, with what was left out counted.
    pub fn draw(&self, l: &Loaded) -> String {
        let linked = self.linked();
        if self.whole && self.nodes.is_empty() {
            return "no items in this phase yet\n  next: `ris item add \"...\"`".into();
        }
        if !self.whole && linked.is_empty() {
            return "no links between open decisions and unmerged items".into();
        }
        let lines = self.lines(&|n| label(l, n));
        let mut out: Vec<String> = lines.iter().take(MAX_LINES).cloned().collect();
        if lines.len() > MAX_LINES {
            out.push(format!(
                "and {} more line(s): ris --json summary has the whole graph",
                lines.len() - MAX_LINES
            ));
        }
        let unlinked = self.nodes.len() - linked.len();
        if !self.whole && unlinked > 0 {
            out.push(format!(
                "{unlinked} open decision(s) and unmerged item(s) have no links"
            ));
        }
        out.join("\n")
    }

    /// Every drawn line, uncut in number. Roots (shown nodes with no
    /// parent) are walked first; then any shown node the walk missed is
    /// walked as a root. Only a cycle has no root, and the log can hold
    /// one written before supersede refused them; without this second
    /// pass its nodes and edges would vanish from the drawing.
    fn lines(&self, label: &dyn Fn(Node) -> String) -> Vec<String> {
        let linked = self.shown();
        let has_parent: BTreeSet<Node> = self
            .edges
            .values()
            .flat_map(|c| c.iter().map(|(n, _)| *n))
            .collect();
        let mut lines = Vec::new();
        let mut drawn = BTreeSet::new();
        for &root in linked.iter().filter(|n| !has_parent.contains(n)) {
            self.walk(label, root, None, "", true, &mut drawn, &mut lines);
        }
        for &node in &linked {
            if !drawn.contains(&node) {
                self.walk(label, node, None, "", true, &mut drawn, &mut lines);
            }
        }
        lines
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        label: &dyn Fn(Node) -> String,
        node: Node,
        via: Option<Kind>,
        prefix: &str,
        last: bool,
        drawn: &mut BTreeSet<Node>,
        lines: &mut Vec<String>,
    ) {
        let (branch, carry) = match (via, last) {
            (None, _) => ("", ""),
            (Some(Kind::Blocks | Kind::Met), false) => ("├── ", "│   "),
            (Some(Kind::Blocks | Kind::Met), true) => ("└── ", "    "),
            (Some(Kind::Informs), false) => ("├╌╌ ", "│   "),
            (Some(Kind::Informs), true) => ("└╌╌ ", "    "),
        };
        let seen = !drawn.insert(node);
        let text = if seen {
            format!("{node}  (above)")
        } else {
            label(node)
        };
        lines.push(cut(&format!("{prefix}{branch}{text}"), WIDTH));
        if seen {
            return;
        }
        let children = self.edges.get(&node).cloned().unwrap_or_default();
        let next = format!("{prefix}{carry}");
        for (k, (child, kind)) in children.iter().enumerate() {
            self.walk(
                label,
                *child,
                Some(*kind),
                &next,
                k + 1 == children.len(),
                drawn,
                lines,
            );
        }
    }

    /// Nodes and edges for `--json`.
    pub fn json(&self, l: &Loaded) -> crate::schema::Graph {
        let nodes = self
            .shown()
            .iter()
            .map(|n| crate::schema::GraphNode {
                id: n.to_string(),
                state: state(l, *n),
                phase: match n {
                    Node::Item(i) => Some(l.state.items[i].phase),
                    _ => None,
                },
            })
            .collect();
        let edges = self
            .edges
            .iter()
            .flat_map(|(from, children)| {
                children
                    .iter()
                    .map(move |(to, kind)| crate::schema::GraphEdge {
                        from: from.to_string(),
                        to: to.to_string(),
                        kind: kind.word(),
                    })
            })
            .collect();
        crate::schema::Graph {
            nodes,
            edges,
            unlinked: self.nodes.len() - self.linked().len(),
        }
    }
}

fn state(l: &Loaded, n: Node) -> &'static str {
    match n {
        Node::Decision(d) => match l.state.decisions[&d].state {
            DecisionState::Open => "open",
            DecisionState::Taken { .. } => "taken",
            DecisionState::Superseded(_) => "superseded",
        },
        Node::Action(a) => match l.state.actions[&a].state {
            ActionState::Open => "for you",
            ActionState::Done { .. } => "done",
        },
        Node::Item(i) => crate::render::shown_state(l, &l.state.items[&i]),
    }
}

/// The nodes of the open phase's picture: its items, what they wait on
/// from earlier phases, and the open decisions and person actions
/// attached to any of those. Nothing, with no phase open.
fn phase_nodes(l: &Loaded, scope: Scope) -> BTreeSet<Node> {
    let s = &l.state;
    let mut nodes = BTreeSet::new();
    let mut queue: Vec<ItemId> = s
        .items
        .values()
        .filter(|i| scope.item(i) && !matches!(i.state, ItemState::Superseded { .. }))
        .map(|i| i.id)
        .collect();
    nodes.extend(queue.iter().map(|i| Node::Item(*i)));
    while let Some(item) = queue.pop() {
        for dep in s.unlanded_dependencies(&s.items[&item]) {
            if s.items.contains_key(&dep) && nodes.insert(Node::Item(dep)) {
                queue.push(dep);
            }
        }
        for source in s.lineage(item) {
            for d in s.decisions_about(source) {
                if matches!(s.decisions[d].state, DecisionState::Open) {
                    nodes.insert(Node::Decision(*d));
                }
            }
            for a in s.actions_about(source) {
                if s.actions[a].state == ActionState::Open {
                    nodes.insert(Node::Action(*a));
                }
            }
        }
    }
    nodes
}

/// A node's line: id, state, who holds a claimed item, the phase of an
/// item that is not the open phase's, and its text.
fn label(l: &Loaded, n: Node) -> String {
    let text = match n {
        Node::Decision(d) => l.text(l.state.decisions[&d].at, "question"),
        Node::Action(a) => l.text(l.state.actions[&a].at, "what"),
        Node::Item(i) => l.text(l.state.items[&i].at, "goal"),
    };
    let mut note = String::new();
    if let Node::Item(i) = n {
        let item = &l.state.items[&i];
        if let ItemState::Claimed { agent } = &item.state {
            note.push_str(&format!(" ({agent})"));
        }
        if l.state.open_phase().is_some_and(|p| p.id != item.phase) {
            note.push_str(&format!(" [{}]", item.phase));
        }
    }
    format!("{n}  {}{note}  {text}", state(l, n))
}

/// At most `width` characters, ending in `…` when cut.
fn cut(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
#[path = "tests/graph.rs"]
mod tests;
