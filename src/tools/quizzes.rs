use std::collections::HashMap;

use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, CourseParams, ToolResult, err, more_note};
use crate::{
    canvas::models::{Assignment, Quiz, QuizSubmissions, Submission},
    error::CanvasError,
    format::{html_to_text, points},
    resolve::ResolvedCourse,
    untrusted::{fence_untrusted, inline},
};

const MAX_QUIZZES: usize = 300;
const MAX_ASSIGNMENTS: usize = 300;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QuizDetailsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// A Classic Quiz ID, as listed by list_quizzes. Pass this or assignment_id, not both.
    #[serde(default)]
    pub quiz_id: Option<u64>,
    /// The assignment ID of a New Quiz (or of a graded Classic Quiz).
    #[serde(default)]
    pub assignment_id: Option<u64>,
}

fn attempts_allowed(n: Option<i64>) -> String {
    match n {
        Some(n) if n > 0 => format!("{n} attempt{}", if n == 1 { "" } else { "s" }),
        _ => "unlimited attempts".into(),
    }
}

fn submission_state(sub: Option<&Submission>) -> &'static str {
    match sub {
        Some(s) if s.excused == Some(true) => "excused",
        Some(s) if s.missing == Some(true) => "MISSING",
        Some(s) if s.submitted_at.is_some() => "submitted",
        Some(s) if s.workflow_state.as_deref() == Some("graded") => "graded",
        _ => "not submitted",
    }
}

impl CanvasServer {
    fn quiz_line(&self, q: &Quiz, sub: Option<&Submission>) -> String {
        let kind = match q.quiz_type.as_deref() {
            Some("assignment") => "graded quiz",
            Some("practice_quiz") => "practice quiz",
            Some("graded_survey") => "graded survey",
            Some("survey") => "survey",
            _ => "quiz",
        };
        let mut line = format!(
            "- [quiz {}] {} · {kind} · due {}",
            q.id,
            inline(q.title.as_deref().unwrap_or("Untitled quiz")),
            self.date(q.due_at, "no due date")
        );
        if let Some(mins) = q.time_limit {
            line.push_str(&format!(" · {mins} min"));
        }
        line.push_str(&format!(" · {}", attempts_allowed(q.allowed_attempts)));
        if let Some(p) = q.points_possible {
            line.push_str(&format!(" · {} pts", points(p)));
        }
        if q.assignment_id.is_some() {
            line.push_str(&format!(" · {}", submission_state(sub)));
        }
        if q.locked_for_user == Some(true) {
            line.push_str(" · locked");
        }
        line
    }

