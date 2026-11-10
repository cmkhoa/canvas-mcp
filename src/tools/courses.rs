use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, CourseParams, ToolResult, err};
use crate::{
    canvas::models::Course,
    format::{html_to_text, truncate},
    untrusted::fence_untrusted,
};

const MAX_COURSES: usize = 500;
const SYLLABUS_PREVIEW_CHARS: usize = 400;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConcludedParams {
    /// Also include courses that have ended. Default: only current courses.
    #[serde(default)]
    pub include_concluded: bool,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SyllabusFormat {
    /// Readable text converted from the HTML (default).
    #[default]
    Text,
    /// The raw HTML Canvas stores.
    Html,
    /// Text followed by the raw HTML.
    Both,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SyllabusParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// `text` (default), `html`, or `both`.
    #[serde(default)]
    pub output_format: SyllabusFormat,
    /// Optional cap on characters returned. Omit for the complete syllabus; any cut is stated.
    #[serde(default)]
    pub max_chars: Option<usize>,
}

/// One line per course: `CS 101 — Intro to CS (id 101) · Fall 2026 · you: student`.
fn course_line(c: &Course) -> String {
    let mut line = match (&c.course_code, &c.name) {
        (Some(code), Some(name)) if code != name => format!("{code} — {name} (id {})", c.id),
        _ => format!("{} (id {})", c.label(), c.id),
    };
    if let Some(term) = c.term.as_ref().and_then(|t| t.name.as_deref()) {
        line.push_str(&format!(" · {term}"));
    }
    let roles = c.my_roles();
    if !roles.is_empty() {
        line.push_str(&format!(" · you: {}", roles.join(", ")));
    }
    if c.access_restricted_by_date == Some(true) {
        line.push_str(" · closed (access restricted by date)");
    }
    line
}

#[tool_router(router = courses_router, vis = "pub(super)")]
impl CanvasServer {
    /// List the courses you are enrolled in, with your role in each (student, TA, …).
    /// Use this for any question about your own enrollment; it needs no roster access.
    #[tool(annotations(
        title = "My enrollments",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_enrollments(
        &self,
        Parameters(p): Parameters<ConcludedParams>,
    ) -> ToolResult {
        // /courses carries the caller's enrollments alongside course names in one call.
        // enrollment_state is dropped (not widened) for concluded courses: absent means any state.
        let mut query = vec![("include[]", "term")];
        if p.include_concluded {
            query.extend([
                ("state[]", "available"),
                ("state[]", "unpublished"),
                ("state[]", "completed"),
            ]);
        } else {
            query.extend([
                ("enrollment_state", "active"),
                ("state[]", "available"),
                ("state[]", "unpublished"),
            ]);
        }
        let courses = self
            .client()
            .get_all::<Course>("courses", &query, MAX_COURSES)
            .await
            .map_err(err)?;
        if courses.items.is_empty() {
            let scope = if p.include_concluded { "" } else { " active" };
            return Ok(format!(
                "You have no{scope} course enrollments visible to this token."
            ));
        }
        let mut out = String::from("Your enrollments:\n");
        for c in &courses.items {
            out.push_str(&format!("- {}\n", course_line(c)));
        }
        if !p.include_concluded {
            out.push_str(
                "\nActive enrollments only. Pass include_concluded=true for past courses.",
            );
        }
        out.push_str(&super::more_note(courses.truncated, courses.items.len()));
        Ok(out)
    }

    /// List your courses with course code, name, ID and term. Course codes from here work as
    /// `course_identifier` in every other tool.
    #[tool(annotations(title = "List courses", read_only_hint = true, open_world_hint = true))]
    pub async fn list_courses(&self, Parameters(p): Parameters<ConcludedParams>) -> ToolResult {
        let (courses, truncated) = if p.include_concluded {
            let paged = self
                .client()
                .get_all::<Course>(
                    "courses",
                    &[
                        ("include[]", "term"),
                        ("state[]", "available"),
                        ("state[]", "completed"),
                    ],
                    MAX_COURSES,
                )
                .await
                .map_err(err)?;
            (paged.items, paged.truncated)
        } else {
            let cached = self.courses().active_courses().await.map_err(err)?;
            (cached.as_ref().clone(), false)
        };
        if courses.is_empty() {
            return Ok("No courses found.".into());
        }
        let mut out = String::from("Courses:\n");
        for c in &courses {
            out.push_str(&format!("- {}\n", course_line(c)));
        }
        out.push_str(&super::more_note(truncated, courses.len()));
        Ok(out)
    }

    /// Get one course's details: code, name, term, dates, time zone, your role and a short
    /// syllabus preview. Use get_syllabus for the full syllabus.
    #[tool(annotations(
        title = "Course details",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_course_details(&self, Parameters(p): Parameters<CourseParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let c: Course = self
            .client()
            .get(
                &format!("courses/{}", course.path_id),
                &[("include[]", "term"), ("include[]", "syllabus_body")],
            )
            .await
            .map_err(err)?;

        let mut out = format!("{}\n", course_line(&c));
        out.push_str(&format!("Starts: {}\n", self.date(c.start_at, "not set")));
        out.push_str(&format!("Ends: {}\n", self.date(c.end_at, "not set")));
        if let Some(tz) = &c.time_zone {
            out.push_str(&format!("Course time zone: {tz}\n"));
        }
        if let Some(view) = &c.default_view {
            out.push_str(&format!("Home page shows: {view}\n"));
        }
        if c.my_roles().is_empty() {
            out.push_str("Your role: none (you are not enrolled in this course)\n");
        }
        match c
            .syllabus_body
            .as_deref()
            .map(|h| html_to_text(h, SYLLABUS_PREVIEW_CHARS))
        {
            Some(text) if !text.is_empty() => {
                out.push_str("Syllabus preview (get_syllabus has the full text):\n");
                out.push_str(&fence_untrusted(&text, &format!("syllabus: {}", c.label())));
            }
            _ => out.push_str("Syllabus: empty"),
        }
        Ok(out)
    }

    /// Get the complete Syllabus tab of a course. Complete by default; pass max_chars to cap it
    /// (the cut is stated).
    #[tool(annotations(title = "Syllabus", read_only_hint = true, open_world_hint = true))]
    pub async fn get_syllabus(&self, Parameters(p): Parameters<SyllabusParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let c: Course = self
            .client()
            .get(
                &format!("courses/{}", course.path_id),
                &[("include[]", "syllabus_body")],
            )
            .await
            .map_err(err)?;
        let html = c.syllabus_body.as_deref().unwrap_or_default();
        if html.trim().is_empty() {
            return Ok(format!(
                "{} has no syllabus on its Syllabus tab. Course content may be in pages or modules instead.",
                c.label()
            ));
        }
        let limit = p.max_chars.unwrap_or(usize::MAX);
        let body = match p.output_format {
            SyllabusFormat::Text => html_to_text(html, limit),
            SyllabusFormat::Html => truncate(html, limit),
            SyllabusFormat::Both => format!(
                "{}\n\n--- HTML ---\n{}",
                html_to_text(html, limit),
                truncate(html, limit)
            ),
        };
        Ok(format!(
            "Syllabus for {}:\n{}",
            c.label(),
            fence_untrusted(&body, &format!("syllabus: {}", c.label()))
        ))
    }
}
