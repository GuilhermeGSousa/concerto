use std::collections::HashMap;

use concerto::ecs::Resource;

use crate::case::{Case, DeductionId, FactId, Id, TermBankId, TermId};
use crate::players::PLAYER_COUNT;

/// What the players get when both have locked the same deduction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The two copies differ.
    Disagree,
    /// Every blank is right; the deduction is confirmed.
    Correct,
    /// One or two blanks are wrong.
    Close,
    /// More than two blanks are wrong.
    Wrong,
}

impl Verdict {
    /// What the casebook says about it.
    pub fn text(self) -> &'static str {
        match self {
            Verdict::Disagree => "You don't agree",
            Verdict::Correct => "Confirmed",
            Verdict::Close => "Close, but something doesn't fit",
            Verdict::Wrong => "Something doesn't add up",
        }
    }
}

/// One player's copy of a deduction: a term or nothing in each blank, and whether it is locked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub picks: Vec<Option<TermId>>,
    pub locked: bool,
}

/// What the players know: found facts, confirmed deductions and each player's drafts. It survives every loop.
#[derive(Resource, Clone, Debug, Default)]
pub struct Casebook {
    found: Vec<FactId>,
    fresh: Vec<FactId>,
    confirmed: Vec<DeductionId>,
    drafts: [HashMap<DeductionId, Draft>; PLAYER_COUNT as usize],
    loops: u32,
}

impl Casebook {
    /// Adds `fact` for both players; false when they already had it.
    pub fn grant(&mut self, fact: FactId) -> bool {
        if self.found.contains(&fact) {
            return false;
        }
        self.found.push(fact);
        self.fresh.push(fact);
        true
    }

    pub fn knows(&self, fact: FactId) -> bool {
        self.found.contains(&fact)
    }

    pub fn knows_all(&self, facts: &[FactId]) -> bool {
        facts.iter().all(|fact| self.knows(*fact))
    }

    /// The found facts, oldest first.
    pub fn facts(&self) -> &[FactId] {
        &self.found
    }

    /// The facts found since the night began.
    pub fn fresh(&self) -> &[FactId] {
        &self.fresh
    }

    /// How many nights have ended.
    pub fn loops(&self) -> u32 {
        self.loops
    }

    /// Ends a night: counts it, forgets which facts were new and unlocks every draft.
    pub fn end_night(&mut self) {
        self.loops += 1;
        self.fresh.clear();
        for drafts in &mut self.drafts {
            for draft in drafts.values_mut() {
                draft.locked = false;
            }
        }
    }

    /// Restores a saved casebook.
    pub fn restored(found: Vec<FactId>, confirmed: Vec<DeductionId>, loops: u32) -> Self {
        Self {
            found,
            confirmed,
            loops,
            ..Self::default()
        }
    }

    pub fn is_confirmed(&self, deduction: DeductionId) -> bool {
        self.confirmed.contains(&deduction)
    }

    /// Confirms `deduction` without a joint lock, for the debug tools.
    pub fn force_confirm(&mut self, deduction: DeductionId) {
        if !self.is_confirmed(deduction) {
            self.confirmed.push(deduction);
        }
    }

    pub fn confirmed(&self) -> &[DeductionId] {
        &self.confirmed
    }

    /// Whether a found fact has unlocked `term`.
    pub fn term_unlocked(&self, case: &Case, term: TermId) -> bool {
        self.found
            .iter()
            .any(|fact| case.facts.get(*fact).unlocks_terms.contains(&term))
    }

    /// The terms of `bank` the players can pick from.
    pub fn bank_options(&self, case: &Case, bank: TermBankId) -> Vec<TermId> {
        case.term_banks
            .get(bank)
            .terms
            .iter()
            .copied()
            .filter(|term| self.term_unlocked(case, *term))
            .collect()
    }

    fn is_statement(case: &Case, deduction: DeductionId) -> bool {
        case.finale
            .statements
            .iter()
            .any(|statement| statement.deduction == deduction)
    }

