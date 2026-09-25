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
    use super::Reachability;

    #[test]
    fn a_fresh_graph_has_no_reachability() {
        let r = Reachability::new(3);
        assert!(!r.reaches(0, 1));
        assert!(!r.reaches(0, 0));
    }

    #[test]
    fn an_edge_makes_its_head_reachable() {
        let mut r = Reachability::new(2);
        r.add_edge(0, 1);
        assert!(r.reaches(0, 1));
        assert!(!r.reaches(1, 0));
    }

    #[test]
    fn reachability_is_transitive() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        r.add_edge(1, 2);
        assert!(r.reaches(0, 2));
    }

    #[test]
    fn an_edge_into_an_existing_chain_reaches_its_descendants() {
        let mut r = Reachability::new(3);
        r.add_edge(1, 2);
        r.add_edge(0, 1);
        assert!(r.reaches(0, 2));
    }

    #[test]
    fn reachability_spans_more_than_one_word() {
        let mut r = Reachability::new(130);
        r.add_edge(0, 70);
        r.add_edge(70, 129);
        assert!(r.reaches(0, 129));
        assert!(!r.reaches(129, 0));
    }

    #[test]
    fn try_add_edge_refuses_an_edge_that_would_close_a_cycle() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        r.add_edge(1, 2);
        assert!(!r.try_add_edge(2, 0));
        assert!(!r.reaches(2, 0));
    }

    #[test]
    fn try_add_edge_refuses_a_self_loop() {
        let mut r = Reachability::new(1);
        assert!(!r.try_add_edge(0, 0));
    }

    #[test]
    fn try_add_edge_accepts_an_edge_that_does_not_close_a_cycle() {
        let mut r = Reachability::new(3);
        r.add_edge(0, 1);
        assert!(r.try_add_edge(1, 2));
        assert!(r.reaches(0, 2));
    }
}
