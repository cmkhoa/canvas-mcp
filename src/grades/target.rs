//! "What do I need on the rest?": solving for a uniform score on remaining work.
use super::calc::{Group, Mode, calculate, project_uniform, remaining};

/// What uniform score on all remaining work reaches a target grade.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// Already reached even with zeros on everything left (`at_zero` is that grade).
    AlreadyMet { at_zero: f64 },
    /// The uniform percent needed, rounded up to 0.01 so it is never too low.
    Needs { percent: f64 },
    /// Out of reach even with 100% on everything left.
    Unreachable { at_full: Option<f64> },
    /// No ungraded work left to change the grade.
    NothingRemaining { current: Option<f64> },
}

const GRID_STEP: f64 = 0.5;

/// Solves for the uniform percent on remaining work that brings the unrounded grade to at
/// least `target`. Asking for the unrounded grade can cost 0.01 more than strictly needed,
/// but never reports a score as enough when it isn't.
pub fn required_uniform_percent(groups: &[Group], weighted: bool, target: f64) -> Target {
    let grade = |p: f64| calculate(&project_uniform(groups, p), weighted, Mode::Current).percent;
    let meets = |p: f64| grade(p).is_some_and(|g| g >= target - 1e-9);
    if remaining(groups).is_empty() {
        return Target::NothingRemaining {
            current: calculate(groups, weighted, Mode::Current).percent,
        };
    }
    if meets(0.0) {
        return Target::AlreadyMet {
            at_zero: grade(0.0).unwrap_or_default(),
        };
    }
    // Dropping both lowest and highest can make the grade non-monotone in the score, so
    // bisection could miss; scan a grid instead.
    if groups
        .iter()
        .any(|g| g.drop_lowest > 0 && g.drop_highest > 0)
    {
        let steps = (100.0 / GRID_STEP) as usize;
        return (0..=steps)
            .map(|i| i as f64 * GRID_STEP)
            .find(|&p| meets(p))
            .map_or(
                Target::Unreachable {
                    at_full: grade(100.0),
                },
                |p| Target::Needs { percent: p },
            );
    }
    if !meets(100.0) {
        return Target::Unreachable {
            at_full: grade(100.0),
        };
    }
    let (mut lo, mut hi) = (0.0, 100.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if meets(mid) { hi = mid } else { lo = mid }
    }
    let rounded_up = ((hi * 100.0) - 1e-9).ceil() / 100.0;
    Target::Needs {
        percent: if meets(rounded_up) { rounded_up } else { hi },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grades::test_support::{group, item};

    #[test]
    fn target_solver_finds_the_uniform_score_needed() {
        let g = [group(
            1,
            0.0,
            vec![item(1, Some(70.0), 100.0), item(2, None, 100.0)],
        )];
        assert_eq!(
            required_uniform_percent(&g, false, 80.0),
            Target::Needs { percent: 90.0 }
        );
        assert_eq!(
            required_uniform_percent(&g, false, 30.0),
            Target::AlreadyMet { at_zero: 35.0 }
        );
        assert_eq!(
            required_uniform_percent(&g, false, 90.0),
            Target::Unreachable {
                at_full: Some(85.0)
            }
        );

        let done = [group(1, 0.0, vec![item(1, Some(70.0), 100.0)])];
        assert_eq!(
            required_uniform_percent(&done, false, 80.0),
            Target::NothingRemaining {
                current: Some(70.0)
            }
        );
    }

    #[test]
    fn target_answers_never_round_down() {
        // 1/3 of the remaining points: exact need is 66.666…%, reported as 66.67%.
        let g = [group(
            1,
            0.0,
            vec![item(1, Some(0.0), 100.0), item(2, None, 300.0)],
        )];
        assert_eq!(
            required_uniform_percent(&g, false, 50.0),
            Target::Needs { percent: 66.67 }
        );
    }

    #[test]
    fn non_monotone_groups_use_the_grid() {
        let mut g = group(
            1,
            0.0,
            vec![
                item(1, Some(50.0), 100.0),
                item(2, Some(90.0), 100.0),
                item(3, None, 100.0),
            ],
        );
        g.drop_lowest = 1;
        g.drop_highest = 1;
        match required_uniform_percent(&[g], false, 60.0) {
            Target::Needs { percent } => assert!((percent - 60.0).abs() < 1e-9, "{percent}"),
            other => panic!("{other:?}"),
        }
    }
}