    /// The deductions the casebook shows, in the case's order.
    pub fn visible_deductions(&self, case: &Case) -> Vec<DeductionId> {
        case.deductions
            .ids()
            .filter(|id| {
                let deduction = case.deductions.get(*id);
                let by_fact = deduction.appears_with.iter().any(|fact| self.knows(*fact));
                let unlockers: Vec<DeductionId> = case
                    .deductions
                    .ids()
                    .filter(|other| case.deductions.get(*other).unlocks.contains(id))
                    .collect();
                let by_unlock = unlockers.iter().any(|other| self.is_confirmed(*other));
                let from_start = deduction.appears_with.is_empty()
                    && unlockers.is_empty()
                    && !Self::is_statement(case, *id);
                by_fact || by_unlock || from_start || self.is_confirmed(*id)
            })
            .collect()
    }

    /// Whether every deduction the finale needs is confirmed.
    pub fn finale_ready(&self, case: &Case) -> bool {
        case.deductions
            .ids()
            .filter(|id| case.deductions.get(*id).prerequisite_for_finale)
            .all(|id| self.is_confirmed(id))
    }

    /// `player`'s copy of `deduction`, empty until they pick something.
    pub fn draft(&self, case: &Case, player: u8, deduction: DeductionId) -> Draft {
        self.drafts[player as usize]
            .get(&deduction)
            .cloned()
            .unwrap_or_else(|| Draft {
                picks: vec![None; case.deductions.get(deduction).blanks.len()],
                locked: false,
            })
    }

    /// Steps `blank` of `player`'s copy to the next or previous option of its bank, through "nothing"; a locked or confirmed copy stays.
    pub fn cycle(
        &mut self,
        case: &Case,
        player: u8,
        deduction: DeductionId,
        blank: usize,
        forward: bool,
    ) {
        if self.is_confirmed(deduction) {
            return;
        }
        let mut draft = self.draft(case, player, deduction);
        let Some(pick) = draft.picks.get_mut(blank).filter(|_| !draft.locked) else {
            return;
        };
        let bank = case.deductions.get(deduction).blanks[blank].bank;
        let mut options: Vec<Option<TermId>> = vec![None];
        options.extend(self.bank_options(case, bank).into_iter().map(Some));
        let at = options
            .iter()
            .position(|option| option == pick)
            .unwrap_or(0);
        let step = if forward { 1 } else { options.len() - 1 };
        *pick = options[(at + step) % options.len()];
        self.drafts[player as usize].insert(deduction, draft);
    }

    /// Locks or unlocks `player`'s copy; once the copy of every `present` player is locked, judges them, confirms a correct one and unlocks the rest.
    pub fn toggle_lock(
        &mut self,
        case: &Case,
        player: u8,
        deduction: DeductionId,
        present: [bool; PLAYER_COUNT as usize],
    ) -> Option<Verdict> {
        if self.is_confirmed(deduction) {
            return None;
        }
        let mut draft = self.draft(case, player, deduction);
        draft.locked = !draft.locked;
        self.drafts[player as usize].insert(deduction, draft);
        let drafts: Vec<Draft> = (0..PLAYER_COUNT)
            .filter(|other| *other == player || present[*other as usize])
            .map(|player| self.draft(case, player, deduction))
            .collect();
        if !drafts.iter().all(|draft| draft.locked) {
            return None;
        }
        let agreed = drafts.windows(2).all(|pair| pair[0].picks == pair[1].picks);
        let wrong = case
            .deductions
            .get(deduction)
            .blanks
            .iter()
            .zip(&drafts[0].picks)
            .filter(|(blank, pick)| **pick != Some(blank.answer))
            .count();
        let verdict = match (agreed, wrong) {
            (false, _) => Verdict::Disagree,
            (true, 0) => Verdict::Correct,
            (true, 1 | 2) => Verdict::Close,
            (true, _) => Verdict::Wrong,
        };
        if verdict == Verdict::Correct {
            self.confirmed.push(deduction);
        }
        for drafts in &mut self.drafts {
            if let Some(draft) = drafts.get_mut(&deduction) {
                draft.locked = false;
            }
        }
        Some(verdict)
    }

