//! Each roadmap item's measure and bound, red on /plan and one wake when crossed (task roadmap-measures;
//! incident-2026-11-03-arrow-not-ready section 6 row 8; @nha, 2026-11-03: « see if we did put this in the dash directly it
//! would have told us we're at risk of not being ready »). An Urgent item with no task linked or no definition of done is the
//! first alarm; a measure (`crate::tasks::MEASURES`) read past its bound is the next. The stall watcher runs `pass` on
//! the open PRs it lists; /plan draws `alarms`.
use crate::older::RoadmapItem;
use crate::tasks::Plan;
use std::collections::HashMap;
use std::path::Path;

/// The horizons in their order, each its group's name on /plan.
pub const HORIZONS: [(&str, &str); 4] = [
    ("urgent", "Urgent"),
    ("now", "Now"),
    ("next", "Next"),
    ("later", "Later"),
];

/// ponytail: his key's user leads every project (`Dashboard.Focus`'s @lead), the one member of the workspace; a `lead` in the
/// plan once someone else leads one.
const LEAD: &str = "Nicolas Ha";

/// A roadmap item with its plan and what Linear's project panel shows of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub item: RoadmapItem,
    pub plan: Plan,
}

impl Item {
    /// Every live task done, and at least one.
    pub fn complete(&self) -> bool {
        self.item.points > 0 && self.item.done == self.item.points
    }

    /// Linear's project status: completed once every live task is done, else its horizon's, as `Focus.project/1`.
    pub fn status(&self) -> &'static str {
        match (self.complete(), self.item.horizon.as_str()) {
            (true, _) => "Completed",
            (_, "urgent" | "now") => "In Progress",
            (_, "next") => "Planned",
            _ => "Backlog",
        }
    }

    /// Linear's project priority, from its horizon: Urgent is urgent, Now high, Next medium, Later low.
    pub fn priority(&self) -> &'static str {
        match self.item.horizon.as_str() {
            "urgent" => "Urgent",
            "now" => "High",
            "next" => "Medium",
            _ => "Low",
        }
    }

    pub fn lead(&self) -> &'static str {
        LEAD
    }

    /// Its start: the plan's, else the day its first task was added.
    pub fn start(&self) -> Option<&str> {
        self.plan.start.as_deref().or(self.item.first.as_deref())
    }

    /// Its target: the plan's, else, once complete, the day its last task was done.
    pub fn target(&self) -> Option<&str> {
        let done = self.complete().then_some(self.item.last.as_deref());
        self.plan.target.as_deref().or(done.flatten())
    }

    /// Its done points of its points, floored: 199 of 200 is 99 %, never 100 % with a task left. None with no task.
    pub fn progress(&self) -> Option<i64> {
        (self.item.points > 0).then(|| self.item.done * 100 / self.item.points)
    }

    pub fn progress_said(&self) -> String {
        match self.progress() {
            Some(p) => format!("{p} % ({} of {} points)", self.item.done, self.item.points),
            None => "No task yet".into(),
        }
    }

    /// Its RFC: its `rfc`, else its `link` when that is an RFC page, as `Focus.rfc/1`.
    pub fn rfc(&self) -> Option<&str> {
        let link = self.item.link.as_deref().filter(|l| {
            let page = l.split(['#', '?']).next().unwrap_or_default();
            page.rsplit('/')
                .next()
                .is_some_and(|f| f.starts_with("rfc-") && f.ends_with(".html"))
        });
        self.item.rfc.as_deref().or(link)
    }

    /// The days its bar is drawn on: a target alone is a one-day bar, never a milestone's diamond; an unfinished item with no
    /// target runs to today, as Linear draws a project with none; a start after its target starts on it (/code-review).
    pub fn drawn(&self, today: i64) -> (Option<String>, Option<String>) {
        let today = dash_core::date(today);
        let target = (self.target().map(String::from))
            .or_else(|| (!self.complete() && self.start().is_some()).then(|| today.clone()));
        let start = self.start().map(String::from).or_else(|| target.clone());
        match (start, target) {
            (Some(s), Some(t)) if s > t => (Some(t.clone()), Some(t)),
            (s, t) => (s, t),
        }
    }

    /// A milestone's mark: done once the item is, failed past its day before that, else next. One reading for the Gantt and
    /// the panel (/code-review: a past day read as done).
    pub fn milestone(&self, day: &str, today: i64) -> &'static str {
        match (
            self.complete(),
            dash_core::days(day).is_some_and(|d| d < today),
        ) {
            (true, _) => "done",
            (_, true) => "failed",
            _ => "next",
        }
    }

    pub fn group(&self) -> &'static str {
        HORIZONS
            .iter()
            .find(|(h, _)| *h == self.item.horizon)
            .map_or("Later", |(_, g)| g)
    }
}

