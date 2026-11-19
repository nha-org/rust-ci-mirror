//! The CI router's tier order with the Hostman runner in it (task rust-hostman-ci-runner; @nha, 2026-10-30: « Mac is a
//! FALLBACK. », 2026-11-14: « use hostman as I asked »; replaces 2026-11-04's whatsnr VPS tier). Only the pick is here:
//! each CI's room is read by whoever calls it.
//! `dash ci route` is answered here since task rust-hostman-ci-wire, the front door handing in the older app's free pick.
//! ponytail: each free tier's room is still `Dashboard.CiRouter`'s count; task rust-ci-router-port ports it, then `older` goes.

/// His order: free tiers first, then our own runner, the Mac last. No `vps`: the whatsnr box's runner is dropped.
pub const TIERS: [&str; 5] = ["expiring", "monthly", "credits", "hostman", "mac"];

#[derive(Clone, Debug, PartialEq)]
pub struct Ci {
    pub id: String,
    pub tier: String,
    pub room: bool,
    pub labels: Vec<String>,
}

/// The first tier with a CI that has room; a CI of a tier not in `TIERS` is never picked. The Mac is the fallback: when none
/// has room, it is returned whatever its own room; with no Mac listed, none (the caller's list always holds it).
pub fn pick(cis: &[Ci]) -> Option<&Ci> {
    TIERS
        .iter()
        .find_map(|t| cis.iter().find(|c| c.tier == *t && c.room))
        .or_else(|| cis.iter().find(|c| c.tier == "mac"))
}

/// `dash ci route [--json] [--actions] [--branch <name>]`'s words: whether JSON is asked, and the branch (main when none);
/// none for any other words, which the older app answers as before.
pub fn asked(args: &[String]) -> Option<(bool, String)> {
    let [ci, route, rest @ ..] = args else {
        return None;
    };
    let (mut json, mut branch, mut words) = (false, None, rest.iter());
    while let Some(w) = words.next() {
        match w.as_str() {
            "--json" => json = true,
            "--actions" => {}
            "--branch" => branch = Some(words.next()?.clone()),
            _ => return None,
        }
    }
    let branch = branch.filter(|b| !b.is_empty()).unwrap_or("main".into());
    (ci == "ci" && route == "route").then_some((json, branch))
}

/// The older app's pick as a CI, with its `why`: from its JSON, or its text's first line. Its whatsnr VPS and its Mac are no
/// free tier, so neither is one: the Hostman runner takes the VPS's place, the Mac stays last.
fn free(answer: &str) -> Option<(Ci, String)> {
    let (id, why, labels) = match serde_json::from_str::<serde_json::Value>(answer) {
        Ok(j) => {
            let labels = (j["runs-on"].as_array().into_iter().flatten())
                .filter_map(|l| l.as_str().map(String::from))
                .collect();
            let (id, why) = (j["provider"].as_str()?, j["why"].as_str()?);
            (id.to_string(), why.to_string(), labels)
        }
        Err(_) => {
            let (id, why) = answer.lines().next()?.split_once(": ")?;
            (id.to_string(), why.to_string(), vec![])
        }
    };
    if ["vps", "mac"].contains(&id.as_str()) {
        return None;
    }
    // the tier its `why` ends on (« ..., monthly tier »); a free pick that names none is still a free tier's
    let named = TIERS[..3]
        .iter()
        .find(|t| why.contains(&format!("{t} tier")));
    let tier = named.unwrap_or(&"credits").to_string();
    Some((
        Ci {
            id,
            tier,
            room: true,
            labels,
        },
        why,
    ))
}

/// One CI the router lists beside the older app's pick.
fn ours(id: &str, room: bool, labels: &[&str]) -> Ci {
    Ci {
        id: id.into(),
        tier: id.into(),
        room,
        labels: labels.iter().map(|l| l.to_string()).collect(),
    }
}

/// The pick (task rust-hostman-ci-wire): the older app's free tiers first, their room its count until task
/// rust-ci-router-port ports it, then the Hostman runner when the job last saw it up, the Mac last. `older`: its answer to
/// the same words; `up`: the runner's state. Out: define-ci's JSON, or the text: the pick, the older app's other lines but
/// its VPS's, then the runner's state.
pub fn answer(
    json: bool,
    branch: &str,
    older: Result<String, String>,
    up: Result<bool, String>,
) -> (i32, String, String) {
    let older_free = older.as_deref().ok().and_then(free);
    let cis: Vec<Ci> = (older_free.iter().map(|(c, _)| c.clone()))
        .chain([
            ours("hostman", up == Ok(true), &[super::LABEL]),
            ours("mac", true, &["self-hosted", "site-perso"]),
        ])
        .collect();
    let picked = pick(&cis).expect("the Mac is listed");
    let runner = match &up {
        Ok(true) => "up".to_string(),
        Ok(false) => "not up".to_string(),
        Err(e) => format!("unread: {e}"),
    };
    let (image, why) = match picked.id.as_str() {
        "hostman" => (
            "debian-13",
            format!("the Hostman runner for {branch}: up, hostman tier"),
        ),
        "mac" => (
            "macos",
            format!(
                "this Mac: no free tier has room for {branch} and the Hostman runner is {runner}"
            ),
        ),
        _ => (
            "",
            older_free
                .as_ref()
                .map(|(_, w)| w.clone())
                .unwrap_or_default(),
        ),
    };
    let tiers: String = match &older {
        Ok(text) if !json => (text.lines().skip(1))
            .filter(|l| !l.contains("whatsnr VPS"))
            .map(|l| format!("{l}\n"))
            .collect(),
        Err(e) => format!("- the free tiers: unread, the older app gave no pick: {e}\n"),
        Ok(_) => String::new(),
    };
    let out = match (json, picked.id.as_str()) {
        (true, "hostman" | "mac") => {
            let j = serde_json::json!({"runs-on": picked.labels, "image": image, "provider": picked.id, "why": why});
            format!("{j}\n")
        }
        // a free tier's: the older app's own JSON, as define-ci read it before
        (true, _) => format!("{}\n", older.unwrap_or_default().trim()),
        (false, id) => format!("{id}: {why}\n{tiers}- the Hostman runner: {runner}\n"),
    };
    // JSON on stdout alone: define-ci reads it whole, so what the text would say goes on stderr
    (0, out, if json { tiers } else { String::new() })
}