    /// The deduction's sentence with each blank shown as `player`'s pick, or as underscores.
    pub fn sentence(&self, case: &Case, player: u8, deduction: DeductionId) -> String {
        let template = case
            .strings
            .get(case.deductions.get(deduction).template)
            .as_str();
        let draft = self.draft(case, player, deduction);
        let mut picks = draft.picks.iter();
        let mut sentence = String::new();
        let mut rest = template;
        while let Some(start) = rest.find("___") {
            sentence.push_str(&rest[..start]);
            let run = rest[start..].chars().take_while(|c| *c == '_').count();
            match picks.next().copied().flatten() {
                Some(term) => {
                    sentence.push('[');
                    sentence.push_str(case.strings.get(case.terms.get(term).text).as_str());
                    sentence.push(']');
                }
                None => sentence.push_str("[____]"),
            }
            rest = &rest[start + run..];
        }
        sentence.push_str(rest);
        sentence
    }
}

/// The index a saved id has in a table of `len` rows, if it still exists.
pub fn saved_id<I: Id>(index: usize, len: usize) -> Option<I> {
    (index < len).then(|| I::from_index(index))
}

#[cfg(test)]
mod tests {
    use super::{Casebook, Verdict};
    use crate::casefile::CaseData;

    const BOTH: [bool; 2] = [true, true];

    #[test]
    fn a_player_alone_judges_their_own_copy() {
        let data = CaseData::test();
        let case = &data.case;
        let d1 = case.deductions.id("d_01").unwrap();
        let mut book = Casebook::default();

        assert_eq!(
            book.toggle_lock(case, 0, d1, [true, false]),
            Some(Verdict::Close)
        );
    }

    #[test]
    fn facts_unlock_terms_and_deductions_and_a_joint_lock_judges_them() {
        let data = CaseData::test();
        let case = &data.case;
        let fact = |name: &str| case.facts.id(name).unwrap();
        let deduction = |name: &str| case.deductions.id(name).unwrap();
        let term = |name: &str| case.terms.id(name).unwrap();
        let mut book = Casebook::default();

        assert!(book.visible_deductions(case).is_empty());
        assert!(book.grant(fact("f_01")));
        assert!(!book.grant(fact("f_01")), "a fact is added once");
        assert_eq!(book.facts(), [fact("f_01")]);
        assert_eq!(book.visible_deductions(case), [deduction("d_01")]);

        let d1 = deduction("d_01");
        let things = case.deductions.get(d1).blanks[1].bank;
        assert_eq!(book.bank_options(case, things), [term("t_box")]);
        assert_eq!(
            book.sentence(case, 0, d1),
            "[____] took [____] from the study."
        );

        book.cycle(case, 0, d1, 1, true);
        assert_eq!(book.draft(case, 0, d1).picks, [None, Some(term("t_box"))]);
        assert_eq!(
            book.toggle_lock(case, 0, d1, BOTH),
            None,
            "one lock waits for the other"
        );
        book.cycle(case, 0, d1, 1, true);
        assert_eq!(
            book.draft(case, 0, d1).picks,
            [None, Some(term("t_box"))],
            "a locked copy does not change"
        );
        assert_eq!(book.toggle_lock(case, 1, d1, BOTH), Some(Verdict::Disagree));
        assert!(!book.draft(case, 0, d1).locked, "a verdict unlocks both");

        book.cycle(case, 1, d1, 1, true);
        book.toggle_lock(case, 0, d1, BOTH);
        assert_eq!(book.toggle_lock(case, 1, d1, BOTH), Some(Verdict::Close));

        book.grant(fact("f_02"));
        for player in 0..2 {
            book.cycle(case, player, d1, 0, true);
        }
        assert_eq!(
            book.sentence(case, 1, d1),
            "[Mr Corwin] took [the snuffbox] from the study."
        );
        book.toggle_lock(case, 0, d1, BOTH);
        assert_eq!(book.toggle_lock(case, 1, d1, BOTH), Some(Verdict::Correct));
        assert!(book.is_confirmed(d1));
        assert_eq!(
            book.visible_deductions(case),
            [d1, deduction("d_02")],
            "a confirmed deduction shows what it unlocks, and never a finale statement"
        );
        assert!(!book.finale_ready(case));

        book.end_night();
        assert_eq!(book.loops(), 1);
        assert!(book.fresh().is_empty());
        assert_eq!(book.facts().len(), 2, "facts survive the night");
    }
}