/// Every item in the roadmap's order, each with its plan.
pub fn items(older: &Path, plans: &HashMap<String, Plan>) -> Result<Vec<Item>, String> {
    let read = crate::older::roadmap_items(older)?;
    Ok(read
        .into_iter()
        .map(|item| {
            let plan = plans.get(&item.id).cloned().unwrap_or_default();
            Item { item, plan }
        })
        .collect())
}

/// An item past what it may be: its id, what crossed (`no-task`, `no-dod` or its measure), the words /plan and the wake say.
#[derive(Debug, PartialEq)]
pub struct Alarm {
    pub id: String,
    pub what: String,
    pub said: String,
}

/// Each item's alarms: an Urgent one with no task linked or no definition of done, and any whose measure read past its bound
/// at the last pass (`Plan::seen`).
pub fn alarms(items: &[Item]) -> Vec<Alarm> {
    let mut out = vec![];
    for Item { item: i, plan } in items {
        let (id, title) = (&i.id, &i.title);
        let mut say = |what: &str, said: String| {
            let (id, what) = (id.clone(), what.into());
            out.push(Alarm { id, what, said })
        };
        let urgent = i.horizon == "urgent";
        if urgent && i.tasks == 0 {
            say(
                "no-task",
                format!("Urgent {title} ({id}) has no task linked"),
            );
        }
        if urgent && i.dod.as_deref().is_none_or(|d| d.trim().is_empty()) {
            say(
                "no-dod",
                format!("Urgent {title} ({id}) has no definition of done"),
            );
        }
        if let (Some(m), Some(b), Some(n)) = (&plan.measure, plan.bound, plan.seen)
            && n > b
        {
            say(
                m,
                format!("{title} ({id}): {m} reads {n}, over its bound of {b}"),
            );
        }
    }
    out
}

/// Fields written to an item's plan, as `SqliteTasks::plan` takes them.
pub type Write<'a> = dyn FnMut(&str, &[(&str, &str)]) -> Result<(), String> + 'a;

/// One pass: `open_prs`, site_perso's open PRs counted now, read by each item measuring them. Each alarm not yet told
/// (`Plan::told`) goes to `wake` keyed by `now_ms`, which lands a row that wakes the orchestrator (dash_comments::watch::land);
/// then `write` keeps the item's reading and what is told, so a wake that fails is tried again next pass. An alarm that ends
/// leaves `told`, so it wakes again when it comes back: one wake per crossing, whatever moved (the reading, the bound, the
/// item's tasks or done-when). The words that woke.
pub fn pass(
    items: &[Item],
    open_prs: i64,
    now_ms: i64,
    write: &mut Write,
    wake: &mut dyn FnMut(&str, &str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let mut woke = vec![];
    for i in items {
        let mut now = i.clone();
        if now.plan.measure.as_deref() == Some("open-prs") {
            now.plan.seen = Some(open_prs);
        }
        let mut told = vec![];
        for a in alarms(std::slice::from_ref(&now)) {
            if !i.plan.told.contains(&a.what) {
                wake(&format!("roadmap {} {} {now_ms}", a.id, a.what), &a.said)?;
                woke.push(a.said);
            }
            told.push(a.what);
        }
        let (seen, told) = (now.plan.seen.map(|n| n.to_string()), told.join(" "));
        let mut fields = vec![];
        if now.plan.seen != i.plan.seen {
            fields.push(("seen", seen.as_deref().unwrap_or_default()));
        }
        if told != i.plan.told.join(" ") {
            fields.push(("told", told.as_str()));
        }
        if !fields.is_empty() {
            write(&i.item.id, &fields)?;
        }
    }
    Ok(woke)
}

/// The live pass: the older app's items in `older`, their plans in `tasks`, each new reading written to its plan.
pub fn run(
    tasks: &mut crate::tasks::SqliteTasks,
    older: &Path,
    open_prs: i64,
    now_ms: i64,
    wake: &mut dyn FnMut(&str, &str) -> Result<bool, String>,
) -> Result<Vec<String>, String> {
    let items = items(older, &tasks.plans()?)?;
    let mut write = |id: &str, fields: &[(&str, &str)]| tasks.plan(id, fields);
    pass(&items, open_prs, now_ms, &mut write, wake)
}
