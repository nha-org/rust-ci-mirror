//! The CI router's tier order with the Hostman runner in it (task rust-hostman-ci-runner; @nha, 2026-10-30: « Mac is a
//! FALLBACK. », 2026-11-14: « use hostman as I asked »; replaces 2026-11-04's whatsnr VPS tier). Only the pick is here:
//! each CI's room is read by whoever calls it.
//! ponytail: the Elixir `Dashboard.CiRouter` still serves `dash ci route`; its port (task rust-ci-router-port) reads this.

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
