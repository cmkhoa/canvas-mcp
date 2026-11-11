use chrono::Utc;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{
    CanvasServer, CourseAssignmentParams, OptionalCourseParams, ToolResult, err, more_note,
};
use crate::{
    canvas::models::{Assignment, Course, Submission},
    error::CanvasError,
    format::{html_to_text, points},
    grades,
    untrusted::{fence_untrusted, inline},
};

const MAX_ASSIGNMENTS_PER_COURSE: usize = 300;
const SUBMISSION_BODY_CHARS: usize = 4000;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AssignmentDetailsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Canvas assignment ID (from get_my_upcoming_assignments, get_my_submission_status, …).
    pub assignment_id: u64,
    /// Optional cap on description characters. Omit for the full description.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// Where the caller stands on an assignment, following Canvas's own flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Missing,
    Submitted,
    /// Submitted through an external tool such as Gradescope; Canvas may never see it.
    ExternalTool,
    Excused,
    GradedWithoutSubmission,
    NoOnlineSubmission,
    NotSubmitted,
}

fn status(a: &Assignment) -> Status {
    let sub = a.submission.as_ref();
    let external = a.is_external_tool();
    // Trust Canvas's explicit missing flag (including manual overrides), except for
    // external-tool work Canvas can't see.
    if sub.is_some_and(|s| s.missing == Some(true)) && !external {
        Status::Missing
    } else if sub.is_some_and(|s| s.submitted_at.is_some()) {
        Status::Submitted
    } else if external {
        Status::ExternalTool
    } else if sub.is_some_and(|s| s.excused == Some(true)) {
        Status::Excused
    } else if sub.is_some_and(|s| s.workflow_state.as_deref() == Some("graded")) {
        Status::GradedWithoutSubmission
    } else if !a.needs_online_submission() {
        Status::NoOnlineSubmission
    } else {
        Status::NotSubmitted
    }
}

fn attempts_text(allowed: Option<i64>, used: Option<u32>) -> String {
    let used = used.unwrap_or(0);
    match allowed {
        Some(n) if n > 0 => format!("{used} of {n} attempts used"),
        _ => format!("{used} attempts used (unlimited)"),
    }
}

/// `8/10 (B)`, `not graded yet`, or `grade not released yet`.
fn score_text(sub: &Submission, possible: Option<f64>) -> String {
    match (sub.score, &sub.grade) {
        (Some(score), grade) => {
            let mut s = match possible {
                Some(p) => format!("{}/{}", points(score), points(p)),
                None => points(score),
            };
            if let Some(g) = grade.as_deref().filter(|g| *g != points(score)) {
                s.push_str(&format!(" ({g})"));
            }
            if let Some(d) = sub.points_deducted.filter(|d| *d > 0.0) {
                s.push_str(&format!(", {} late penalty", points(d)));
            }
            s
        }
        _ if grades::grade_not_posted(sub) => "grade not released yet".into(),
        _ => "not graded yet".into(),
    }
}

impl CanvasServer {
    async fn assignments_with_submissions(
        &self,
        course_path_id: &str,
    ) -> Result<(Vec<Assignment>, bool), CanvasError> {
        let paged = self
            .client()
            .get_all::<Assignment>(
                &format!("courses/{course_path_id}/assignments"),
                &[("include[]", "submission"), ("order_by", "due_at")],
                MAX_ASSIGNMENTS_PER_COURSE,
            )
            .await?;
        Ok((paged.items, paged.truncated))
    }

    pub(super) fn assignment_line(&self, course: &str, a: &Assignment) -> String {
        let mut line = format!(
            "- {} · {} · due {}",
            course,
            inline(a.title()),
            self.date(a.due_at, "no due date")
        );
        line.push_str(&format!(" · id {}", a.id));
        if a.submission.as_ref().is_some_and(|s| s.late == Some(true)) {
            line.push_str(" · late");
        }
        line
    }
}

