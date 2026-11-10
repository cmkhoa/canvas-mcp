use chrono::{Duration, Utc};
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{
    CanvasServer, CourseAssignmentParams, OptionalCourseParams, ToolResult, err, more_note,
};
use crate::{
    canvas::models::{Assignment, Course, PlannerItem, Submission, TodoItem},
    error::CanvasError,
    format::{html_to_text, points},
    grades,
    untrusted::{fence_untrusted, inline},
};

const MAX_ASSIGNMENTS_PER_COURSE: usize = 300;
const MAX_PLANNER_ITEMS: usize = 500;
const MAX_TODO_ITEMS: usize = 200;
const SUBMISSION_BODY_CHARS: usize = 4000;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpcomingParams {
    /// How many days ahead to look, from now. Default 7, at most 365.
    #[serde(default = "default_days")]
    pub days: u32,
}

fn default_days() -> u32 {
    7
}

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

    fn assignment_line(&self, course: &str, a: &Assignment) -> String {
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
    /// Your assignments, quizzes and graded discussions due in the next N days (default 7)
    /// across all courses, soonest first, with whether you've submitted each.
    #[tool(annotations(
        title = "Upcoming assignments",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_upcoming_assignments(
        &self,
        Parameters(p): Parameters<UpcomingParams>,
    ) -> ToolResult {
        if !(1..=365).contains(&p.days) {
            return Err("days must be between 1 and 365.".into());
        }
        // The planner honors an explicit window (upcoming_events is fixed at 7 days) and
        // carries submission state, saving a request per assignment.
        let start = Utc::now();
        let end = start + Duration::days(i64::from(p.days));
        let (start_s, end_s) = (start.to_rfc3339(), end.to_rfc3339());
        let items = self
            .client()
            .get_all::<PlannerItem>(
                "planner/items",
                &[("start_date", &start_s), ("end_date", &end_s)],
                MAX_PLANNER_ITEMS,
            )
            .await
            .map_err(err)?;
        let labels = self.course_labels().await.unwrap_or_default();

        let mut due: Vec<_> = items
            .items
            .iter()
            .filter_map(|item| {
                let plannable = item.plannable.as_ref()?;
                let kind = item.plannable_type.as_deref()?;
                let due_at = match kind {
                    "assignment" | "quiz" => plannable.due_at.or(item.plannable_date),
                    // Only graded discussions carry due_at; ungraded ones have todo_date.
                    "discussion_topic" => plannable.due_at,
                    _ => None,
                }?;
                (due_at <= end).then_some((due_at, kind, plannable, item))
            })
            .collect();
        due.sort_by_key(|(at, ..)| *at);
        if due.is_empty() {
            return Ok(format!("Nothing due in the next {} days.", p.days));
        }

        let mut out = format!("Due in the next {} days:\n", p.days);
        for (due_at, kind, plannable, item) in due {
            let course = item
                .course_id
                .and_then(|id| labels.get(&id).cloned())
                .or_else(|| item.context_name.clone())
                .unwrap_or_else(|| "unknown course".into());
            let mut line = format!(
                "- {} · {} · {}",
                self.date(Some(due_at), ""),
                course,
                inline(plannable.title.as_deref().unwrap_or("Untitled"))
            );
            if kind != "assignment" {
                line.push_str(&format!(" ({})", kind.replace('_', " ")));
            }
            if let Some(pts) = plannable.points_possible {
                line.push_str(&format!(" · {} pts", points(pts)));
            }
            let state = match &item.submissions {
                Some(s) if s.excused => "excused",
                Some(s) if s.missing => "MISSING",
                Some(s) if s.submitted && s.late => "submitted late",
                Some(s) if s.submitted => "submitted",
                Some(s) if s.graded => "graded",
                Some(_) => "not submitted",
                None => "no submission needed",
            };
            line.push_str(&format!(" · {state}"));
            if let Some(id) = plannable.id.or(item.plannable_id) {
                line.push_str(&format!(" · id {id}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(&more_note(items.truncated, items.items.len()));
        Ok(out)
    }

    /// Your Canvas to-do list: work Canvas thinks you still need to submit.
    #[tool(annotations(title = "To-do list", read_only_hint = true, open_world_hint = true))]
    pub async fn get_my_todo_items(&self) -> ToolResult {
        let todos = self
            .client()
            .get_all::<TodoItem>("users/self/todo", &[], MAX_TODO_ITEMS)
            .await
            .map_err(err)?;
        if todos.items.is_empty() {
            return Ok("Your Canvas to-do list is empty.".into());
        }
        let labels = self.course_labels().await.unwrap_or_default();
        let mut out = String::from("Your Canvas to-do list:\n");
        for t in &todos.items {
            let course = t
                .course_id
                .and_then(|id| labels.get(&id).cloned())
                .or_else(|| t.context_name.clone())
                .unwrap_or_else(|| "unknown course".into());
            match &t.assignment {
                Some(a) => out.push_str(&self.assignment_line(&course, a)),
                None => out.push_str(&format!(
                    "- {course} · {}",
                    t.kind.as_deref().unwrap_or("item")
                )),
            }
            out.push('\n');
        }
        out.push_str(&more_note(todos.truncated, todos.items.len()));
        Ok(out)
    }

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
                "Submitted in an external tool (Canvas can't confirm)",
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
            Status::ExternalTool => "submitted in an external tool (Canvas can't confirm)",
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