    async fn classic_quiz_details(&self, course: &ResolvedCourse, quiz_id: u64) -> ToolResult {
        let q: Quiz = self
            .client()
            .get(
                &format!("courses/{}/quizzes/{quiz_id}", course.path_id),
                &[],
            )
            .await
            .map_err(err)?;
        let mut out = format!(
            "{} in {}\n",
            self.quiz_line(&q, None).trim_start_matches("- "),
            course.label
        );
        if q.unlock_at.is_some() {
            out.push_str(&format!("Opens: {}\n", self.date(q.unlock_at, "")));
        }
        if q.lock_at.is_some() {
            out.push_str(&format!("Closes: {}\n", self.date(q.lock_at, "")));
        }
        if let Some(n) = q.question_count {
            out.push_str(&format!("Questions: {n}\n"));
        }
        if let Some(policy) = &q.scoring_policy {
            out.push_str(&format!(
                "Score kept: {}\n",
                policy.trim_start_matches("keep_")
            ));
        }
        if q.one_question_at_a_time == Some(true) {
            let back = if q.cant_go_back == Some(true) {
                ", no going back"
            } else {
                ""
            };
            out.push_str(&format!("One question at a time{back}\n"));
        }
        if q.has_access_code == Some(true) {
            out.push_str("Requires an access code from your instructor\n");
        }
        if q.locked_for_user == Some(true)
            && let Some(why) = &q.lock_explanation
        {
            out.push_str(&format!("Locked: {}\n", inline(why)));
        }

        match self
            .client()
            .get::<QuizSubmissions>(
                &format!("courses/{}/quizzes/{quiz_id}/submission", course.path_id),
                &[],
            )
            .await
        {
            Ok(subs) => match subs.quiz_submissions.iter().max_by_key(|s| s.attempt) {
                Some(s) => {
                    let used = s.attempt.unwrap_or(0);
                    out.push_str(&format!("Your attempts: {used} used"));
                    match s.attempts_left {
                        Some(left) if left >= 0 && q.allowed_attempts.is_some_and(|a| a > 0) => {
                            out.push_str(&format!(", {left} left"))
                        }
                        _ => {}
                    }
                    out.push('\n');
                    out.push_str(&format!(
                        "Latest attempt: {}",
                        s.workflow_state.as_deref().unwrap_or("unknown")
                    ));
                    if s.finished_at.is_some() {
                        out.push_str(&format!(", finished {}", self.date(s.finished_at, "")));
                    } else if s.end_at.is_some() {
                        out.push_str(&format!(", in progress until {}", self.date(s.end_at, "")));
                    }
                    out.push('\n');
                    if let Some(score) = s.score {
                        out.push_str(&format!("Latest score: {}\n", points(score)));
                    }
                    if let Some(kept) = s.kept_score {
                        out.push_str(&format!("Kept score: {}\n", points(kept)));
                    }
                }
                None => out.push_str("Your attempts: none yet\n"),
            },
            Err(CanvasError::NotFound(_)) => out.push_str("Your attempts: none yet\n"),
            Err(e) => out.push_str(&format!("Your attempts: unavailable ({e})\n")),
        }
        if let Some(url) = &q.html_url {
            out.push_str(&format!("Link: {url}\n"));
        }
        if let Some(html) = q.description.as_deref().filter(|d| !d.trim().is_empty()) {
            out.push_str("Instructions:\n");
            out.push_str(&fence_untrusted(
                &html_to_text(html, usize::MAX),
                "quiz instructions",
            ));
        }
        Ok(out)
    }
}