#[tool_router(router = assignments_router, vis = "pub(super)")]
impl CanvasServer {
    /// Which assignments you've submitted, which Canvas flags as missing, and which are still
    /// open, for one course or (without course_identifier) all active courses. Work done in
    /// external tools like Gradescope is listed separately, since Canvas can't see it.
    #[tool(annotations(
        title = "Submission status",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_submission_status(
        &self,
        Parameters(p): Parameters<OptionalCourseParams>,
    ) -> ToolResult {
        let mut rows: Vec<(String, Assignment)> = Vec::new();
        let mut notes = String::new();
        let heading = match p.course_identifier.as_deref() {
            Some(ident) => {
                let course = self.resolve(ident).await?;
                let (assignments, truncated) = self
                    .assignments_with_submissions(&course.path_id)
                    .await
                    .map_err(err)?;
                notes.push_str(&more_note(truncated, assignments.len()));
                rows.extend(assignments.into_iter().map(|a| (course.label.clone(), a)));
                format!("Submission status for {}:", course.label)
            }
            None => {
                let courses = self.courses().active_courses().await.map_err(err)?;
                let results = self
                    .fan_out(&courses, |c: &Course| {
                        let path_id = c.id.to_string();
                        async move { self.assignments_with_submissions(&path_id).await }
                    })
                    .await;
                notes.push_str(&results.failure_note());
                for (course, (assignments, truncated)) in results.ok {
                    if truncated {
                        notes.push_str(&format!(
                            "\n{}: only the first {} assignments were checked.",
                            course.label(),
                            assignments.len()
                        ));
                    }
                    rows.extend(assignments.into_iter().map(|a| (course.label(), a)));
                }
                "Submission status across your active courses:".into()
            }
        };
        if rows.is_empty() {
            return Ok(format!("{heading}\nNo assignments found.{notes}"));
        }

        let now = Utc::now();
        let sections: [(Status, &str); 7] = [
            (Status::Missing, "Missing (Canvas flags these)"),
            (Status::NotSubmitted, "Not submitted yet"),
            (
                Status::ExternalTool,
                "Done in an external tool like Gradescope or PrairieLearn (check there; Canvas can't tell)",
            ),
            (Status::Submitted, "Submitted"),
            (
                Status::GradedWithoutSubmission,
                "Graded without a submission",
            ),
            (Status::Excused, "Excused"),
            (Status::NoOnlineSubmission, "No online submission needed"),
        ];
        let mut out = format!("{heading}\n");
        for (wanted, title) in sections {
            let matching: Vec<_> = rows.iter().filter(|(_, a)| status(a) == wanted).collect();
            if matching.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{title} ({}):\n", matching.len()));
            for (course, a) in matching {
                let mut line = self.assignment_line(course, a);
                if wanted == Status::NotSubmitted && a.due_at.is_some_and(|d| d < now) {
                    line.push_str(" · past due");
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push_str(&notes);
        Ok(out)
    }

    /// Your own submission for one assignment: status, attempts used, score, files and
    /// comments from your instructor.
    #[tool(annotations(title = "My submission", read_only_hint = true, open_world_hint = true))]
    pub async fn get_my_submission(
        &self,
        Parameters(p): Parameters<CourseAssignmentParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let sub: Submission = self
            .client()
            .get(
                &format!(
                    "courses/{}/assignments/{}/submissions/self",
                    course.path_id, p.assignment_id
                ),
                &[
                    ("include[]", "submission_comments"),
                    ("include[]", "assignment"),
                ],
            )
            .await
            .map_err(err)?;
        let assignment = sub.assignment.as_deref();
        let title = assignment.map_or("Assignment", Assignment::title);

        let mut out = format!(
            "{} ({}, assignment {})\n",
            inline(title),
            course.label,
            p.assignment_id
        );
        let state = sub.workflow_state.as_deref().unwrap_or("unknown");
        out.push_str(&format!(
            "Status: {state} · {}\n",
            attempts_text(assignment.and_then(|a| a.allowed_attempts), sub.attempt)
        ));
        if let Some(a) = assignment {
            out.push_str(&format!("Due: {}\n", self.date(a.due_at, "no due date")));
        }
        if sub.submitted_at.is_some() {
            let late = if sub.late == Some(true) {
                " (late)"
            } else {
                ""
            };
            out.push_str(&format!(
                "Submitted: {}{late}\n",
                self.date(sub.submitted_at, "")
            ));
        }
        for (flag, text) in [
            (sub.missing, "Canvas marks this MISSING"),
            (sub.excused, "Excused"),
        ] {
            if flag == Some(true) {
                out.push_str(&format!("{text}\n"));
            }
        }
        out.push_str(&format!(
            "Score: {}\n",
            score_text(&sub, assignment.and_then(|a| a.points_possible))
        ));
        if let Some(kind) = &sub.submission_type {
            out.push_str(&format!("Submitted as: {kind}\n"));
        }
        if let Some(url) = &sub.url {
            out.push_str(&format!("URL: {url}\n"));
        }
        for f in &sub.attachments {
            out.push_str(&format!("File: {} (file id {})\n", f.name(), f.id));
        }
        if let Some(body) = sub.body.as_deref().filter(|b| !b.trim().is_empty()) {
            out.push_str("Your text:\n");
            out.push_str(&html_to_text(body, SUBMISSION_BODY_CHARS));
            out.push('\n');
        }
        if !sub.submission_comments.is_empty() {
            let comments: Vec<String> = sub
                .submission_comments
                .iter()
                .map(|c| {
                    format!(
                        "{} ({}): {}",
                        c.author_name.as_deref().unwrap_or("someone"),
                        self.date(c.created_at, "undated"),
                        c.comment.as_deref().unwrap_or("").trim()
                    )
                })
                .collect();
            out.push_str(&format!("Comments ({}):\n", comments.len()));
            out.push_str(&fence_untrusted(
                &comments.join("\n"),
                "submission comments",
            ));
        }
        Ok(out)
    }

    /// Full details of one assignment: description, due/availability dates, points, how to
    /// submit, allowed file types and attempts, lock status, and your submission status.
    #[tool(annotations(
        title = "Assignment details",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_assignment_details(
        &self,
        Parameters(p): Parameters<AssignmentDetailsParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let a: Assignment = self
            .client()
            .get(
                &format!("courses/{}/assignments/{}", course.path_id, p.assignment_id),
                &[("include[]", "submission")],
            )
            .await
            .map_err(err)?;

        let mut out = format!(
            "{} ({}, assignment {})\n",
            inline(a.title()),
            course.label,
            a.id
        );
        out.push_str(&format!("Due: {}\n", self.date(a.due_at, "no due date")));
        if a.unlock_at.is_some() {
            out.push_str(&format!("Available from: {}\n", self.date(a.unlock_at, "")));
        }
        if a.lock_at.is_some() {
            out.push_str(&format!("Closes: {}\n", self.date(a.lock_at, "")));
        }
        if let Some(pts) = a.points_possible {
            let kind = a.grading_type.as_deref().unwrap_or("points");
            out.push_str(&format!("Points: {} (graded as {kind})\n", points(pts)));
        }
        if a.omit_from_final_grade == Some(true) {
            out.push_str("Does not count toward the final grade\n");
        }
        if !a.submission_types.is_empty() {
            out.push_str(&format!("Submit as: {}\n", a.submission_types.join(", ")));
        }
        if !a.allowed_extensions.is_empty() {
            out.push_str(&format!(
                "Allowed file types: {}\n",
                a.allowed_extensions.join(", ")
            ));
        }
        out.push_str(&format!(
            "Attempts: {}\n",
            attempts_text(
                a.allowed_attempts,
                a.submission.as_ref().and_then(|s| s.attempt)
            )
        ));
        if a.peer_reviews == Some(true) {
            out.push_str("Peer reviews: required (see get_my_peer_reviews_todo)\n");
        }
        if a.is_quiz_lti_assignment == Some(true) {
            out.push_str("This is a New Quiz: use get_quiz_details with this assignment_id\n");
        } else if let Some(quiz_id) = a.quiz_id {
            out.push_str(&format!(
                "This is a quiz: use get_quiz_details with quiz_id {quiz_id}\n"
            ));
        }
        if a.locked_for_user == Some(true) {
            let why = a
                .lock_explanation
                .as_deref()
                .map(inline)
                .unwrap_or_default();
            out.push_str(&format!("Locked for you {why}\n"));
        }
        let status_text = match status(&a) {
            Status::Missing => "MISSING",
            Status::Submitted => "submitted",
            Status::ExternalTool => "done in an external tool (check there; Canvas can't tell)",
            Status::Excused => "excused",
            Status::GradedWithoutSubmission => "graded without a submission",
            Status::NoOnlineSubmission => "no online submission needed",
            Status::NotSubmitted => "not submitted",
        };
        out.push_str(&format!("Your status: {status_text}"));
        if let Some(sub) = &a.submission {
            out.push_str(&format!(" · score {}", score_text(sub, a.points_possible)));
        }
        out.push('\n');
        if let Some(url) = &a.html_url {
            out.push_str(&format!("Link: {url}\n"));
        }
        match a.description.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(html) => {
                let text = html_to_text(html, p.max_chars.unwrap_or(usize::MAX));
                out.push_str("Description:\n");
                out.push_str(&fence_untrusted(
                    &text,
                    &format!("assignment description: {}", a.title()),
                ));
            }
            None => out.push_str("Description: none"),
        }
        Ok(out)
    }
}
