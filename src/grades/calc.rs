//! A pure reproduction of Canvas's course-grade arithmetic (no I/O), following Canvas's own
//! calculator (`AssignmentGroupGradeCalculator` / `CourseGradeCalculator` in canvas-lms):
//!
//! - An assignment counts only when published, graded and not omitted from the final grade;
//!   excused work is removed before anything else. An unposted submission counts as ungraded,
//!   as in the student's own view.
//! - **Current** grade: graded work only (no score, or pending review, is left out).
//!   **Final** grade: every remaining assignment counts, ungraded ones as 0.
//! - Drop rules apply per group to what survived. Canvas doesn't drop the lowest percentages:
//!   it keeps the subset with the best overall score/possible ratio (worst, for drop-highest),
//!   a fractional optimization solved here with Dinkelbach's iteration.
//! - Weighted groups: groups with nothing possible are skipped, the rest contribute
//!   `score / possible × weight`, rescaled to 100 only when the counted weights total less
//!   than 100. Unweighted: total points earned over total possible.
//!
//! Not reproduced: grading-period weighting, final grade overrides, and anything hidden from
//! the student. Callers say so.
use std::collections::HashMap;

use super::{assignment_counts, is_unposted};
use crate::canvas::models::AssignmentGroup;

/// One assignment as the grade sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub assignment_id: u64,
    pub name: String,
    pub possible: f64,
    /// A visible score; `None` while ungraded, unposted or pending review.
    pub score: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub id: u64,
    pub name: String,
    pub weight: f64,
    pub drop_lowest: usize,
    pub drop_highest: usize,
    pub never_drop: Vec<u64>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Graded work only.
    Current,
    /// Ungraded work counts as zero.
    Final,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupResult {
    pub id: u64,
    pub score: f64,
    pub possible: f64,
    /// Assignment IDs removed by drop rules.
    pub dropped: Vec<u64>,
    /// Whether this group took part in the course grade.
    pub counted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CourseResult {
    /// Unrounded percent; `None` when nothing counts yet.
    pub percent: Option<f64>,
    pub groups: Vec<GroupResult>,
}

/// Builds the grade model from Canvas's assignment groups (with assignments and submissions).
pub fn build_groups(groups: &[AssignmentGroup]) -> Vec<Group> {
    groups
        .iter()
        .map(|g| {
            let rules = g.rules.clone().unwrap_or_default();
            let items = g
                .assignments
                .iter()
                .filter(|a| assignment_counts(a))
                .filter_map(|a| {
                    let sub = a.submission.as_ref();
                    let unposted = sub.is_some_and(is_unposted);
                    // An unposted excusal doesn't apply yet; a posted one removes the assignment.
                    if !unposted && sub.is_some_and(|s| s.excused == Some(true)) {
                        return None;
                    }
                    let pending =
                        sub.is_some_and(|s| s.workflow_state.as_deref() == Some("pending_review"));
                    let score = if unposted || pending {
                        None
                    } else {
                        sub.and_then(|s| s.score)
                    };
                    Some(Item {
                        assignment_id: a.id,
                        name: a.title().to_owned(),
                        possible: a.points_possible.unwrap_or(0.0),
                        score,
                    })
                })
                .collect();
            Group {
                id: g.id,
                name: g.name.clone().unwrap_or_else(|| "Unnamed group".into()),
                weight: g.group_weight.unwrap_or(0.0),
                drop_lowest: rules.drop_lowest.unwrap_or(0) as usize,
                drop_highest: rules.drop_highest.unwrap_or(0) as usize,
                never_drop: rules.never_drop,
                items,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: u64,
    score: f64,
    possible: f64,
}

fn totals(entries: &[Entry]) -> (f64, f64) {
    entries
        .iter()
        .fold((0.0, 0.0), |(s, p), e| (s + e.score, p + e.possible))
}

/// Keeps `keep` of `candidates` so that, together with `fixed`, the score/possible ratio is as
/// high (`maximize`) or as low as possible. Dinkelbach's iteration: with ratio guess q, the best
/// subset of a fixed size is the top `keep` by `score - q·possible`; repeat until q settles.
fn choose(candidates: &[Entry], fixed: &[Entry], keep: usize, maximize: bool) -> Vec<Entry> {
    if keep >= candidates.len() {
        return candidates.to_vec();
    }
    let ratio = |kept: &[Entry]| {
        let (s, p) = totals(kept);
        let (fs, fp) = totals(fixed);
        let possible = p + fp;
        (possible > 0.0).then(|| (s + fs) / possible)
    };
    let pick = |q: f64| {
        let mut sorted = candidates.to_vec();
        sorted.sort_by(|a, b| {
            let (ka, kb) = (a.score - q * a.possible, b.score - q * b.possible);
            let ord = if maximize {
                kb.total_cmp(&ka)
            } else {
                ka.total_cmp(&kb)
            };
            ord.then(a.id.cmp(&b.id))
        });
        sorted.truncate(keep);
        sorted
    };
    let Some(mut q) = ratio(candidates) else {
        // Nothing possible anywhere: rank by raw score.
        return pick(0.0);
    };
    let mut kept = pick(q);
    for _ in 0..100 {
        let Some(next) = ratio(&kept) else { break };
        if (next - q).abs() < 1e-12 {
            break;
        }
        q = next;
        kept = pick(q);
    }
    kept
}

fn group_result(group: &Group, mode: Mode) -> GroupResult {
    let entries: Vec<Entry> = group
        .items
        .iter()
        .filter_map(|i| match (mode, i.score) {
            (Mode::Current, None) => None,
            (_, score) => Some(Entry {
                id: i.assignment_id,
                score: score.unwrap_or(0.0),
                possible: i.possible,
            }),
        })
        .collect();
    let (fixed, candidates): (Vec<Entry>, Vec<Entry>) = entries
        .iter()
        .partition(|e| group.never_drop.contains(&e.id));

    let mut kept = candidates.clone();
    if (group.drop_lowest > 0 || group.drop_highest > 0) && !candidates.is_empty() {
        // Canvas always keeps at least one droppable assignment.
        let n = candidates.len();
        let lowest = group.drop_lowest.min(n - 1);
        kept = choose(&candidates, &fixed, n - lowest, true);
        let highest = group.drop_highest.min(kept.len().saturating_sub(1));
        kept = choose(&kept, &fixed, kept.len() - highest, false);
    }
    let dropped = candidates
        .iter()
        .filter(|c| !kept.iter().any(|k| k.id == c.id))
        .map(|c| c.id)
        .collect();
    let (s, p) = totals(&kept);
    let (fs, fp) = totals(&fixed);
    GroupResult {
        id: group.id,
        score: s + fs,
        possible: p + fp,
        dropped,
        counted: false,
    }
}

/// The course grade under Canvas's rules.
pub fn calculate(groups: &[Group], weighted: bool, mode: Mode) -> CourseResult {
    let mut results: Vec<GroupResult> = groups.iter().map(|g| group_result(g, mode)).collect();
    let percent = if weighted {
        let mut total = 0.0;
        let mut weights = 0.0;
        for (g, r) in groups.iter().zip(results.iter_mut()) {
            if r.possible > 0.0 {
                r.counted = true;
                total += r.score / r.possible * g.weight;
                weights += g.weight;
            }
        }
        match weights {
            w if w <= 0.0 => None,
            // Weights over 100 act as extra credit, so only shortfalls are rescaled.
            w if w < 100.0 => Some(total * 100.0 / w),
            _ => Some(total),
        }
    } else {
        let (mut score, mut possible) = (0.0, 0.0);
        for r in &mut results {
            r.counted = r.possible > 0.0 || r.score > 0.0;
            score += r.score;
            possible += r.possible;
        }
        (possible > 0.0).then(|| score / possible * 100.0)
    };
    CourseResult {
        percent,
        groups: results,
    }
}

/// Applies what-if scores (assignment ID → points) as if graded.
pub fn with_scores(groups: &[Group], scores: &HashMap<u64, f64>) -> Vec<Group> {
    let mut out = groups.to_vec();
    for item in out.iter_mut().flat_map(|g| g.items.iter_mut()) {
        if let Some(score) = scores.get(&item.assignment_id) {
            item.score = Some(*score);
        }
    }
    out
}

/// Assignments that still count, are worth points, and have no visible score.
pub fn remaining(groups: &[Group]) -> Vec<&Item> {
    // Zero-point items can't move the grade, so they aren't "remaining work".
    groups
        .iter()
        .flat_map(|g| &g.items)
        .filter(|i| i.score.is_none() && i.possible > 0.0)
        .collect()
}

/// Every remaining assignment scored at the same `percent`.
pub fn project_uniform(groups: &[Group], percent: f64) -> Vec<Group> {
    let mut out = groups.to_vec();
    for item in out.iter_mut().flat_map(|g| g.items.iter_mut()) {
        if item.score.is_none() {
            item.score = Some(item.possible * percent / 100.0);
        }
    }
    out
}

/// Rounds the way Ruby does (half away from zero), as Canvas does before assigning letters.
/// A tiny relative nudge undoes binary noise, so 89.995 (stored as 89.99499…) rounds to 90.00.
pub fn canvas_round(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    let scaled = value * factor;
    let nudged = scaled + scaled.signum() * scaled.abs().max(1.0) * 1e-9;
    nudged.round() / factor
}

/// A letter-grade scheme: (letter, minimum fraction), highest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub entries: Vec<(String, f64)>,
    /// Points-based schemes (e.g. a 4.0 scale) round in points before picking a letter.
    pub points_based: bool,
    pub scaling_factor: f64,
}

impl Scheme {
    pub fn new(mut entries: Vec<(String, f64)>, points_based: bool, scaling_factor: f64) -> Self {
        entries.sort_by(|a, b| b.1.total_cmp(&a.1));
        Self {
            entries,
            points_based,
            scaling_factor: if scaling_factor > 0.0 {
                scaling_factor
            } else {
                1.0
            },
        }
    }

    /// Canvas's built-in scheme, used when a course enables letter grades without its own.
    pub fn canvas_default() -> Self {
        let entries = [
            ("A", 0.94),
            ("A-", 0.90),
            ("B+", 0.87),
            ("B", 0.84),
            ("B-", 0.80),
            ("C+", 0.77),
            ("C", 0.74),
            ("C-", 0.70),
            ("D+", 0.67),
            ("D", 0.64),
            ("D-", 0.61),
            ("F", 0.0),
        ];
        Self::new(
            entries.iter().map(|(l, v)| (l.to_string(), *v)).collect(),
            false,
            1.0,
        )
    }

    /// The letter Canvas shows for an (unrounded) course percent.
    pub fn letter(&self, percent: f64) -> &str {
        let found = if self.points_based {
            let points = canvas_round(percent * self.scaling_factor / 100.0, 2);
            self.entries
                .iter()
                .find(|(_, min)| points >= canvas_round(min * self.scaling_factor, 2) - 1e-9)
        } else {
            let rounded = canvas_round(percent, 2);
            self.entries
                .iter()
                .find(|(_, min)| rounded >= canvas_round(min * 100.0, 2) - 1e-9)
        };
        found
            .or(self.entries.last())
            .map_or("", |(letter, _)| letter.as_str())
    }

    /// The lowest percent that earns `letter` (case-insensitive), if the scheme has it.
    pub fn min_percent(&self, letter: &str) -> Option<f64> {
        self.entries
            .iter()
            .find(|(l, _)| l.eq_ignore_ascii_case(letter.trim()))
            .map(|(_, min)| min * 100.0)
    }
}

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

    fn item(id: u64, score: Option<f64>, possible: f64) -> Item {
        Item {
            assignment_id: id,
            name: format!("A{id}"),
            possible,
            score,
        }
    }

    fn group(id: u64, weight: f64, items: Vec<Item>) -> Group {
        Group {
            id,
            name: format!("G{id}"),
            weight,
            drop_lowest: 0,
            drop_highest: 0,
            never_drop: vec![],
            items,
        }
    }

    fn pct(groups: &[Group], weighted: bool, mode: Mode) -> f64 {
        calculate(groups, weighted, mode).percent.unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn unweighted_is_total_points() {
        let g = [group(
            1,
            0.0,
            vec![item(1, Some(8.0), 10.0), item(2, Some(15.0), 20.0)],
        )];
        assert!(close(pct(&g, false, Mode::Current), 23.0 / 30.0 * 100.0));
    }

    #[test]
    fn weighted_groups_combine_by_weight() {
        let g = [
            group(1, 40.0, vec![item(1, Some(9.0), 10.0)]),
            group(2, 60.0, vec![item(2, Some(70.0), 100.0)]),
        ];
        assert!(close(pct(&g, true, Mode::Current), 0.4 * 90.0 + 0.6 * 70.0));
    }

    #[test]
    fn current_rescales_missing_weight_and_final_counts_zeros() {
        let g = [
            group(1, 40.0, vec![item(1, Some(9.0), 10.0)]),
            group(2, 60.0, vec![item(2, None, 100.0)]),
        ];
        assert!(close(pct(&g, true, Mode::Current), 90.0));
        assert!(close(pct(&g, true, Mode::Final), 36.0));
    }

    #[test]
    fn weights_over_100_are_extra_credit() {
        let g = [
            group(1, 100.0, vec![item(1, Some(8.0), 10.0)]),
            group(2, 10.0, vec![item(2, Some(10.0), 10.0)]),
        ];
        assert!(close(pct(&g, true, Mode::Current), 80.0 + 10.0));
    }

    #[test]
    fn drop_lowest_keeps_best_ratio_not_best_percentages() {
        // Dropping the lowest percentage (9/20 = 45%) leaves 180/200 = 90%.
        // Canvas instead drops 80/100, leaving 109/120 = 90.83%.
        let mut g = group(
            1,
            0.0,
            vec![
                item(1, Some(100.0), 100.0),
                item(2, Some(80.0), 100.0),
                item(3, Some(9.0), 20.0),
            ],
        );
        g.drop_lowest = 1;
        let r = calculate(&[g], false, Mode::Current);
        assert_eq!(r.groups[0].dropped, vec![2]);
        assert!(close(r.percent.unwrap(), 109.0 / 120.0 * 100.0));
    }

    #[test]
    fn never_drop_is_protected_and_one_assignment_always_stays() {
        let mut g = group(
            1,
            0.0,
            vec![item(1, Some(1.0), 10.0), item(2, Some(9.0), 10.0)],
        );
        g.drop_lowest = 5;
        g.never_drop = vec![1];
        let r = calculate(&[g.clone()], false, Mode::Current);
        assert!(
            r.groups[0].dropped.is_empty(),
            "the only droppable assignment stays"
        );
        assert!(close(r.percent.unwrap(), 50.0));

        g.never_drop = vec![];
        let r = calculate(&[g], false, Mode::Current);
        assert_eq!(r.groups[0].dropped, vec![1]);
    }

    #[test]
    fn drop_highest_removes_the_best() {
        let mut g = group(
            1,
            0.0,
            vec![
                item(1, Some(10.0), 10.0),
                item(2, Some(5.0), 10.0),
                item(3, Some(6.0), 10.0),
            ],
        );
        g.drop_highest = 1;
        let r = calculate(&[g], false, Mode::Current);
        assert_eq!(r.groups[0].dropped, vec![1]);
        assert!(close(r.percent.unwrap(), 55.0));
    }

    #[test]
    fn drops_only_see_graded_work_in_current_mode() {
        let mut g = group(
            1,
            0.0,
            vec![
                item(1, Some(6.0), 10.0),
                item(2, None, 10.0),
                item(3, Some(9.0), 10.0),
            ],
        );
        g.drop_lowest = 1;
        let current = calculate(&[g.clone()], false, Mode::Current);
        assert_eq!(current.groups[0].dropped, vec![1]);
        let fin = calculate(&[g], false, Mode::Final);
        assert_eq!(
            fin.groups[0].dropped,
            vec![2],
            "the ungraded zero is dropped in the final grade"
        );
    }

    #[test]
    fn zero_possible_groups_are_skipped_when_weighted() {
        let g = [
            group(1, 50.0, vec![item(1, Some(8.0), 10.0)]),
            group(2, 50.0, vec![]),
        ];
        let r = calculate(&g, true, Mode::Final);
        assert!(close(r.percent.unwrap(), 80.0));
        assert!(!r.groups[1].counted);
        assert_eq!(
            calculate(&[group(1, 50.0, vec![])], true, Mode::Current).percent,
            None
        );
    }

    #[test]
    fn builds_from_canvas_payloads() {
        let groups: Vec<AssignmentGroup> = serde_json::from_value(serde_json::json!([{
            "id": 1, "name": "Labs", "group_weight": 40.0, "rules": { "drop_lowest": 1, "never_drop": [12] },
            "assignments": [
                { "id": 11, "name": "Lab 1", "points_possible": 10.0,
                  "submission": { "score": 9.0, "posted_at": "2026-09-01T00:00:00Z" } },
                { "id": 12, "name": "Lab 2", "points_possible": 10.0,
                  "submission": { "score": 7.0, "posted_at": null, "workflow_state": "graded" } },
                { "id": 13, "name": "Lab 3", "points_possible": 10.0,
                  "submission": { "excused": true, "posted_at": "2026-09-01T00:00:00Z" } },
                { "id": 14, "name": "Survey", "points_possible": 1.0, "grading_type": "not_graded" },
                { "id": 15, "name": "Essay", "points_possible": 20.0,
                  "submission": { "workflow_state": "pending_review", "score": 18.0 } }
            ]
        }])).unwrap();
        let built = build_groups(&groups);
        let ids: Vec<(u64, Option<f64>)> = built[0]
            .items
            .iter()
            .map(|i| (i.assignment_id, i.score))
            .collect();
        assert_eq!(ids, vec![(11, Some(9.0)), (12, None), (15, None)]);
        assert_eq!(
            (built[0].drop_lowest, built[0].never_drop.clone()),
            (1, vec![12])
        );
    }

    #[test]
    fn rounding_is_half_away_from_zero_despite_binary_noise() {
        assert_eq!(canvas_round(89.995, 2), 90.0);
        assert_eq!(canvas_round(17999.0 / 20000.0 * 100.0, 2), 90.0);
        assert_eq!(canvas_round(89.994, 2), 89.99);
        assert_eq!(canvas_round(-1.005, 2), -1.01);
    }

    #[test]
    fn letters_use_the_rounded_score() {
        let s = Scheme::canvas_default();
        assert_eq!(s.letter(93.995), "A");
        assert_eq!(s.letter(93.994), "A-");
        assert_eq!(s.letter(86.999), "B+");
        assert_eq!(s.letter(12.0), "F");
        assert_eq!(s.min_percent(" b+ "), Some(87.0));
        assert_eq!(s.min_percent("A+"), None);
    }

    #[test]
    fn points_based_schemes_round_in_points() {
        let entries = vec![
            ("A".to_string(), 0.925),
            ("A-".to_string(), 0.9),
            ("F".to_string(), 0.0),
        ];
        // 92.4% is 3.696 points, which rounds to 3.70 = the A cutoff on a 4-point scale.
        assert_eq!(Scheme::new(entries.clone(), true, 4.0).letter(92.4), "A");
        assert_eq!(Scheme::new(entries, false, 1.0).letter(92.4), "A-");
    }

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

    #[test]
    fn what_ifs_and_projection_fill_ungraded_work() {
        let g = [group(
            1,
            0.0,
            vec![
                item(1, Some(70.0), 100.0),
                item(2, None, 100.0),
                item(3, None, 50.0),
            ],
        )];
        let what_if = with_scores(&g, &HashMap::from([(2, 90.0)]));
        assert!(close(pct(&what_if, false, Mode::Current), 80.0));
        assert_eq!(remaining(&what_if).len(), 1);
        let with_zero_point = [group(
            1,
            0.0,
            vec![item(1, Some(5.0), 10.0), item(2, None, 0.0)],
        )];
        assert!(remaining(&with_zero_point).is_empty());
        let projected = project_uniform(&g, 80.0);
        assert!(close(
            pct(&projected, false, Mode::Current),
            (70.0 + 80.0 + 40.0) / 250.0 * 100.0
        ));
    }
}
