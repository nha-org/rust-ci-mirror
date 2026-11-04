//! Section 4's budgets, one row per state. private/dashboard/test/dashboard/rust_budgets_test.exs fails when this table
//! and the RFC's differ.
pub struct Budget {
    pub state: &'static str,
    /// As section 4 writes it.
    pub budget: &'static str,
    pub limit_mb: f64,
    /// A budget written « +60 MB » is on top of the server's own idle footprint.
    pub over_idle: bool,
    pub check: Check,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Check {
    /// tests/memory_budgets.rs holds it. Turning a row to this means writing that row's reading there: a test below
    /// counts the rows, so the table cannot claim a check alone.
    Now,
    /// The binary cannot reach this state yet; named by the task whose crate brings it (the orchestrator's answer in
    /// the task's dialog, 2026-11-05: faking the state with an allocation would measure nothing).
    With(&'static str),
}

pub const SERVER_IDLE: &str = "Server idle, no model loaded";

pub const BUDGETS: [Budget; 4] = [
    Budget {
        state: "Server idle, no model loaded",
        budget: "100 MB",
        limit_mb: 100.0,
        over_idle: false,
        check: Check::Now,
    },
    Budget {
        state: "With the embedder loaded",
        budget: "350 MB",
        limit_mb: 350.0,
        over_idle: false,
        check: Check::With("rust-vector-bench"),
    },
    Budget {
        state: "Native window open",
        budget: "+60 MB",
        limit_mb: 60.0,
        over_idle: true,
        check: Check::With("rust-ui-tree"),
    },
    Budget {
        state: "Transcribing (large model)",
        budget: "+4 GB, for the job only",
        limit_mb: 4096.0,
        over_idle: true,
        check: Check::With("rust-speech-bench"),
    },
];

/// The check every state's test calls, with the server's idle footprint for a budget written on top of it: past the
/// limit, an error that names the state and the number.
pub fn within(state: &str, measured_mb: f64, idle_mb: f64) -> Result<(), String> {
    let b = BUDGETS
        .iter()
        .find(|b| b.state == state)
        .ok_or(format!("no budget for {state}"))?;
    let used = if b.over_idle {
        measured_mb - idle_mb
    } else {
        measured_mb
    };
    if used > b.limit_mb {
        return Err(format!(
            "{state}: {used:.1} MB is past its budget of {}",
            b.budget
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_past_the_budget_names_the_state_and_the_number() {
        let _budget = crate::memory::TestBudget::new();
        assert!(within(SERVER_IDLE, 99.9, 0.0).is_ok());
        let e = within(SERVER_IDLE, 100.1, 0.0).expect_err("past it");
        assert!(
            e.contains(SERVER_IDLE) && e.contains("100.1 MB") && e.contains("100 MB"),
            "{e}"
        );
        assert!(within("a state nobody budgeted", 1.0, 0.0).is_err());
    }

    #[test]
    fn a_budget_written_on_top_of_idle_subtracts_it() {
        let _budget = crate::memory::TestBudget::new();
        assert!(within("Native window open", 85.0, 40.0).is_ok());
        assert!(within("Native window open", 101.0, 40.0).is_err());
    }

    #[test]
    fn every_state_not_checked_yet_names_the_task_that_brings_it() {
        let _budget = crate::memory::TestBudget::new();
        assert_eq!(BUDGETS.iter().filter(|b| b.check == Check::Now).count(), 1);
        assert!(BUDGETS.iter().all(|b| match b.check {
            Check::Now => true,
            Check::With(task) => task.starts_with("rust-"),
        }));
    }
}
