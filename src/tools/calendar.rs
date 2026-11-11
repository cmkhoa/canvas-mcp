use std::collections::HashMap;

use chrono::{DateTime, Duration, TimeZone, Utc};
use futures::future::join_all;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err};
use crate::{
    canvas::models::{CalendarEvent, PlannerNote, Profile},
    error::CanvasError,
    format::{html_to_text, parse_date_param},
    untrusted::{fence_untrusted, inline},
};

/// Canvas caps how many contexts one calendar request may name.
const CONTEXT_CHUNK: usize = 10;
const MAX_EVENTS: usize = 500;
const MAX_NOTES: usize = 300;
const DEFAULT_DAYS: i64 = 14;
const MAX_DAYS: i64 = 366;
const DESCRIPTION_CHARS: usize = 500;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// Events and assignment due dates (default).
    #[default]
    All,
    /// Calendar events only.
    Event,
    /// Assignment due dates only.
    Assignment,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CalendarParams {
    /// Window start: YYYY-MM-DD (your time zone) or a full timestamp. Default: today.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Window end: YYYY-MM-DD (inclusive) or a full timestamp. Default: start + days.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Window length in days when end_date is omitted (default 14, at most 366).
    #[serde(default)]
    pub days: Option<i64>,
    /// Limit to one course (code, name, ID or sis_course_id:<id>). Default: all active courses,
    /// your personal calendar and your groups.
    #[serde(default)]
    pub course_identifier: Option<String>,
    /// all (default), event, or assignment.
    #[serde(default)]
    pub event_type: EventType,
    /// Include each event's description (default false).
    #[serde(default)]
    pub include_descriptions: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EventParams {
    /// Calendar event ID (from list_calendar_events; numeric events only).
    pub event_id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NotesParams {
    /// Window start: YYYY-MM-DD (your time zone) or a full timestamp. Default: today.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Window end: YYYY-MM-DD (inclusive) or a full timestamp. Default: start + days.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Window length in days when end_date is omitted (default 14, at most 366).
    #[serde(default)]
    pub days: Option<i64>,
    /// Only notes attached to this course.
    #[serde(default)]
    pub course_identifier: Option<String>,
}

/// Course code for course events (like other tools), else Canvas's context name.
fn context_label(e: &CalendarEvent, labels: &HashMap<u64, String>) -> String {
    let course = e
        .context_code
        .as_deref()
        .and_then(|c| c.strip_prefix("course_"))
        .and_then(|id| id.parse::<u64>().ok())
        .and_then(|id| labels.get(&id));
    match (course, &e.context_name, e.context_code.as_deref()) {
        (Some(label), ..) => label.clone(),
        (None, _, Some(code)) if code.starts_with("user_") => "personal".to_owned(),
        (None, Some(name), _) => inline(name),
        _ => "personal".to_owned(),
    }
}

impl CanvasServer {
    /// Resolves a start/end/days window; defaults to today through the next 14 days.
    fn window(
        &self,
        start: Option<&str>,
        end: Option<&str>,
        days: Option<i64>,
    ) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
        if let Some(d) = days
            && !(1..=MAX_DAYS).contains(&d)
        {
            return Err(format!("days must be between 1 and {MAX_DAYS}."));
        }
        let start = match start {
            Some(s) => parse_date_param(s, self.timezone, false)?,
            None => {
                let today = Utc::now().with_timezone(&self.timezone).date_naive();
                self.timezone
                    .from_local_datetime(&today.and_time(chrono::NaiveTime::MIN))
                    .earliest()
                    .map_or_else(Utc::now, |t| t.with_timezone(&Utc))
            }
        };
        let end = match end {
            Some(e) => parse_date_param(e, self.timezone, true)?,
            None => start + Duration::days(days.unwrap_or(DEFAULT_DAYS)),
        };
        if start > end {
            return Err("start_date is after end_date.".into());
        }
        if end - start > Duration::days(MAX_DAYS) {
            return Err(format!("The window can be at most {MAX_DAYS} days."));
        }
        Ok((start, end))
    }

    async fn events_for(
        &self,
        codes: &[String],
        kind: &str,
        start: &str,
        end: &str,
    ) -> Result<Vec<CalendarEvent>, CanvasError> {
        let mut query: Vec<(&str, &str)> =
            vec![("type", kind), ("start_date", start), ("end_date", end)];
        query.extend(codes.iter().map(|c| ("context_codes[]", c.as_str())));
        Ok(self
            .client()
            .get_all::<CalendarEvent>("calendar_events", &query, MAX_EVENTS)
            .await?
            .items)
    }

    fn event_line(
        &self,
        e: &CalendarEvent,
        labels: &HashMap<u64, String>,
        include_description: bool,
    ) -> String {
        // Assignment events are marked all-day; their due time is what matters.
        let due = e.assignment.as_ref().and_then(|a| a.due_at);
        let when = match (e.all_day, e.start_at) {
            _ if due.is_some() => self.date(due, ""),
            (Some(true), Some(start)) => format!(
                "{} (all day)",
                start.with_timezone(&self.timezone).format("%a %b %-d")
            ),
            _ => self.date(e.start_at, "no time"),
        };
        let mut line = format!(
            "- {when} · {} · {}",
            context_label(e, labels),
            inline(e.title.as_deref().unwrap_or("Untitled"))
        );
        if let Some(a) = &e.assignment {
            line.push_str(&format!(" (assignment {})", a.id));
        } else {
            if let Some(end) = e
                .end_at
                .filter(|end| Some(*end) != e.start_at && e.all_day != Some(true))
            {
                line.push_str(&format!(
                    " until {}",
                    end.with_timezone(&self.timezone).format("%-I:%M %p")
                ));
            }
            if let Some(place) = e.location_name.as_deref().filter(|p| !p.trim().is_empty()) {
                line.push_str(&format!(" · at {}", inline(place)));
            }
            line.push_str(&format!(" · event {}", e.id_text()));
        }
        if include_description && let Some(html) = e.description.as_deref() {
            let text = html_to_text(html, DESCRIPTION_CHARS);
            if !text.is_empty() {
                line.push_str(&format!("\n  {}", text.replace('\n', "\n  ")));
            }
        }
        line
    }
}

#[tool_router(router = calendar_router, vis = "pub(super)")]
impl CanvasServer {
    /// Your Canvas calendar in date order: course events, personal events, group events and
    /// assignment due dates. Covers all active courses, your personal calendar and your groups
    /// unless course_identifier narrows it. Default window: today plus 14 days. For submission
    /// status use get_my_upcoming_assignments instead.
    #[tool(annotations(title = "Calendar", read_only_hint = true, open_world_hint = true))]
    pub async fn list_calendar_events(
        &self,
        Parameters(p): Parameters<CalendarParams>,
    ) -> ToolResult {
        let (start, end) = self.window(p.start_date.as_deref(), p.end_date.as_deref(), p.days)?;
        let codes: Vec<String> = match &p.course_identifier {
            Some(ident) => {
                let course = self.resolve(ident).await?;
                vec![format!("course_{}", self.numeric_course_id(&course).await?)]
            }
            None => {
                let me: Profile = self
                    .client()
                    .get("users/self/profile", &[])
                    .await
                    .map_err(err)?;
                let courses = self.courses().active_courses().await.map_err(err)?;
                let groups = self.my_groups().await.unwrap_or_default();
                std::iter::once(format!("user_{}", me.id))
                    .chain(courses.iter().map(|c| format!("course_{}", c.id)))
                    .chain(groups.iter().map(|g| format!("group_{}", g.id)))
                    .collect()
            }
        };
        let kinds: &[&str] = match p.event_type {
            EventType::All => &["event", "assignment"],
            EventType::Event => &["event"],
            EventType::Assignment => &["assignment"],
        };
        let (start_s, end_s) = (start.to_rfc3339(), end.to_rfc3339());
        let requests: Vec<(&[String], &str)> = codes
            .chunks(CONTEXT_CHUNK)
            .flat_map(|chunk| kinds.iter().map(move |k| (chunk, *k)))
            .collect();
        let results = join_all(
            requests
                .iter()
                .map(|(chunk, kind)| self.events_for(chunk, kind, &start_s, &end_s)),
        )
        .await;

        let mut events = Vec::new();
        let mut failures = Vec::new();
        for result in results {
            match result {
                Ok(found) => events.extend(found),
                Err(e) => failures.push(e.to_string()),
            }
        }
        events.sort_by_key(|e| {
            e.start_at
                .or_else(|| e.assignment.as_ref().and_then(|a| a.due_at))
        });
        events.dedup_by(|a, b| a.id == b.id);

        let labels = self.course_labels().await.unwrap_or_default();
        let window = format!(
            "{} to {}",
            self.date(Some(start), ""),
            self.date(Some(end), "")
        );
        let mut out = if events.is_empty() {
            format!("Nothing on your calendar from {window}.")
        } else {
            let lines: Vec<String> = events
                .iter()
                .map(|e| self.event_line(e, &labels, p.include_descriptions))
                .collect();
            format!(
                "Calendar from {window} ({}):\n{}",
                events.len(),
                fence_untrusted(&lines.join("\n"), "calendar")
            )
        };
        if !failures.is_empty() {
            failures.dedup();
            out.push_str(&format!(
                "\nSome calendars couldn't be read: {}",
                failures.join("; ")
            ));
        }
        Ok(out)
    }

    /// One calendar event in full, including its description and location.
    #[tool(annotations(
        title = "Calendar event",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_calendar_event(&self, Parameters(p): Parameters<EventParams>) -> ToolResult {
        let e: CalendarEvent = self
            .client()
            .get(&format!("calendar_events/{}", p.event_id), &[])
            .await
            .map_err(err)?;
        let labels = self.course_labels().await.unwrap_or_default();
        let mut out = format!(
            "{}\n",
            self.event_line(&e, &labels, false).trim_start_matches("- ")
        );
        if e.end_at.is_some() {
            out.push_str(&format!("Ends: {}\n", self.date(e.end_at, "")));
        }
        if let Some(address) = e
            .location_address
            .as_deref()
            .filter(|a| !a.trim().is_empty())
        {
            out.push_str(&format!("Address: {}\n", inline(address)));
        }
        if let Some(url) = &e.html_url {
            out.push_str(&format!("Link: {url}\n"));
        }
        let text = html_to_text(e.description.as_deref().unwrap_or(""), usize::MAX);
        out.push_str(&fence_untrusted(
            if text.is_empty() {
                "(no description)"
            } else {
                &text
            },
            "calendar event description",
        ));
        Ok(out)
    }

    /// Your own planner notes (personal to-do items you added in Canvas) in a date window.
    /// Default: today plus 14 days.
    #[tool(annotations(title = "Planner notes", read_only_hint = true, open_world_hint = true))]
    pub async fn list_planner_notes(&self, Parameters(p): Parameters<NotesParams>) -> ToolResult {
        let (start, end) = self.window(p.start_date.as_deref(), p.end_date.as_deref(), p.days)?;
        let (start_s, end_s) = (start.to_rfc3339(), end.to_rfc3339());
        let mut query = vec![
            ("start_date", start_s.as_str()),
            ("end_date", end_s.as_str()),
        ];
        let code;
        if let Some(ident) = &p.course_identifier {
            let course = self.resolve(ident).await?;
            code = format!("course_{}", self.numeric_course_id(&course).await?);
            query.push(("context_codes[]", &code));
        }
        let notes = self
            .client()
            .get_all::<PlannerNote>("planner_notes", &query, MAX_NOTES)
            .await
            .map_err(err)?;
        if notes.items.is_empty() {
            return Ok("No planner notes in that window.".into());
        }
        let labels = self.course_labels().await.unwrap_or_default();
        let mut out = format!("Your planner notes ({}):\n", notes.items.len());
        for n in &notes.items {
            out.push_str(&format!(
                "- [note {}] {} · {}",
                n.id,
                self.date(n.todo_date, "no date"),
                inline(n.title.as_deref().unwrap_or("Untitled"))
            ));
            if let Some(c) = n.course_id {
                out.push_str(&format!(
                    " · {}",
                    labels.get(&c).map_or("course", String::as_str)
                ));
            }
            if n.workflow_state.as_deref() == Some("deleted") {
                out.push_str(" · deleted");
            }
            if let (Some(kind), Some(id)) = (&n.linked_object_type, n.linked_object_id) {
                out.push_str(&format!(" · linked to {kind} {id}"));
            }
            if let Some(details) = n.details.as_deref().filter(|d| !d.trim().is_empty()) {
                out.push_str(&format!("\n  {}", details.trim().replace('\n', "\n  ")));
            }
            out.push('\n');
        }
        out.push_str(&super::more_note(notes.truncated, notes.items.len()));
        Ok(out)
    }
}
