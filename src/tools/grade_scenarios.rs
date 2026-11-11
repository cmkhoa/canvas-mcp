//! The grade what-if calculator tool, built on `grades::calc`.
use std::collections::HashMap;

use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{
    CanvasServer, ToolResult, err,
    grades::{MAX_GROUPS, RESTRICTED, RESTRICTION_UNKNOWN, student_enrollment},
};
use crate::{
    canvas::models::{AssignmentGroup, Course, GradingStandard},
    format::points,
    grades::{
        calc::{self, Mode},
        scheme::{Scheme, canvas_round},
        target::{self, Target},
    },
    resolve::ResolvedCourse,
    untrusted::inline,
};

const MAX_REMAINING_LISTED: usize = 15;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScenarioParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// What-if scores in points, keyed by assignment ID (from get_my_assignment_scores),
    /// e.g. {"453891": 9.5}. They're treated as graded.
    #[serde(default)]
    pub hypothetical_scores: HashMap<String, f64>,
    /// Target course percent, e.g. 90. Finds the uniform score needed on remaining work.
    #[serde(default)]
    pub target_percent: Option<f64>,
    /// Target letter grade, e.g. "B+", using the course's grading scheme.
    #[serde(default)]
    pub target_letter: Option<String>,
}

/// Reads the course's letter scheme: its own grading standard, else the scheme Canvas embeds
/// in the course, else Canvas's default when the course shows letters. `None` = no letters.
fn scheme_from_course(c: &Course) -> Option<Scheme> {
    let entries: Vec<(String, f64)> = c
        .grading_scheme
        .as_ref()?
        .as_array()?
        .iter()
        .filter_map(|e| match e {
            serde_json::Value::Array(pair) => {
                Some((pair.first()?.as_str()?.to_owned(), pair.get(1)?.as_f64()?))
            }
            serde_json::Value::Object(o) => Some((
                o.get("name")?.as_str()?.to_owned(),
                o.get("value")?.as_f64()?,
            )),
            _ => None,
        })
        .collect();
    (!entries.is_empty()).then(|| Scheme::new(entries, false, 1.0))
}

/// `88.43% (B+)`, or `no grade yet`.
fn percent_text(percent: Option<f64>, scheme: Option<&Scheme>) -> String {
    match (percent, scheme) {
        (Some(p), Some(s)) => format!("{:.2}% ({})", canvas_round(p, 2), s.letter(p)),
        (Some(p), None) => format!("{:.2}%", canvas_round(p, 2)),
        (None, _) => "no grade yet".into(),
    }
}

impl CanvasServer {
    /// The course's letter scheme and where it came from.
    async fn letter_scheme(&self, course: &ResolvedCourse, c: &Course) -> (Option<Scheme>, String) {
        if let Some(id) = c.grading_standard_id.filter(|id| *id > 0) {
            match self
                .client()
                .get::<GradingStandard>(
                    &format!("courses/{}/grading_standards/{id}", course.path_id),
                    &[],
                )
                .await
            {
                Ok(std) if !std.grading_scheme.is_empty() => {
                    let entries = std
                        .grading_scheme
                        .into_iter()
                        .map(|e| (e.name, e.value))
                        .collect();
                    let title = std
                        .title
                        .map(|t| inline(&t))
                        .unwrap_or_else(|| "the course's".into());
                    return (
                        Some(Scheme::new(
                            entries,
                            std.points_based == Some(true),
                            std.scaling_factor.unwrap_or(1.0),
                        )),
                        format!("{title} grading scheme"),
                    );
                }
                _ => {
                    if let Some(scheme) = scheme_from_course(c) {
                        return (Some(scheme), "the course's grading scheme".into());
                    }
                    return (
                        Some(Scheme::canvas_default()),
                        "Canvas's default scheme (the course's own scheme couldn't be read, so letters may differ)".into(),
                    );
                }
            }
        }
        // Canvas embeds a scheme in every course, even ones that don't show students letters,
        // so without a standard of its own the course's letters count only if Canvas shows one.
        if student_enrollment(c).is_some_and(|e| e.computed_current_grade.is_some()) {
            return match scheme_from_course(c) {
                Some(scheme) => (Some(scheme), "the course's grading scheme".into()),
                None => (
                    Some(Scheme::canvas_default()),
                    "Canvas's default scheme".into(),
                ),
            };
        }
        (None, "none (this course doesn't show letter grades)".into())
    }
}

