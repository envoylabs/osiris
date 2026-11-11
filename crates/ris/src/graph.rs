//! The graph of work in motion, for `ris summary`: open decisions and
//! unmerged items as nodes, and the links between them as edges, drawn
//! as a tree.
//!
//! Edges point from what must happen first to what waits on it or is
//! shaped by it:
//!
//! - a decision to each item it was raised about, and a person action
//!   to each item it blocks or informs: `blocks` when the item is set
//!   aside on it, `informs` otherwise;
//! - an item to each item that depends on it, `blocks` (a dependency
//!   must merge first).
//!
//! The links come from the reverse index, and an item stands for the
//! items it replaced. Nodes with no edge are counted, not drawn. A node
//! with two parents is drawn under the first one reached and shown as
//! `(above)` under the others, so every edge appears exactly once.
//! Lines are cut at [`WIDTH`] columns and the drawing at [`MAX_LINES`].

use std::collections::{BTreeMap, BTreeSet};

use ris_core::ids::{ActionId, DecisionId, ItemId};
use ris_core::primitives::{ActionState, Blocker, DecisionState, ItemState};
use serde_json::{Value, json};

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
/// resolves (`Blocks`), or proceeds and is shaped by it (`Informs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Blocks,
    Informs,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::Informs => "informs",
        }
    }
}

/// The nodes in motion and the edges between them.
pub struct Graph {
    /// Every node in motion, linked or not.
    pub nodes: BTreeSet<Node>,
    /// `from → [(to, kind)]`, children in order.
    pub edges: BTreeMap<Node, Vec<(Node, Kind)>>,
}

impl Graph {
    pub fn of(l: &Loaded) -> Self {
        Self::of_scope(l, Scope::All)
    }

    /// The graph over the decisions and items in `scope`. Open person
    /// actions are the person's own to-do and are always nodes.
    pub fn of_scope(l: &Loaded, scope: Scope) -> Self {
        let s = &l.state;
        let mut nodes = BTreeSet::new();
        for d in s.open_decisions() {
            if scope.decision(l, d) {
                nodes.insert(Node::Decision(d.id));
            }
        }
        for a in s.actions.values() {
            if a.state == ActionState::Open {
                nodes.insert(Node::Action(a.id));
            }
        }
        for i in s.items.values() {
            if scope.item(i)
                && !matches!(
                    i.state,
                    ItemState::Merged { .. } | ItemState::Superseded { .. }
                )
            {
                nodes.insert(Node::Item(i.id));
            }
        }
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
                let from = Node::Item(s.current(*dep));
                if nodes.contains(&from) {
                    edges.entry(from).or_default().push((n, Kind::Blocks));
                }
            }
        }
        for children in edges.values_mut() {
            children.sort();
            children.dedup();
        }
        Self { nodes, edges }
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
        if linked.is_empty() {
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
        if unlinked > 0 {
            out.push(format!(
                "{unlinked} open decision(s) and unmerged item(s) have no links"
            ));
        }
        out.join("\n")
    }

    /// Every drawn line, uncut in number. Roots (linked nodes with no
    /// parent) are walked first; then any linked node the walk missed is
    /// walked as a root. Only a cycle has no root, and the log can hold
    /// one written before supersede refused them; without this second
    /// pass its nodes and edges would vanish from the drawing.
    fn lines(&self, label: &dyn Fn(Node) -> String) -> Vec<String> {
        let linked = self.linked();
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
            (Some(Kind::Blocks), false) => ("├── ", "│   "),
            (Some(Kind::Blocks), true) => ("└── ", "    "),
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
    pub fn json(&self, l: &Loaded) -> Value {
        let nodes: Vec<Value> = self
            .linked()
            .iter()
            .map(|n| json!({ "id": n.to_string(), "state": state(l, *n) }))
            .collect();
        let edges: Vec<Value> = self
            .edges
            .iter()
            .flat_map(|(from, children)| {
                children.iter().map(move |(to, kind)| {
                    json!({ "from": from.to_string(), "to": to.to_string(), "kind": kind.word() })
                })
            })
            .collect();
        json!({ "nodes": nodes, "edges": edges, "unlinked": self.nodes.len() - self.linked().len() })
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

fn label(l: &Loaded, n: Node) -> String {
    let text = match n {
        Node::Decision(d) => l.text(l.state.decisions[&d].at, "question"),
        Node::Action(a) => l.text(l.state.actions[&a].at, "what"),
        Node::Item(i) => l.text(l.state.items[&i].at, "goal"),
    };
    format!("{n}  {}  {text}", state(l, n))
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
mod tests {
    use super::*;

    /// A cycle has no root. Each of its nodes is still drawn once in
    /// full, and each of its edges appears once: the edge back into the
    /// cycle shows as `(above)`.
    #[test]
    fn a_cycle_is_drawn_with_every_node_and_edge() {
        let a = Node::Item(ItemId(5));
        let b = Node::Item(ItemId(6));
        let c = Node::Item(ItemId(7));
        let d = Node::Decision(DecisionId(1));
        let mut edges = BTreeMap::new();
        edges.insert(a, vec![(b, Kind::Blocks)]);
        edges.insert(b, vec![(a, Kind::Blocks)]);
        edges.insert(d, vec![(c, Kind::Blocks)]);
        let g = Graph {
            nodes: [a, b, c, d].into_iter().collect(),
            edges,
        };
        let lines = g.lines(&|n| format!("{n}  x"));
        assert_eq!(
            lines,
            vec![
                "D-1  x",
                "└── I-7  x",
                "I-5  x",
                "└── I-6  x",
                "    └── I-5  (above)"
            ]
        );
    }
}
