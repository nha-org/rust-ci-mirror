use super::*;
use dash_meter::dash_test;

/// A step's verdict, as `Dashboard.Handoff.verdict/3` gives it: a line that is no JSON refused, no closer refused, a past
/// `until` closed, a later `from` later, a judge older than a week stale, his answer last answered.
#[dash_test]
fn each_step_reads_its_verdict() {
    let f = Facts::default();
    let one = |body: &str| {
        verdict(
            &steps(&format!("- [ ] **Fais.** [x](/y)\n{body}"))[0],
            "2026-11-13",
            &f,
        )
    };
    assert_eq!(
        one("  {:check}"),
        Verdict::Refused("unreadable check line: {:check}".into())
    );
    assert_eq!(
        one(r#"  {"judged":"2026-11-13"}"#),
        Verdict::Refused("no :check nor :until".into())
    );
    assert_eq!(
        one(r#"  {"until":"2026-11-12"}"#),
        Verdict::Closed("expiré le 2026-11-12".into())
    );
    assert_eq!(
        one(r#"  {"task":"t","from":"2026-11-20"}"#),
        Verdict::Later("2026-11-20".into())
    );
    assert_eq!(
        one(r#"  {"task":"t","judged":"2026-11-01"}"#),
        Verdict::Stale("2026-11-01".into())
    );
    assert_eq!(
        one(r#"  {"task":"t","judged":"2026-11-13"}"#),
        Verdict::Open
    );
    assert_eq!(
        one("  {\"task\":\"t\",\"judged\":\"2026-11-13\"}\n  @nha: fait"),
        Verdict::Answered
    );
    assert_eq!(
        one(r#"  {"task":"t","waits":{"reply-from":"a@b.c d@e.f","after":"2026-11-01"}}"#),
        Verdict::Refused(
            r#"a {"waits": …} needs "reply-from" one address and "after" a day"#.into()
        )
    );
}

/// The page's faults and rules: a step with no task, a duplicate, an interview without its date, a long title.
#[dash_test]
fn a_step_breaking_the_page_s_rules_is_named() {
    let text = "- [ ] **Prépare l'entretien avec Acme, le plus tôt possible cette semaine.** [x](/y)\n  {\"until\":\"2099-01-01\"}\n\
                - [ ] **Prépare l'entretien avec Acme, le plus tôt possible cette semaine.** [x](/y)\n  {\"task\":\"t\",\"until\":\"2099-01-01\"}\n";
    let mut f = Facts::default();
    f.tasks.insert("t".into(), ("open".into(), "claude".into()));
    let faults = faults(text, &f, "2026-11-13");
    let has = |w: &str| faults.iter().any(|x| x.contains(w));
    assert!(
        has("line 1: no :task names the task it belongs to"),
        "{faults:?}"
    );
    assert!(has("line 3: duplicate of another open step"), "{faults:?}");
    assert!(
        has("names an interview, call or deadline without its date"),
        "{faults:?}"
    );
    let open: Vec<Step> = steps(text);
    let w = warnings(&open, &f);
    assert!(
        w.iter()
            .any(|x| x.contains("step title over 50 characters")),
        "{w:?}"
    );
    assert!(
        w.iter()
            .any(|x| x.contains("step on t, a task claude owns, not him")),
        "{w:?}"
    );
    assert!(!undated(&steps("- [ ] **Appel avec Ana le 23/10.**\n")[0]));
}

/// `said`: a placeholder stands for one to four words, never a negation; a sentence under 8 letters tells nothing.
#[dash_test]
fn a_placeholder_never_stands_for_a_negation() {
    assert_eq!(
        said(
            "je suis <dispo> pour un appel",
            &plain("Je suis disponible pour un appel.")
        ),
        Some(true)
    );
    assert_eq!(
        said(
            "je suis <dispo> pour un appel",
            &plain("Je suis pas disponible pour un appel")
        ),
        Some(false)
    );
    assert_eq!(said("oui <x>", "oui merci"), None);
    assert_eq!(
        sentences("Bonjour **Ana**. nicolas-ha.com\nMerci !"),
        ["bonjour ana", "nicolas ha com", "merci"]
    );
}