#[tool_router(router = grade_scenarios_router, vis = "pub(super)")]
impl CanvasServer {
    /// Recompute your course grade the way Canvas does (weights, drop rules, unposted work),
    /// compare it with Canvas's own number, try what-if scores, and find the uniform score
    /// you'd need on all remaining work to reach a target percent or letter.
    #[tool(annotations(
        title = "Grade what-if calculator",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn calculate_grade_scenarios(
        &self,
        Parameters(p): Parameters<ScenarioParams>,
    ) -> ToolResult {
        if p.target_percent.is_some() && p.target_letter.is_some() {
            return Err("Pass target_percent or target_letter, not both.".into());
        }
        if let Some(t) = p.target_percent
            && !(t.is_finite() && (0.0..=200.0).contains(&t))
        {
            return Err("target_percent must be between 0 and 200.".into());
        }
        let course = self.resolve(&p.course_identifier).await?;
        let c: Course = self
            .client()
            .get(
                &format!("courses/{}", course.path_id),
                &[
                    ("include[]", "total_scores"),
                    ("include[]", "current_grading_period_scores"),
                    ("include[]", "grading_scheme"),
                    ("include[]", "restrict_quantitative_data"),
                ],
            )
            .await
            .map_err(err)?;
        match c.restrict_quantitative_data {
            Some(false) => {}
            Some(true) => return Err(RESTRICTED.into()),
            None => return Err(RESTRICTION_UNKNOWN.into()),
        }
        let raw_groups = self
            .client()
            .get_all::<AssignmentGroup>(
                &format!("courses/{}/assignment_groups", course.path_id),
                &[("include[]", "assignments"), ("include[]", "submission")],
                MAX_GROUPS,
            )
            .await
            .map_err(err)?;
        if raw_groups.truncated {
            return Err("This course has more assignment groups than this tool reads, so it won't compute a partial grade.".into());
        }
        let groups = calc::build_groups(&raw_groups.items);
        let weighted = c.apply_assignment_group_weights == Some(true);
        let (scheme, scheme_source) = self.letter_scheme(&course, &c).await;

        // What-if scores, checked against assignments that actually count.
        let names: HashMap<u64, (&str, f64)> = groups
            .iter()
            .flat_map(|g| &g.items)
            .map(|i| (i.assignment_id, (i.name.as_str(), i.possible)))
            .collect();
        let mut what_if = HashMap::new();
        for (key, score) in &p.hypothetical_scores {
            let id: u64 = key.trim().parse().map_err(|_| {
                format!("hypothetical_scores keys are assignment IDs; got {key:?}.")
            })?;
            if !names.contains_key(&id) {
                return Err(format!(
                    "Assignment {id} doesn't count toward this course's grade (or isn't visible to you). \
                     get_my_assignment_scores lists the IDs."
                ));
            }
            if !(score.is_finite() && *score >= 0.0) {
                return Err(format!(
                    "The what-if score for {id} must be a number of points, 0 or more."
                ));
            }
            what_if.insert(id, *score);
        }

        let target = match (&p.target_letter, p.target_percent) {
            (Some(letter), _) => {
                let scheme = scheme
                    .as_ref()
                    .ok_or("This course doesn't use letter grades; pass target_percent instead.")?;
                let min = scheme.min_percent(letter).ok_or_else(|| {
                    let letters: Vec<&str> =
                        scheme.entries.iter().map(|(l, _)| l.as_str()).collect();
                    format!(
                        "{letter:?} isn't in this course's scheme ({}).",
                        letters.join(", ")
                    )
                })?;
                Some((format!("{} ({min:.2}%)", letter.trim().to_uppercase()), min))
            }
            (None, Some(t)) => Some((format!("{t}%"), t)),
            (None, None) => None,
        };

        let current = calc::calculate(&groups, weighted, Mode::Current);
        let fin = calc::calculate(&groups, weighted, Mode::Final);
        let mut out = format!("Grade calculation for {}\n", course.label);
        out.push_str(&format!(
            "Weighting: {}\nLetters: {scheme_source}\n\n",
            if weighted {
                "by assignment group"
            } else {
                "total points"
            }
        ));

        let enrollment = student_enrollment(&c);
        let canvas_current = enrollment.and_then(|e| e.computed_current_score);
        out.push_str(&format!(
            "Canvas reports: current {}, final {}\n",
            enrollment.map_or("unknown".into(), |e| {
                match (e.computed_current_score, &e.computed_current_grade) {
                    (Some(s), Some(g)) => format!("{s:.2}% ({g})"),
                    (Some(s), None) => format!("{s:.2}%"),
                    (None, Some(g)) => g.clone(),
                    (None, None) => "no grade yet".into(),
                }
            }),
            enrollment
                .and_then(|e| e.computed_final_score)
                .map_or("unknown".into(), |f| format!("{f:.2}%"))
        ));
        out.push_str(&format!(
            "Recomputed: current {}, final {}\n",
            percent_text(current.percent, scheme.as_ref()),
            percent_text(fin.percent, scheme.as_ref())
        ));
        if let (Some(ours), Some(theirs)) = (current.percent, canvas_current)
            && (canvas_round(ours, 2) - theirs).abs() > 0.011
        {
            out.push_str(&format!(
                "Note: this differs from Canvas's {theirs:.2}%. Likely causes: grading periods, a final grade override, \
                 or assignments hidden from you. Trust Canvas's number.\n"
            ));
        }

        out.push_str("\nBy group (current):\n");
        for (g, r) in groups.iter().zip(&current.groups) {
            let mut line = format!("- {}", inline(&g.name));
            if weighted {
                line.push_str(&format!(" (weight {}%)", points(g.weight)));
            }
            if r.possible > 0.0 {
                line.push_str(&format!(
                    ": {}/{} = {:.2}%",
                    points(r.score),
                    points(r.possible),
                    r.score / r.possible * 100.0
                ));
            } else {
                line.push_str(": nothing graded yet");
                if weighted {
                    line.push_str(" (not counted)");
                }
            }
            if !r.dropped.is_empty() {
                let dropped: Vec<String> = r
                    .dropped
                    .iter()
                    .map(|id| inline(names.get(id).map_or("?", |n| n.0)))
                    .collect();
                line.push_str(&format!(" · dropped {}", dropped.join(", ")));
            }
            out.push_str(&line);
            out.push('\n');
        }

        let scenario = calc::with_scores(&groups, &what_if);
        if !what_if.is_empty() {
            let applied: Vec<String> = what_if
                .iter()
                .map(|(id, score)| {
                    let (name, possible) = names[id];
                    format!("{} = {}/{}", inline(name), points(*score), points(possible))
                })
                .collect();
            out.push_str(&format!(
                "\nWith what-ifs ({}): current {}, final {}\n",
                applied.join("; "),
                percent_text(
                    calc::calculate(&scenario, weighted, Mode::Current).percent,
                    scheme.as_ref()
                ),
                percent_text(
                    calc::calculate(&scenario, weighted, Mode::Final).percent,
                    scheme.as_ref()
                )
            ));
        }

        if let Some((label, target)) = target {
            let remaining = calc::remaining(&scenario);
            let remaining_points: f64 = remaining.iter().map(|i| i.possible).sum();
            out.push('\n');
            match target::required_uniform_percent(&scenario, weighted, target) {
                Target::Needs { percent } => out.push_str(&format!(
                    "To reach {label}: about {percent:.2}% on each of the {} remaining assignments ({} points in all).\n",
                    remaining.len(),
                    points(remaining_points)
                )),
                Target::AlreadyMet { at_zero } => out.push_str(&format!(
                    "{label} is already secured: even zeros on the {} remaining assignments leave you at {}.\n",
                    remaining.len(),
                    percent_text(Some(at_zero), scheme.as_ref())
                )),
                Target::Unreachable { at_full } => out.push_str(&format!(
                    "{label} is out of reach: 100% on all {} remaining assignments gives {}.\n",
                    remaining.len(),
                    percent_text(at_full, scheme.as_ref())
                )),
                Target::NothingRemaining { current } => out.push_str(&format!(
                    "No ungraded work is left to change the grade; it stands at {}.\n",
                    percent_text(current, scheme.as_ref())
                )),
            }
            if !remaining.is_empty() {
                let listed: Vec<String> = remaining
                    .iter()
                    .take(MAX_REMAINING_LISTED)
                    .map(|i| format!("{} ({} pts)", inline(&i.name), points(i.possible)))
                    .collect();
                let more = remaining.len().saturating_sub(MAX_REMAINING_LISTED);
                out.push_str(&format!(
                    "Remaining: {}{}\n",
                    listed.join(", "),
                    if more > 0 {
                        format!(" and {more} more")
                    } else {
                        String::new()
                    }
                ));
            }
        }

        out.push_str("\nCaveats: only assignments visible to you are included; final grade overrides aren't reflected");
        if enrollment.is_some_and(|e| e.has_grading_periods == Some(true)) {
            out.push_str("; this course uses grading periods, which Canvas may weight separately");
        }
        out.push('.');
        Ok(out)
    }
}
