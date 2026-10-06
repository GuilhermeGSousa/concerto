//! The words: night cards, Marrow's diary pages, the ending.

pub const TITLE: &str = "STILL LIFE";
pub const TAGLINE: &str = "they only move when you are not looking";

/// The card before each night: a heading and the clerk's own notes.
pub fn intro(night: u32) -> (&'static str, &'static str) {
    match night {
        1 => (
            "The First Night",
            "Marrow House, Bloomsbury. The 14th of November, 1893.\n\n\
             Mr Pike has sent me to catalogue the late Mr Marrow's pictures\n\
             before the sale. The house has stood shut for a year.\n\
             He says it will take five nights. He did not say why by night.",
        ),
        2 => (
            "The Second Night",
            "The figures were not where I left them.\n\n\
             Mr Pike says I am tired, and that the porters moved them.\n\
             The porters will not come to Marrow House.",
        ),
        3 => (
            "The Third Night",
            "They say at the Lamb that after his wife died, Marrow painted\n\
             nothing but his figures, and that he spoke to them.\n\n\
             They say he was never found.",
        ),
        4 => (
            "The Fourth Night",
            "I have stopped closing the lantern.\n\
             I have started counting them.\n\n\
             Last night there was one more than there were.",
        ),
        5 => (
            "The Last Night",
            "The last lots are in her room.\n\n\
             Mr Pike will come for me at six.\n\
             If the ledger is not signed, he is to burn the house.",
        ),
        _ => (
            "Another Night",
            "Mr Pike did not come at six.\n\n\
             There are always more pictures.",
        ),
    }
}

/// The goal line under each card.
pub fn goal(lots: usize) -> String {
    format!("Catalogue the {lots} marked pictures, then sign the ledger in the hall.")
}

/// Marrow's diary page for the night.
pub fn page(night: u32) -> &'static str {
    match night {
        1 => {
            "12th March, 1889.\n\n\
             The figures came from Paris today, eleven of them, jointed at every limb.\n\
             Clara laughs that they sit better than the Duchess.\n\
             I have dressed one in her grey silk to paint the drape."
        }
        2 => {
            "3rd June, 1890.\n\n\
             They take a pose and hold it for a week. No sitter alive can do that.\n\
             Clara says she does not like the tall one watching her while she reads.\n\
             I have turned it to the wall. It does not stay turned."
        }
        3 => {
            "9th January, 1891.\n\n\
             Clara is dead. The doctor says her heart.\n\
             The house is quiet but for the figures, and I find\n\
             I cannot bear to put them away."
        }
        4 => {
            "Undated.\n\n\
             I have made one more. I carved her face from memory and could not paint it.\n\
             It sits in her room. When I come in, it has always just stopped moving.\n\
             They only move when I am not looking. So I look."
        }
        _ => {
            "Undated.\n\n\
             I have not slept. I have not slept.\n\
             If you are reading this, keep the lantern lit,\n\
             and do not turn your back on the one in grey."
        }
    }
}

/// After the fifth night.
pub const ENDING_TITLE: &str = "SOLD";
pub const ENDING: &str = "The sale was held on the 20th of November.\n\n\
     Lot 1. A lay figure, life-size, in a clerk's coat. Unusually fine.\n\
     Withdrawn.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_night_has_words() {
        for night in 1..=8 {
            assert!(!intro(night).1.is_empty());
            assert!(!page(night).is_empty());
        }
    }
}
