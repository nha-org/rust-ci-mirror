//! The one reader of a green state: a leaf both `checks` and `mirror` depend on, so neither depends on the other for it.

/// What dash ci wait and the merge on green count as passed.
pub const GREEN: [&str; 3] = ["SUCCESS", "NEUTRAL", "SKIPPED"];

/// The one reader of a green state, whatever case a source writes it in (a rollup's SUCCESS, a check run's `success`): every
/// verdict in this crate (the hook's rows, the mirror's runs, Forgejo's) asks here, so none holds a list of its own.
pub fn is_green(state: &str) -> bool {
    GREEN.iter().any(|g| g.eq_ignore_ascii_case(state))
}

/// A green state that ran: SKIPPED and NEUTRAL pass beside a run that passed, never alone.
pub fn ran(state: &str) -> bool {
    state.eq_ignore_ascii_case("SUCCESS")
}

/// The one verdict of a green head, `dash ci wait`'s, the merge on green's and the review's: every check green and one of
/// them ran. A head of skipped or neutral checks alone, or of none, is not green yet (task rust-ci-wait-skipped: `dash ci
/// wait 1879` said green on Blacksmith's lone skipped Proof, before any workflow of the head ran).
pub fn passed(rows: &[(String, String)]) -> bool {
    rows.iter().all(|(_, s)| is_green(s)) && rows.iter().any(|(_, s)| ran(s))
}

#[cfg(test)]
mod one_classifier {
    use dash_meter::dash_test;

    #[dash_test]
    fn green_reads_either_case_and_only_this_module_lists_it() {
        assert!(
            super::is_green("SUCCESS") && super::is_green("success") && super::is_green("Skipped")
        );
        assert!(!super::is_green("FAILURE") && !super::is_green("PENDING") && !super::is_green(""));
        let rows = |r: &[(&str, &str)]| -> Vec<(String, String)> {
            r.iter()
                .map(|(n, s)| (n.to_string(), s.to_string()))
                .collect()
        };
        assert!(!super::passed(&rows(&[])) && !super::passed(&rows(&[("Proof", "SKIPPED")])));
        assert!(!super::passed(&rows(&[
            ("a", "NEUTRAL"),
            ("Proof", "SKIPPED")
        ])));
        assert!(super::passed(&rows(&[
            ("Proof", "SKIPPED"),
            ("test", "success")
        ])));
        assert!(!super::passed(&rows(&[
            ("test", "SUCCESS"),
            ("superapp", "UNSTARTED")
        ])));
        // The gate count: a second list of green states in this crate is a second classifier (PRs 1654 and 1655 crossed on one).
        let src = std::fs::read_dir(env!("CARGO_MANIFEST_DIR").to_string() + "/src").expect("src");
        let lists = src
            .filter_map(Result::ok)
            .filter(|f| f.path().extension().is_some_and(|e| e == "rs"))
            .filter(|f| {
                std::fs::read_to_string(f.path())
                    .expect("read")
                    .contains("const GREEN")
            })
            .count();
        assert_eq!(lists, 1, "one `const GREEN` in dash-forge, green.rs's");
    }
}
