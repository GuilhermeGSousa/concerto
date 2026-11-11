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
    use proptest::prelude::*;

    struct Case {
        case: &'static str,
        requested: &'static [Option<&'static str>],
        names: &'static [&'static str],
    }

    #[test]
    fn each_clip_gets_a_sanitised_unique_name_or_its_index() {
        let cases = [
            Case {
                case: "plain names",
                requested: &[Some("Idle_Loop"), Some("Jog Fwd")],
                names: &["Idle_Loop", "Jog Fwd"],
            },
            Case {
                case: "reserved characters",
                requested: &[Some("a/b\\c#d:e*f?g\"h<i>j|k\tl")],
                names: &["a_b_c_d_e_f_g_h_i_j_k_l"],
            },
            Case {
                case: "surrounding whitespace and dots",
                requested: &[Some("  .Run. ")],
                names: &["Run"],
            },
            Case {
                case: "missing and empty names",
                requested: &[Some("A"), None, Some(""), Some("..."), Some(" ")],
                names: &["A", "1", "2", "3", "4"],
            },
            Case {
                case: "a duplicate",
                requested: &[Some("Run"), Some("Walk"), Some("Run")],
                names: &["Run", "Walk", "Run_2"],
            },
            Case {
                case: "a duplicate in another case",
                requested: &[Some("Run"), Some("run")],
                names: &["Run", "run_1"],
            },
            Case {
                case: "a name equal to an earlier fallback",
                requested: &[None, Some("0")],
                names: &["0", "0_1"],
            },
            Case {
                case: "a fallback equal to an earlier name",
                requested: &[Some("1"), None],
                names: &["1", "1_1"],
            },
            Case {
                case: "a suffix that collides",
                requested: &[Some("Run"), Some("Run_2"), Some("Run")],
                names: &["Run", "Run_2", "Run_2_2"],
            },
        ];
        for case in &cases {
            assert_eq!(clip_names(case.requested), case.names, "{}", case.case);
        }
    }

    proptest! {
        #[test]
        fn clip_names_are_one_per_clip_non_empty_unique_and_free_of_reserved_characters(
            requested in prop::collection::vec(prop::option::of("[aA0-9_ ./#:]{0,4}"), 0..12),
        ) {
            let requested: Vec<Option<&str>> = requested.iter().map(Option::as_deref).collect();
            let names = clip_names(&requested);
            prop_assert_eq!(names.len(), requested.len());
            let mut seen = HashSet::new();
            for name in &names {
                prop_assert!(!name.is_empty());
                prop_assert!(!name.contains(['/', '#', ':']), "{}", name);
                prop_assert!(seen.insert(name.to_lowercase()), "{} twice in {:?}", name, names);
            }
        }
    }
}
