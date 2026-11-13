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
        whole: false,
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