#[tool_router(router = quizzes_router, vis = "pub(super)")]
impl CanvasServer {
    /// List the quizzes in a course, Classic and New Quizzes: due/open dates, time limit,
    /// attempts, points and your submission state. Read-only: never opens a quiz, starts an
    /// attempt, or reads questions or answers. Classic quizzes are listed by quiz ID, New
    /// Quizzes by assignment ID.
    #[tool(annotations(title = "Quizzes", read_only_hint = true, open_world_hint = true))]
    pub async fn list_quizzes(&self, Parameters(p): Parameters<CourseParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let quizzes_path = format!("courses/{}/quizzes", course.path_id);
        let assignments_path = format!("courses/{}/assignments", course.path_id);
        let (quizzes, assignments) = futures::join!(
            self.client()
                .get_all::<Quiz>(&quizzes_path, &[], MAX_QUIZZES),
            self.client().get_all::<Assignment>(
                &assignments_path,
                &[("include[]", "submission")],
                MAX_ASSIGNMENTS
            ),
        );
        let assignments = assignments.map_err(err)?;
        let submissions: HashMap<u64, &Submission> = assignments
            .items
            .iter()
            .filter_map(|a| a.submission.as_ref().map(|s| (a.id, s)))
            .collect();

        let mut out = format!("Quizzes in {}:\n", course.label);
        match &quizzes {
            Ok(q) if q.items.is_empty() => out.push_str("Classic quizzes: none\n"),
            Ok(q) => {
                out.push_str("Classic quizzes:\n");
                for quiz in &q.items {
                    let sub = quiz
                        .assignment_id
                        .and_then(|id| submissions.get(&id).copied());
                    out.push_str(&self.quiz_line(quiz, sub));
                    out.push('\n');
                }
                out.push_str(&more_note(q.truncated, q.items.len()));
            }
            // Canvas answers 404 "That page has been disabled" when the Quizzes tab is off.
            Err(CanvasError::NotFound(_)) => out
                .push_str("Classic quizzes: none (the Quizzes tab is turned off in this course)\n"),
            Err(e) => out.push_str(&format!("Classic quizzes: unavailable ({e})\n")),
        }
        let new_quizzes: Vec<&Assignment> = assignments
            .items
            .iter()
            .filter(|a| a.is_quiz_lti_assignment == Some(true))
            .collect();
        if new_quizzes.is_empty() {
            out.push_str("New Quizzes: none\n");
        } else {
            out.push_str("New Quizzes:\n");
            for a in new_quizzes {
                let mut line = format!(
                    "- [assignment {}] {} · due {} · {}",
                    a.id,
                    inline(a.title()),
                    self.date(a.due_at, "no due date"),
                    attempts_allowed(a.allowed_attempts)
                );
                if let Some(p) = a.points_possible {
                    line.push_str(&format!(" · {} pts", points(p)));
                }
                line.push_str(&format!(" · {}", submission_state(a.submission.as_ref())));
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push_str("\nget_quiz_details shows one quiz's settings and your attempts.");
        Ok(out)
    }

    /// One quiz's settings and YOUR OWN attempts (used, remaining, latest and kept score).
    /// Pass exactly one of quiz_id (Classic Quiz) or assignment_id (New Quiz, or a graded
    /// Classic Quiz's assignment). Read-only: never starts an attempt or reads questions.
    #[tool(annotations(title = "Quiz details", read_only_hint = true, open_world_hint = true))]
    pub async fn get_quiz_details(
        &self,
        Parameters(p): Parameters<QuizDetailsParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        match (p.quiz_id, p.assignment_id) {
            (Some(quiz_id), None) => self.classic_quiz_details(&course, quiz_id).await,
            (None, Some(assignment_id)) => {
                let a: Assignment = self
                    .client()
                    .get(
                        &format!("courses/{}/assignments/{assignment_id}", course.path_id),
                        &[("include[]", "submission")],
                    )
                    .await
                    .map_err(err)?;
                if let Some(quiz_id) = a.quiz_id {
                    return self.classic_quiz_details(&course, quiz_id).await;
                }
                if a.is_quiz_lti_assignment != Some(true) {
                    return Err(format!(
                        "Assignment {assignment_id} isn't a quiz. Use get_assignment_details for it."
                    ));
                }
                let mut out = format!(
                    "{} (New Quiz, assignment {}) in {}\n",
                    inline(a.title()),
                    a.id,
                    course.label
                );
                out.push_str(&format!("Due: {}\n", self.date(a.due_at, "no due date")));
                if a.unlock_at.is_some() {
                    out.push_str(&format!("Opens: {}\n", self.date(a.unlock_at, "")));
                }
                if a.lock_at.is_some() {
                    out.push_str(&format!("Closes: {}\n", self.date(a.lock_at, "")));
                }
                if let Some(pts) = a.points_possible {
                    out.push_str(&format!("Points: {}\n", points(pts)));
                }
                out.push_str(&format!(
                    "Allowed: {}\n",
                    attempts_allowed(a.allowed_attempts)
                ));
                let sub = a.submission.as_ref();
                out.push_str(&format!(
                    "Your status: {} · {} attempts used\n",
                    submission_state(sub),
                    sub.and_then(|s| s.attempt).unwrap_or(0)
                ));
                if let Some(score) = sub.and_then(|s| s.score) {
                    out.push_str(&format!("Score: {}\n", points(score)));
                }
                out.push_str(
                    "Canvas doesn't expose New Quiz settings (time limit, question count) or per-attempt \
                     history to students; check the quiz in Canvas for those.",
                );
                Ok(out)
            }
            _ => Err("Pass exactly one of quiz_id or assignment_id.".into()),
        }
    }
}
