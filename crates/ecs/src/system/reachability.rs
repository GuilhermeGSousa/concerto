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

    pub(crate) fn add_node(&mut self) {
        let count = self.descendants.len() + 1;
        for descendants in &mut self.descendants {
            descendants.grow(count);
        }
        self.descendants.push(FixedBitSet::with_capacity(count));
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

    pub(crate) fn redundant_edges(
        &self,
        successors: &[Vec<usize>],
        order: &[usize],
    ) -> Vec<FixedBitSet> {
        let count = self.descendants.len();
        let mut position = vec![0; count];
        for (rank, &node) in order.iter().enumerate() {
            position[node] = rank;
        }
        successors
            .iter()
            .map(|targets| {
                let mut targets = targets.clone();
                targets.sort_unstable_by_key(|&to| position[to]);
                let mut covered = FixedBitSet::with_capacity(count);
                let mut redundant = FixedBitSet::with_capacity(count);
                for to in targets {
                    if covered.contains(to) {
                        redundant.insert(to);
                    } else {
                        covered.union_with(&self.descendants[to]);
                    }
                }
                redundant
            })
            .collect()
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
        fn try_add_edge_keeps_the_graph_acyclic_and_its_reduction_reaches_the_same_closure(
            nodes in 1usize..140,
            ends in prop::collection::vec(any::<Index>(), 0..120),
        ) {
            let mut reachability = Reachability::new(nodes);
            let mut accepted = Vec::new();
            for pair in ends.as_chunks::<2>().0 {
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

            let mut successors = vec![Vec::new(); nodes];
            for edge in &accepted {
                if !successors[edge.from].contains(&edge.to) {
                    successors[edge.from].push(edge.to);
                }
            }
            let mut order: Vec<usize> = (0..nodes).collect();
            order.sort_by_key(|&node| std::cmp::Reverse(descendants(nodes, &accepted, node).iter().filter(|reached| **reached).count()));
            let redundant = reachability.redundant_edges(&successors, &order);
            let reduced: Vec<Edge> = accepted
                .iter()
                .filter(|edge| !redundant[edge.from].contains(edge.to))
                .map(|edge| Edge { from: edge.from, to: edge.to })
                .collect();
            for from in 0..nodes {
                prop_assert_eq!(descendants(nodes, &reduced, from), descendants(nodes, &accepted, from));
                for to in redundant[from].ones() {
                    prop_assert!(successors[from].iter().any(|&other| other != to && reachability.reaches(other, to)));
                }
            }
        }
    }
}
