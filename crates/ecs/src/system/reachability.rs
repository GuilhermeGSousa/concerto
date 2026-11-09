use fixedbitset::FixedBitSet;

pub(crate) struct Reachability {
    descendants: Vec<FixedBitSet>,
}

impl Reachability {
    pub(crate) fn new(node_count: usize) -> Self {
        Self {
            descendants: vec![FixedBitSet::with_capacity(node_count); node_count],
        }
    }

    pub(crate) fn reaches(&self, from: usize, to: usize) -> bool {
        self.descendants[from].contains(to)
    }

    pub(crate) fn add_edge(&mut self, from: usize, to: usize) {
        if self.reaches(from, to) {
            return;
        }

        let mut closure = self.descendants[to].clone();
        closure.insert(to);

        for node in 0..self.descendants.len() {
            if node == from || self.reaches(node, from) {
                self.descendants[node].union_with(&closure);
            }
        }
    }

    pub(crate) fn try_add_edge(&mut self, from: usize, to: usize) -> bool {
        if from == to || self.reaches(to, from) {
            return false;
        }
        self.add_edge(from, to);
        true
    }
}

#[cfg(test)]
mod tests {
    use proptest::{prelude::*, sample::Index};

    use super::Reachability;

    struct Edge {
        from: usize,
        to: usize,
    }

    fn descendants(nodes: usize, edges: &[Edge], from: usize) -> Vec<bool> {
        let mut reached = vec![false; nodes];
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            for edge in edges.iter().filter(|edge| edge.from == node) {
                if !reached[edge.to] {
                    reached[edge.to] = true;
                    stack.push(edge.to);
                }
            }
        }
        reached
    }

    proptest! {
        #[test]
        fn try_add_edge_keeps_the_graph_acyclic_and_reaches_its_transitive_closure(
            nodes in 1usize..140,
            ends in prop::collection::vec(any::<Index>(), 0..120),
        ) {
            let mut reachability = Reachability::new(nodes);
            let mut accepted = Vec::new();
            for pair in ends.chunks_exact(2) {
                let edge = Edge {
                    from: pair[0].index(nodes),
                    to: pair[1].index(nodes),
                };
                let closes_a_cycle = edge.from == edge.to
                    || descendants(nodes, &accepted, edge.to)[edge.from];

                prop_assert_eq!(reachability.try_add_edge(edge.from, edge.to), !closes_a_cycle);
                if !closes_a_cycle {
                    accepted.push(edge);
                }
            }

            for from in 0..nodes {
                let expected = descendants(nodes, &accepted, from);
                for (to, reached) in expected.into_iter().enumerate() {
                    prop_assert_eq!(reachability.reaches(from, to), reached, "{} -> {}", from, to);
                }
            }
        }
    }
}
