use std::collections::HashMap;

use chrono::{Duration, Utc};
use futures::future::join_all;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, CourseParams, ToolResult, err, more_note};
use crate::{
    canvas::models::{ActivityItem, ActivitySummary, DiscussionTopic},
    error::CanvasError,
    format::{html_to_text, parse_date_param},
    untrusted::{fence_untrusted, inline},
};

const DEFAULT_ANNOUNCEMENT_DAYS: i64 = 14;
/// Canvas rejects very long `context_codes[]` lists; students can have dozens of courses.
const CONTEXT_CODE_CHUNK: usize = 10;
const MAX_LIMIT: usize = 200;
const MAX_PREVIEW_CHARS: usize = 2000;
const MAX_COURSE_ANNOUNCEMENTS: usize = 100;
/// Statuses where one bad course in a chunk could sink the whole request.
const PER_COURSE_RETRY: [u16; 4] = [400, 401, 403, 404];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MyAnnouncementsParams {
    /// Limit to one course (code, name, ID or sis_course_id:<id>). Omit for all active courses.
    #[serde(default)]
    pub course_identifier: Option<String>,
    /// Window start: YYYY-MM-DD (your time zone) or a full timestamp. Default: 14 days ago.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Window end: YYYY-MM-DD (inclusive) or a full timestamp. Default: now.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Maximum announcements to show, newest first (1-200, default 50).
    #[serde(default = "default_limit_50")]
    pub limit: usize,
    /// Characters of each announcement's text to show (0-2000, default 300; 0 = titles only).
    #[serde(default = "default_preview")]
    pub preview_chars: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    #[default]
    All,
    Announcements,
    Discussions,
    Conversations,
    /// Grades and submission comments.
    Submissions,
    Notifications,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ActivityParams {
    /// Show only one kind: announcements, discussions, conversations, submissions (grades and
    /// submission comments), notifications, or all (default).
    #[serde(default)]
    pub item_type: ActivityKind,
    /// Maximum items to show (1-200, default 30).
    #[serde(default = "default_limit_30")]
    pub limit: usize,
    /// Also show per-kind totals and unread counts (default true).
    #[serde(default = "default_true")]
    pub include_summary: bool,
    /// Characters of text to preview per item (0-2000, default 300; 0 = titles only).
    #[serde(default = "default_preview")]
    pub preview_chars: usize,
}

fn default_limit_50() -> usize {
    50
}
fn default_limit_30() -> usize {
    30
}
fn default_preview() -> usize {
    300
}
fn default_true() -> bool {
    true
}

/// Activity-stream `type` → display category, in display order.
const CATEGORIES: [(&str, &[&str]); 6] = [
    ("Announcements", &["Announcement"]),
    ("Discussions", &["DiscussionTopic", "DiscussionEntry"]),
    ("Inbox conversations", &["Conversation"]),
    ("Grades & submission comments", &["Submission"]),
    ("Notifications", &["Message"]),
    ("Peer review requests", &["AssessmentRequest"]),
];
const OTHER: &str = "Other activity";

fn category(kind: Option<&str>) -> &'static str {
    CATEGORIES
        .iter()
        .find(|(_, kinds)| kind.is_some_and(|k| kinds.contains(&k)))
        .map_or(OTHER, |(label, _)| label)
}

fn kind_matches(filter: ActivityKind, kind: Option<&str>) -> bool {
    let wanted: &[&str] = match filter {
        ActivityKind::All => return true,
        ActivityKind::Announcements => &["Announcement"],
        ActivityKind::Discussions => &["DiscussionTopic", "DiscussionEntry"],
        ActivityKind::Conversations => &["Conversation"],
        ActivityKind::Submissions => &["Submission"],
        ActivityKind::Notifications => &["Message"],
    };
    kind.is_some_and(|k| wanted.contains(&k))
}

fn check_range(name: &str, value: usize, min: usize, max: usize) -> Result<(), String> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be between {min} and {max}."))
    }
}

impl CanvasServer {
    /// One announcement as `[id] date · title · author` plus an optional text preview.
    fn announcement_entry(
        &self,
        course: &str,
        a: &DiscussionTopic,
        preview_chars: usize,
    ) -> String {
        let mut entry = format!(
            "[id {}] {} · {} · {}",
            a.id,
            self.date(a.posted_at.or(a.delayed_post_at), "undated"),
            course,
            inline(a.title())
        );
        if let Some(author) = a.author_name() {
            entry.push_str(&format!(" · by {}", inline(author)));
        }
        if preview_chars > 0
            && let Some(html) = a.message.as_deref()
        {
            let text = html_to_text(html, preview_chars);
            if !text.is_empty() {
                entry.push('\n');
                entry.push_str(&text);
            }
        }
        entry
    }

