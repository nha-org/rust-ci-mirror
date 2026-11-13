//! When a login runs out (task rust-account-handover-port, rfc-rust-unified#scheduling): each window's burn, the faster of its
//! live burn (the slope of its last readings, as `Dashboard.Orch.AccountEnds.forecast/2`) and the pace it ran since it opened
//! (`Dashboard.ClaudeAccounts.runs_out_in/2`), carried to 100%. The sooner of the two rules, as the older app's handover
//! read it: a burst shows at once, a steady day never reads as idle.

/// The readings a live burn looks back over, and the span it needs: one reading says nothing of a pace.
pub const LOOK_BACK_S: u64 = 90 * 60;
pub const SPAN_S: u64 = 5 * 60;

/// A window's figure at a time: `at` and `resets` Unix seconds, `used` percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub at: u64,
    pub used: f64,
    pub resets: u64,
}

/// A window forecast: its burn in percent an hour, and when it reaches 100% (Unix seconds), `None` when it resets first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Out {
    pub burn: f64,
    pub at: Option<u64>,
}

/// The live burn of `newest_first` readings of one window, percent an hour: from the oldest reading of the newest one's
/// window within `LOOK_BACK_S`, at least `SPAN_S` before it. `None` when none climbs, or the newest is older than the look-back.
pub fn live_burn(newest_first: &[Reading], now: u64) -> Option<f64> {
    let newest = newest_first.first()?;
    if now.saturating_sub(newest.at) > LOOK_BACK_S {
        return None;
    }
    let same = |r: &&Reading| {
        r.resets == newest.resets
            && newest.at.saturating_sub(r.at) >= SPAN_S
            && now.saturating_sub(r.at) <= LOOK_BACK_S
    };
    let oldest = newest_first.iter().rev().find(same)?;
    let rate = (newest.used - oldest.used) / ((newest.at - oldest.at) as f64 / 3600.0);
    (rate > 0.0).then_some(rate)
}

/// When `newest` (a window of `span_h` hours) reaches 100% at `now`: at the faster of `live` and the pace it ran since it
/// opened, that pace over `floor_h` hours at least so a window just opened never reads as a flood. A window at 100% is out
/// now; one with no reading or no reset is no forecast.
pub fn out(newest: Reading, span_h: f64, floor_h: f64, live: Option<f64>, now: u64) -> Option<Out> {
    if newest.resets <= now {
        return None;
    }
    // the hours it had run when it was read: a stale reading over the hours to now would understate it (/code-review)
    let left_h = newest.resets.saturating_sub(newest.at) as f64 / 3600.0;
    let pace = newest.used / (span_h - left_h).max(floor_h);
    let burn = live.unwrap_or(0.0).max(pace);
    if newest.used >= 100.0 {
        return Some(Out {
            burn,
            at: Some(now),
        });
    }
    if burn <= 0.0 {
        return Some(Out { burn, at: None });
    }
    // from the newest reading's time: the minutes since it already count
    let to_full_h = (100.0 - newest.used) / burn;
    let at = newest.at + (to_full_h * 3600.0) as u64;
    Some(Out {
        burn,
        at: (at < newest.resets).then_some(at.max(now)),
    })
}

/// The week's forecast, as `out` reads a 7-day window: its pace over a day at least.
pub fn week(newest_first: &[Reading], now: u64) -> Option<Out> {
    out(
        *newest_first.first()?,
        168.0,
        24.0,
        live_burn(newest_first, now),
        now,
    )
}

/// The 5 hours', its pace over half an hour at least.
pub fn five_hours(newest_first: &[Reading], now: u64) -> Option<Out> {
    out(
        *newest_first.first()?,
        5.0,
        0.5,
        live_burn(newest_first, now),
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    const NOW: u64 = 1_794_500_000;
    const RESETS: u64 = NOW + 2 * 86_400;

    fn at(minutes_ago: u64, used: f64) -> Reading {
        Reading {
            at: NOW - minutes_ago * 60,
            used,
            resets: RESETS,
        }
    }

    /// A recorded burn: default's week climbing 90% to 95% over the last hour, two days before its reset. At 5% an hour it
    /// reaches 100% an hour from now; at its pace since the week opened alone (95% over 5 days) it would last 6 hours more.
    #[dash_test]
    fn a_recorded_burn_names_the_hour_the_week_runs_out() {
        let burn = [at(0, 95.0), at(20, 93.5), at(40, 91.5), at(60, 90.0)];
        let out = week(&burn, NOW).expect("a forecast");
        assert_eq!(
            out.burn, 5.0,
            "5% an hour, newest against the oldest in the look-back"
        );
        assert_eq!(out.at, Some(NOW + 3600), "100% an hour from now");
        // the same week read once has no live burn: its pace, 95% over 120 hours, takes 6.3 hours to the last 5%
        let paced = week(&burn[..1], NOW).expect("a forecast");
        assert!((paced.burn - 95.0 / 120.0).abs() < 1e-9, "{paced:?}");
        assert_eq!(paced.at, Some(NOW + 22_736), "{paced:?}");
        // a slow week resets first
        let slow = week(&[at(0, 50.0)], NOW).expect("a forecast");
        assert_eq!(
            slow.at, None,
            "50% over 120 hours lasts past its reset in 48"
        );
    }

    #[dash_test]
    fn a_reading_too_close_too_old_or_of_another_window_gives_no_live_burn() {
        assert_eq!(
            live_burn(&[at(0, 95.0), at(3, 90.0)], NOW),
            None,
            "under 5 minutes apart"
        );
        assert_eq!(
            live_burn(&[at(0, 95.0), at(120, 50.0)], NOW),
            None,
            "past the look-back"
        );
        let other = Reading {
            resets: RESETS - 7 * 86_400,
            ..at(30, 10.0)
        };
        assert_eq!(
            live_burn(&[at(0, 5.0), other], NOW),
            None,
            "the week before"
        );
        assert_eq!(
            live_burn(&[at(100, 95.0), at(110, 90.0)], NOW),
            None,
            "a stale newest reading"
        );
        assert_eq!(live_burn(&[at(0, 50.0), at(30, 50.0)], NOW), None, "flat");
    }

    #[dash_test]
    fn a_window_at_100_is_out_now_and_one_past_its_reset_has_no_forecast() {
        assert_eq!(week(&[at(0, 100.0)], NOW).and_then(|o| o.at), Some(NOW));
        let past = Reading {
            resets: NOW - 1,
            ..at(0, 99.0)
        };
        assert_eq!(week(&[past], NOW), None);
        assert_eq!(week(&[], NOW), None);
    }
}
