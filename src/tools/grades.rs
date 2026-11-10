use rmcp::{handler::server::wrapper::Parameters, tool, tool_router};

use super::{CanvasServer, CourseParams, ToolResult, err, more_note};
use crate::{
    canvas::models::{Assignment, AssignmentGroup, Course, Enrollment},
    format::points,
    grades::{self, grade_not_posted},
    untrusted::inline,
};

const MAX_COURSES: usize = 500;
const MAX_GROUPS: usize = 200;

const RESTRICTED: &str = "This course hides points and scores from students (restricted quantitative \
data), so Canvas withholds the numbers this tool needs. Check Canvas for your grade.";
const RESTRICTION_UNKNOWN: &str = "Canvas didn't say whether this course hides points and scores from \
students, so this tool won't guess. Check Canvas for your grade.";

fn student_enrollment(c: &Course) -> Option<&Enrollment> {
    let all = c.enrollments.as_deref()?;
    all.iter()
        .find(|e| e.kind.as_deref() == Some("student"))
        .or_else(|| all.first())
}

/// `B+ (88.4%)`, `88.4%`, `B+`, or `no grade yet`.
fn grade_text(score: Option<f64>, grade: Option<&str>) -> String {
    match (score, grade) {
        (Some(s), Some(g)) => format!("{g} ({s:.1}%)"),
        (Some(s), None) => format!("{s:.1}%"),
        (None, Some(g)) => g.to_owned(),
        (None, None) => "no grade yet".to_owned(),
    }
}

fn score_cell(a: &Assignment) -> String {
    let possible = a.points_possible.unwrap_or(0.0);
    let Some(sub) = &a.submission else {
        return format!("-/{}", points(possible));
    };
    if grade_not_posted(sub) {
        return format!("-/{}", points(possible));
    }
    if sub.excused == Some(true) {
        return "excused".into();
    }
    let Some(score) = sub.score else {
        return format!("-/{}", points(possible));
    };
    if possible > 0.0 {
        format!(
            "{}/{} ({:.1}%)",
            points(score),
            points(possible),
            score / possible * 100.0
        )
    } else {
        format!("{} pts (0 possible: extra credit)", points(score))
    }
}

fn rules_text(group: &AssignmentGroup) -> String {
    let Some(rules) = &group.rules else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(n) = rules.drop_lowest.filter(|n| *n > 0) {
        parts.push(format!("drop lowest {n}"));
    }
    if let Some(n) = rules.drop_highest.filter(|n| *n > 0) {
        parts.push(format!("drop highest {n}"));
    }
    if !rules.never_drop.is_empty() && !parts.is_empty() {
        parts.push(format!(
            "never drop {} assignment(s)",
            rules.never_drop.len()
        ));
    }
    parts.join("; ")
}

#[tool_router(router = grades_router, vis = "pub(super)")]
impl CanvasServer {
    /// Your current grade in each active course, as Canvas reports it (current = graded work
    /// only; final = ungraded work counted as zero), plus the current grading period if any.
    #[tool(annotations(
        title = "My course grades",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_course_grades(&self) -> ToolResult {
        let courses = self
            .client()
            .get_all::<Course>(
                "courses",
                &[
                    ("enrollment_state", "active"),
                    ("include[]", "total_scores"),
                    ("include[]", "current_grading_period_scores"),
                ],
                MAX_COURSES,
            )
            .await
            .map_err(err)?;
        if courses.items.is_empty() {
            return Ok("No active course enrollments found.".into());
        }
        let mut out = String::from("Your grades (as Canvas computes them):\n");
        for c in &courses.items {
            let mut line = format!("- {} (id {}): ", c.label(), c.id);
            match student_enrollment(c) {
                Some(e) => {
                    line.push_str(&format!(
                        "current {}",
                        grade_text(
                            e.computed_current_score,
                            e.computed_current_grade.as_deref()
                        )
                    ));
                    if let Some(final_score) = e.computed_final_score {
                        line.push_str(&format!(" · final {final_score:.1}%"));
                    }
                    if e.has_grading_periods == Some(true)
                        && let Some(title) = &e.current_grading_period_title
                    {
                        line.push_str(&format!(
                            " · {}: {}",
                            inline(title),
                            grade_text(
                                e.current_period_computed_current_score,
                                e.current_period_computed_current_grade.as_deref()
                            )
                        ));
                    }
                }
                None => line.push_str("no enrollment data"),
            }
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str("\nCurrent counts graded work only; final counts ungraded work as 0.");
        out.push_str(&more_note(courses.truncated, courses.items.len()));
        Ok(out)
    }

    /// Your score on every assignment in a course, grouped by assignment group with each
    /// group's weight and drop rules, plus status (graded, missing, late, not posted yet…).
    #[tool(annotations(
        title = "My assignment scores",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_assignment_scores(
        &self,
        Parameters(p): Parameters<CourseParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let c: Course = self
            .client()
            .get(
                &format!("courses/{}", course.path_id),
                &[("include[]", "restrict_quantitative_data")],
            )
            .await
            .map_err(err)?;
        // Fail closed: only an explicit false proves scores aren't withheld.
        match c.restrict_quantitative_data {
            Some(false) => {}
            Some(true) => return Err(RESTRICTED.into()),
            None => return Err(RESTRICTION_UNKNOWN.into()),
        }
        let groups = self
            .client()
            .get_all::<AssignmentGroup>(
                &format!("courses/{}/assignment_groups", course.path_id),
                &[("include[]", "assignments"), ("include[]", "submission")],
                MAX_GROUPS,
            )
            .await
            .map_err(err)?;

        let weighted = c.apply_assignment_group_weights == Some(true);
        let mut out = format!("Assignment scores for {}\n", course.label);
        out.push_str(match c.apply_assignment_group_weights {
            Some(true) => "Weighting: by assignment group (weights below)\n",
            Some(false) => "Weighting: none; course grade = total points earned / total possible\n",
            None => "Weighting: not reported by Canvas\n",
        });
        out.push_str("Only assignments Canvas shows you are listed.\n");
        if groups.items.is_empty() {
            out.push_str("\nNo assignment groups are visible in this course.");
            return Ok(out);
        }

        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for g in &groups.items {
            let mut header = format!(
                "\n== {}",
                inline(g.name.as_deref().unwrap_or("Unnamed group"))
            );
            if weighted {
                header.push_str(&format!(
                    " | weight {}%",
                    points(g.group_weight.unwrap_or(0.0))
                ));
            }
            let rules = rules_text(g);
            if !rules.is_empty() {
                header.push_str(&format!(" | {rules}"));
            }
            out.push_str(&header);
            out.push('\n');
            if g.assignments.is_empty() {
                out.push_str("  (no assignments visible)\n");
            }
            for a in &g.assignments {
                let labels = grades::status_labels(a);
                for l in &labels {
                    if !l.starts_with("late penalty") {
                        *counts.entry(l.clone()).or_default() += 1;
                    }
                }
                out.push_str(&format!(
                    "  - {} (id {}): {} | {} | due {}\n",
                    inline(a.title()),
                    a.id,
                    score_cell(a),
                    labels.join(", "),
                    self.date(a.due_at, "no due date")
                ));
            }
        }
        let summary: Vec<String> = counts.iter().map(|(l, n)| format!("{n} {l}")).collect();
        out.push_str(&format!("\nSummary: {}", summary.join(", ")));
        out.push_str(&more_note(groups.truncated, groups.items.len()));
        Ok(out)
    }
}
