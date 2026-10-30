use std::collections::HashSet;

fn sanitise(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control()
                || matches!(
                    c,
                    '/' | '\\' | '#' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    replaced
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string()
}

/// Returns one sanitised, case-insensitively unique name per requested clip name, falling back to the clip index.
pub fn clip_names(requested: &[Option<&str>]) -> Vec<String> {
    let mut taken: HashSet<String> = HashSet::new();
    let mut names = Vec::with_capacity(requested.len());
    for (index, name) in requested.iter().enumerate() {
        let mut candidate = name.map(sanitise).unwrap_or_default();
        if candidate.is_empty() {
            candidate = index.to_string();
        }
        while !taken.insert(candidate.to_lowercase()) {
            candidate = format!("{candidate}_{index}");
        }
        names.push(candidate);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_plain_names() {
        assert_eq!(
            clip_names(&[Some("Idle_Loop"), Some("Jog Fwd")]),
            vec!["Idle_Loop", "Jog Fwd"]
        );
    }

    #[test]
    fn sanitises_reserved_characters() {
        assert_eq!(
            clip_names(&[Some("a/b\\c#d:e*f?g\"h<i>j|k\tl")]),
            vec!["a_b_c_d_e_f_g_h_i_j_k_l"]
        );
    }

    #[test]
    fn trims_whitespace_and_dots() {
        assert_eq!(clip_names(&[Some("  .Run. ")]), vec!["Run"]);
    }

    #[test]
    fn missing_and_empty_names_use_the_index() {
        assert_eq!(
            clip_names(&[Some("A"), None, Some(""), Some("..."), Some(" ")]),
            vec!["A", "1", "2", "3", "4"]
        );
    }

    #[test]
    fn duplicate_gets_index_suffix() {
        assert_eq!(
            clip_names(&[Some("Run"), Some("Walk"), Some("Run")]),
            vec!["Run", "Walk", "Run_2"]
        );
    }

    #[test]
    fn duplicate_is_case_insensitive() {
        assert_eq!(
            clip_names(&[Some("Run"), Some("run")]),
            vec!["Run", "run_1"]
        );
    }

    #[test]
    fn name_equal_to_earlier_fallback_is_suffixed() {
        assert_eq!(clip_names(&[None, Some("0")]), vec!["0", "0_1"]);
    }

    #[test]
    fn name_taken_by_later_fallback_keeps_the_earlier_claim() {
        assert_eq!(clip_names(&[Some("1"), None]), vec!["1", "1_1"]);
    }

    #[test]
    fn colliding_suffix_is_extended() {
        assert_eq!(
            clip_names(&[Some("Run"), Some("Run_2"), Some("Run")]),
            vec!["Run", "Run_2", "Run_2_2"]
        );
    }
}