    async fn announcements_for(
        &self,
        codes: &[String],
        window: &[(&str, &str)],
    ) -> Result<Vec<DiscussionTopic>, CanvasError> {
        let mut query: Vec<(&str, &str)> = codes
            .iter()
            .map(|c| ("context_codes[]", c.as_str()))
            .collect();
        query.extend_from_slice(window);
        Ok(self
            .client()
            .get_all::<DiscussionTopic>("announcements", &query, MAX_LIMIT * 5)
            .await?
            .items)
    }
}

#[tool_router(router = feed_router, vis = "pub(super)")]
impl CanvasServer {
    /// A single course's announcements, newest first (its full history). For recent
    /// announcements across all your courses use list_my_announcements.
    #[tool(annotations(
        title = "Course announcements",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn list_announcements(&self, Parameters(p): Parameters<CourseParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let topics = self
            .client()
            .get_all::<DiscussionTopic>(
                &format!("courses/{}/discussion_topics", course.path_id),
                &[("only_announcements", "true")],
                MAX_COURSE_ANNOUNCEMENTS,
            )
            .await
            .map_err(err)?;
        if topics.items.is_empty() {
            return Ok(format!("{} has no announcements.", course.label));
        }
        let entries: Vec<String> = topics
            .items
            .iter()
            .map(|a| self.announcement_entry(&course.label, a, 300))
            .collect();
        Ok(format!(
            "Announcements in {} (newest first, {}):\n{}{}",
            course.label,
            entries.len(),
            fence_untrusted(
                &entries.join("\n\n"),
                &format!("announcements: {}", course.label)
            ),
            more_note(topics.truncated, topics.items.len())
        ))
    }

    /// Announcements across ALL your active courses in one call, newest first. Default window:
    /// the last 14 days. For one course's full history use list_announcements.
    #[tool(annotations(
        title = "My announcements",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn list_my_announcements(
        &self,
        Parameters(p): Parameters<MyAnnouncementsParams>,
    ) -> ToolResult {
        check_range("limit", p.limit, 1, MAX_LIMIT)?;
        check_range("preview_chars", p.preview_chars, 0, MAX_PREVIEW_CHARS)?;
        let now = Utc::now();
        let end = match &p.end_date {
            Some(d) => parse_date_param(d, self.timezone, true)?,
            None => now,
        };
        let start = match &p.start_date {
            Some(d) => parse_date_param(d, self.timezone, false)?,
            None => end - Duration::days(DEFAULT_ANNOUNCEMENT_DAYS),
        };
        if start > end {
            return Err("start_date is after end_date.".into());
        }

        let labels: HashMap<String, String> = match &p.course_identifier {
            Some(ident) => {
                let course = self.resolve(ident).await?;
                let id = self.numeric_course_id(&course).await?;
                HashMap::from([(format!("course_{id}"), course.label)])
            }
            None => self
                .course_labels()
                .await?
                .into_iter()
                .map(|(id, label)| (format!("course_{id}"), label))
                .collect(),
        };
        if labels.is_empty() {
            return Ok("You have no active courses.".into());
        }
        let mut codes: Vec<String> = labels.keys().cloned().collect();
        codes.sort();

        let (start_s, end_s) = (start.to_rfc3339(), end.to_rfc3339());
        let window = [
            ("start_date", start_s.as_str()),
            ("end_date", end_s.as_str()),
        ];
        let chunks: Vec<&[String]> = codes.chunks(CONTEXT_CODE_CHUNK).collect();
        let results = join_all(
            chunks
                .iter()
                .map(|chunk| self.announcements_for(chunk, &window)),
        )
        .await;

        let mut found = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for (chunk, result) in chunks.iter().zip(results) {
            match result {
                Ok(items) => found.extend(items),
                // One course the token can't read fails the whole chunk; retry course by course.
                Err(e)
                    if chunk.len() > 1
                        && e.status().is_some_and(|s| PER_COURSE_RETRY.contains(&s)) =>
                {
                    let singles = join_all(chunk.iter().map(|code| {
                        let one = std::slice::from_ref(code);
                        async move { (code, self.announcements_for(one, &window).await) }
                    }))
                    .await;
                    for (code, single) in singles {
                        match single {
                            Ok(items) => found.extend(items),
                            Err(e) => failed.push(format!("{} ({e})", labels[code])),
                        }
                    }
                }
                Err(e) => failed.extend(chunk.iter().map(|code| format!("{} ({e})", labels[code]))),
            }
        }
        found.sort_by_key(|a| std::cmp::Reverse(a.posted_at.or(a.delayed_post_at)));
        let total = found.len();
        found.truncate(p.limit);

        let window_text = format!(
            "{} to {}",
            self.date(Some(start), ""),
            self.date(Some(end), "")
        );
        let mut out = if found.is_empty() {
            format!("No announcements from {window_text}.")
        } else {
            let entries: Vec<String> = found
                .iter()
                .map(|a| {
                    let course = a
                        .context_code
                        .as_ref()
                        .and_then(|c| labels.get(c))
                        .map_or("unknown course", String::as_str);
                    self.announcement_entry(course, a, p.preview_chars)
                })
                .collect();
            format!(
                "Announcements from {window_text}, newest first ({} of {total}):\n{}",
                found.len(),
                fence_untrusted(&entries.join("\n\n"), "announcements")
            )
        };
        if total > found.len() {
            out.push_str(&format!(
                "\n({} more; raise limit or narrow the dates.)",
                total - found.len()
            ));
        }
        if !failed.is_empty() {
            out.push_str(&format!("\nCould not read: {}", failed.join("; ")));
        }
        Ok(out)
    }

    /// Your recent Canvas activity across active courses (the dashboard's Recent Activity
    /// feed), grouped by kind: announcements, discussions, inbox, grades and submission
    /// comments, notifications. Reading it doesn't mark anything read. Group activity and
    /// inbox messages not tied to a course aren't included.
    #[tool(annotations(
        title = "Activity stream",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_activity_stream(
        &self,
        Parameters(p): Parameters<ActivityParams>,
    ) -> ToolResult {
        check_range("limit", p.limit, 1, MAX_LIMIT)?;
        check_range("preview_chars", p.preview_chars, 0, MAX_PREVIEW_CHARS)?;
        let fetch_limit = if p.item_type == ActivityKind::All {
            p.limit
        } else {
            MAX_LIMIT * 2
        };
        let items = self
            .client()
            .get_all::<ActivityItem>(
                "users/self/activity_stream",
                &[("only_active_courses", "true")],
                fetch_limit,
            )
            .await
            .map_err(err)?;
        let labels = self.course_labels().await.unwrap_or_default();

        let mut out = String::new();
        if p.include_summary {
            match self
                .client()
                .get::<Vec<ActivitySummary>>(
                    "users/self/activity_stream/summary",
                    &[("only_active_courses", "true")],
                )
                .await
            {
                Ok(rows) => {
                    let mut totals: Vec<(&str, u64, u64)> = Vec::new();
                    for row in &rows {
                        let label = category(row.kind.as_deref());
                        match totals.iter_mut().find(|(l, ..)| *l == label) {
                            Some(t) => {
                                t.1 += row.count;
                                t.2 += row.unread_count;
                            }
                            None => totals.push((label, row.count, row.unread_count)),
                        }
                    }
                    out.push_str("Activity summary (active courses):\n");
                    for (label, total, unread) in totals {
                        out.push_str(&format!("  {label}: {total} ({unread} unread)\n"));
                    }
                }
                Err(e) => out.push_str(&format!("Summary unavailable: {e}\n")),
            }
            out.push('\n');
        }

        let mut shown: Vec<&ActivityItem> = items
            .items
            .iter()
            .filter(|i| kind_matches(p.item_type, i.kind.as_deref()))
            .collect();
        shown.sort_by_key(|i| std::cmp::Reverse(i.updated_at.or(i.created_at)));
        shown.truncate(p.limit);
        if shown.is_empty() {
            out.push_str("No recent activity.");
            return Ok(out);
        }

        let mut sections = Vec::new();
        for label in CATEGORIES.iter().map(|(l, _)| *l).chain([OTHER]) {
            let in_section: Vec<&&ActivityItem> = shown
                .iter()
                .filter(|i| category(i.kind.as_deref()) == label)
                .collect();
            if in_section.is_empty() {
                continue;
            }
            let mut section = format!("== {label} ({})", in_section.len());
            for i in in_section {
                let course = i
                    .course_id
                    .and_then(|id| labels.get(&id).cloned())
                    .unwrap_or_else(|| "no course".into());
                section.push_str(&format!(
                    "\n- {} · {} · {}",
                    self.date(i.updated_at.or(i.created_at), "undated"),
                    course,
                    inline(i.title.as_deref().unwrap_or("Untitled"))
                ));
                if i.read_state == Some(false) {
                    section.push_str(" · unread");
                }
                if let Some(grade) = &i.grade {
                    section.push_str(&format!(" · grade {}", inline(grade)));
                }
                if p.preview_chars > 0
                    && let Some(msg) = i.message.as_deref()
                {
                    let text = html_to_text(msg, p.preview_chars);
                    if !text.is_empty() {
                        section.push_str(&format!("\n  {}", text.replace('\n', "\n  ")));
                    }
                }
            }
            sections.push(section);
        }
        out.push_str(&fence_untrusted(&sections.join("\n\n"), "activity stream"));
        Ok(out)
    }
}
