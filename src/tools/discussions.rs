use futures::future::join_all;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err, more_note};
use crate::{
    canvas::models::{DiscussionEntry, DiscussionTopic, Group},
    error::CanvasError,
    format::html_to_text,
    resolve::ResolvedCourse,
    untrusted::{fence_untrusted, inline},
};

const MAX_TOPICS: usize = 200;
const MAX_ENTRIES: usize = 500;
const MAX_REPLIES: usize = 500;
const MAX_GROUPS: usize = 200;
const PREVIEW_CHARS: usize = 300;

/// Doc shared by every `group_id` parameter.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TopicsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Also list announcements (default false; list_announcements lists them on their own).
    #[serde(default)]
    pub include_announcements: bool,
    /// Read inside one of your group spaces instead of the course (from list_my_groups).
    /// Discussions students start in a group exist only there.
    #[serde(default)]
    pub group_id: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GroupTopicsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Only groups in this group set (category), from list_my_groups.
    #[serde(default)]
    pub group_category_id: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EntriesParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Discussion topic ID (from list_discussion_topics).
    pub topic_id: u64,
    /// Show each post in full instead of a 300-character preview (default false).
    #[serde(default)]
    pub include_full_content: bool,
    /// Also fetch every reply to each post (default false).
    #[serde(default)]
    pub include_replies: bool,
    /// Read inside one of your group spaces instead of the course.
    #[serde(default)]
    pub group_id: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TopicParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Discussion topic ID (from list_discussion_topics).
    pub topic_id: u64,
    /// Read inside one of your group spaces instead of the course.
    #[serde(default)]
    pub group_id: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EntryParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Discussion topic ID.
    pub topic_id: u64,
    /// Entry (post) ID, from list_discussion_entries.
    pub entry_id: u64,
    /// Also fetch all replies to the entry (default true).
    #[serde(default = "default_true")]
    pub include_replies: bool,
    /// Read inside one of your group spaces instead of the course.
    #[serde(default)]
    pub group_id: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WithRepliesParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Discussion topic ID.
    pub topic_id: u64,
    /// Also fetch every reply, in full (default false).
    #[serde(default)]
    pub include_replies: bool,
    /// Read inside one of your group spaces instead of the course.
    #[serde(default)]
    pub group_id: Option<u64>,
}

fn default_true() -> bool {
    true
}

/// Where a discussion lives: the course, or one of the caller's groups in it.
struct Space {
    prefix: String,
    label: String,
}

fn topic_line(server: &CanvasServer, t: &DiscussionTopic) -> String {
    let mut line = format!("- [id {}] {}", t.id, inline(t.title()));
    if let Some(author) = t.author_name() {
        line.push_str(&format!(" · by {}", inline(author)));
    }
    line.push_str(&format!(
        " · posted {}",
        server.date(t.posted_at, "not yet")
    ));
    if let Some(n) = t.discussion_subentry_count {
        line.push_str(&format!(" · {n} posts"));
        if let Some(u) = t.unread_count.filter(|u| *u > 0) {
            line.push_str(&format!(" ({u} unread)"));
        }
    }
    if t.last_reply_at.is_some() {
        line.push_str(&format!(
            " · last reply {}",
            server.date(t.last_reply_at, "")
        ));
    }
    if let Some(a) = &t.assignment {
        line.push_str(&format!(
            " · graded, due {} (assignment {})",
            server.date(a.due_at, "no due date"),
            a.id
        ));
    } else if let Some(id) = t.assignment_id {
        line.push_str(&format!(" · graded (assignment {id})"));
    }
    if t.require_initial_post == Some(true) {
        line.push_str(" · post first to see others");
    }
    if t.pinned == Some(true) {
        line.push_str(" · pinned");
    }
    if t.locked == Some(true) {
        line.push_str(" · locked");
    }
    line
}

fn entry_header(e: &DiscussionEntry, server: &CanvasServer, kind: &str) -> String {
    let mut h = format!(
        "[{kind} {}] {} · {}",
        e.id,
        inline(e.user_name.as_deref().unwrap_or("unknown")),
        server.date(e.created_at, "undated")
    );
    if e.read_state.as_deref() == Some("unread") {
        h.push_str(" · unread");
    }
    h
}

fn entry_text(e: &DiscussionEntry, max_chars: usize) -> String {
    if e.deleted == Some(true) {
        return "(deleted)".into();
    }
    html_to_text(e.message.as_deref().unwrap_or(""), max_chars)
}

fn indent(text: &str, by: &str) -> String {
    text.lines()
        .map(|l| format!("{by}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl CanvasServer {
    pub(crate) async fn my_groups(&self) -> Result<Vec<Group>, String> {
        self.client()
            .get_all::<Group>("users/self/groups", &[], MAX_GROUPS)
            .await
            .map(|p| p.items)
            .map_err(err)
    }

    /// The API prefix for a course, or for one of the caller's groups in it. Group IDs are
    /// accepted only for groups the caller belongs to that sit in the named course.
    async fn space(&self, course: &ResolvedCourse, group_id: Option<u64>) -> Result<Space, String> {
        let Some(group_id) = group_id else {
            return Ok(Space {
                prefix: format!("courses/{}", course.path_id),
                label: course.label.clone(),
            });
        };
        let course_id = self.numeric_course_id(course).await?;
        let group = self
            .my_groups()
            .await?
            .into_iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| {
                format!(
                    "You are not a member of group {group_id}. list_my_groups shows your groups."
                )
            })?;
        if group.course_id != Some(course_id) {
            return Err(format!(
                "Group {group_id} doesn't belong to {}.",
                course.label
            ));
        }
        let name = group.name.as_deref().unwrap_or("group");
        Ok(Space {
            prefix: format!("groups/{group_id}"),
            label: format!("{} › group {}", course.label, inline(name)),
        })
    }

    async fn topic(&self, space: &Space, topic_id: u64) -> Result<DiscussionTopic, String> {
        self.client()
            .get(
                &format!("{}/discussion_topics/{topic_id}", space.prefix),
                &[],
            )
            .await
            .map_err(err)
    }

    /// Top-level entries, explaining Canvas's "post before you can see replies" refusal.
    async fn entries(
        &self,
        space: &Space,
        topic_id: u64,
    ) -> Result<(Vec<DiscussionEntry>, bool), String> {
        match self
            .client()
            .get_all::<DiscussionEntry>(
                &format!("{}/discussion_topics/{topic_id}/entries", space.prefix),
                &[],
                MAX_ENTRIES,
            )
            .await
        {
            Ok(p) => Ok((p.items, p.truncated)),
            Err(e @ CanvasError::Forbidden(_)) => {
                let requires_post = self
                    .topic(space, topic_id)
                    .await
                    .is_ok_and(|t| t.require_initial_post == Some(true));
                if requires_post {
                    Err(format!(
                        "Topic {topic_id} requires you to post before you can see classmates' posts. \
                         get_discussion_topic_details still shows the prompt."
                    ))
                } else {
                    Err(e.to_string())
                }
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// All replies to an entry, oldest first.
    async fn replies(
        &self,
        space: &Space,
        topic_id: u64,
        entry_id: u64,
    ) -> Result<Vec<DiscussionEntry>, CanvasError> {
        let mut replies = self
            .client()
            .get_all::<DiscussionEntry>(
                &format!(
                    "{}/discussion_topics/{topic_id}/entries/{entry_id}/replies",
                    space.prefix
                ),
                &[],
                MAX_REPLIES,
            )
            .await?
            .items;
        replies.sort_by_key(|r| r.created_at);
        Ok(replies)
    }

    /// Renders entries (and optionally every reply) as text for one untrusted fence.
    async fn render_entries(
        &self,
        space: &Space,
        topic_id: u64,
        entries: &[DiscussionEntry],
        max_chars: usize,
        with_replies: bool,
    ) -> String {
        let reply_sets = if with_replies {
            join_all(entries.iter().map(|e| self.replies(space, topic_id, e.id))).await
        } else {
            Vec::new()
        };
        let mut blocks = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let mut block = entry_header(e, self, "entry");
            block.push('\n');
            block.push_str(&entry_text(e, max_chars));
            match reply_sets.get(i) {
                Some(Ok(replies)) => {
                    for r in replies {
                        block.push_str(&format!(
                            "\n  ↳ {}\n{}",
                            entry_header(r, self, "reply"),
                            indent(&entry_text(r, max_chars), "    ")
                        ));
                    }
                }
                Some(Err(err)) => block.push_str(&format!("\n  (replies unavailable: {err})")),
                None => {
                    let shown = e.recent_replies.len();
                    if shown > 0 || e.has_more_replies == Some(true) {
                        let more = if e.has_more_replies == Some(true) {
                            "+"
                        } else {
                            ""
                        };
                        block.push_str(&format!(
                            "\n  ({shown}{more} replies; pass include_replies=true to read them)"
                        ));
                    }
                }
            }
            blocks.push(block);
        }
        blocks.join("\n\n")
    }
}

#[tool_router(router = discussions_router, vis = "pub(super)")]
impl CanvasServer {
    /// List discussion topics in a course (or, with group_id, one of your group spaces):
    /// title, author, post counts, unread, graded/due, and whether you must post first.
    /// Announcements are excluded unless include_announcements is set.
    #[tool(annotations(
        title = "Discussion topics",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn list_discussion_topics(
        &self,
        Parameters(p): Parameters<TopicsParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let space = self.space(&course, p.group_id).await?;
        let path = format!("{}/discussion_topics", space.prefix);
        let topics = self
            .client()
            .get_all::<DiscussionTopic>(&path, &[("include[]", "assignment")], MAX_TOPICS)
            .await
            .map_err(err)?;
        let mut out = format!("Discussions in {}:\n", space.label);
        if topics.items.is_empty() {
            out.push_str("(none)\n");
        }
        for t in &topics.items {
            out.push_str(&topic_line(self, t));
            out.push('\n');
        }
        out.push_str(&more_note(topics.truncated, topics.items.len()));
        if p.include_announcements {
            let announcements = self
                .client()
                .get_all::<DiscussionTopic>(&path, &[("only_announcements", "true")], MAX_TOPICS)
                .await
                .map_err(err)?;
            out.push_str(&format!(
                "\nAnnouncements ({}):\n",
                announcements.items.len()
            ));
            for t in &announcements.items {
                out.push_str(&topic_line(self, t));
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// List the discussion topics inside every group space of yours in a course, marking each
    /// as a group copy of a course topic or one started in the group. Topics students start
    /// in a group exist only there, so list_discussion_topics doesn't show them.
    #[tool(annotations(
        title = "Group discussions",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn list_group_discussion_topics(
        &self,
        Parameters(p): Parameters<GroupTopicsParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let course_id = self.numeric_course_id(&course).await?;
        let groups: Vec<Group> = self
            .my_groups()
            .await?
            .into_iter()
            .filter(|g| g.course_id == Some(course_id))
            .filter(|g| {
                p.group_category_id
                    .is_none_or(|c| g.group_category_id == Some(c))
            })
            .collect();
        if groups.is_empty() {
            return Ok(format!("You aren't in any groups in {}.", course.label));
        }
        let paths: Vec<String> = groups
            .iter()
            .map(|g| format!("groups/{}/discussion_topics", g.id))
            .collect();
        let results = join_all(paths.iter().map(|path| {
            self.client()
                .get_all::<DiscussionTopic>(path, &[], MAX_TOPICS)
        }))
        .await;
        let mut out = format!("Group discussions in {}:\n", course.label);
        for (g, result) in groups.iter().zip(results) {
            out.push_str(&format!(
                "\n== {} (group {}, set {})\n",
                inline(g.name.as_deref().unwrap_or("group")),
                g.id,
                g.group_category_id.map_or("?".into(), |c| c.to_string())
            ));
            match result {
                Ok(topics) if topics.items.is_empty() => out.push_str("(no topics)\n"),
                Ok(topics) => {
                    for t in &topics.items {
                        let origin = match t.root_topic_id {
                            Some(root) => format!(" · group copy of course topic {root}"),
                            None => " · started in this group".to_owned(),
                        };
                        out.push_str(&format!("{}{origin}\n", topic_line(self, t)));
                    }
                }
                Err(e) => out.push_str(&format!("(could not read: {e})\n")),
            }
        }
        out.push_str("\nRead one with get_discussion_with_replies using the group_id.");
        Ok(out)
    }

    /// The posts in a discussion topic: author, time, a 300-character preview (or full text
    /// with include_full_content) and, with include_replies, every reply.
    #[tool(annotations(
        title = "Discussion posts",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn list_discussion_entries(
        &self,
        Parameters(p): Parameters<EntriesParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let space = self.space(&course, p.group_id).await?;
        let (entries, truncated) = self.entries(&space, p.topic_id).await?;
        if entries.is_empty() {
            return Ok(format!("No posts yet in topic {}.", p.topic_id));
        }
        let max_chars = if p.include_full_content {
            usize::MAX
        } else {
            PREVIEW_CHARS
        };
        let body = self
            .render_entries(&space, p.topic_id, &entries, max_chars, p.include_replies)
            .await;
        Ok(format!(
            "Posts in topic {} ({}), {} top-level:\n{}{}",
            p.topic_id,
            space.label,
            entries.len(),
            fence_untrusted(&body, "discussion posts by course participants"),
            more_note(truncated, entries.len())
        ))
    }

    /// One discussion topic in full: the prompt text, author, dates, graded/due, post counts,
    /// and settings such as "post before seeing replies".
    #[tool(annotations(
        title = "Discussion topic",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_discussion_topic_details(
        &self,
        Parameters(p): Parameters<TopicParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let space = self.space(&course, p.group_id).await?;
        let t = self.topic(&space, p.topic_id).await?;
        let mut out = format!("Topic in {}:\n{}\n", space.label, topic_line(self, &t));
        if let Some(kind) = &t.discussion_type {
            out.push_str(&format!("Format: {kind}\n"));
        }
        if t.todo_date.is_some() {
            out.push_str(&format!("To-do date: {}\n", self.date(t.todo_date, "")));
        }
        if let Some(url) = &t.html_url {
            out.push_str(&format!("Link: {url}\n"));
        }
        let text = html_to_text(t.message.as_deref().unwrap_or(""), usize::MAX);
        out.push_str(&fence_untrusted(
            if text.is_empty() {
                "(no prompt text)"
            } else {
                &text
            },
            &format!("discussion prompt: {}", t.title()),
        ));
        Ok(out)
    }

    /// One post in a discussion with all its replies (in full).
    #[tool(annotations(
        title = "Discussion post",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_discussion_entry_details(
        &self,
        Parameters(p): Parameters<EntryParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let space = self.space(&course, p.group_id).await?;
        let found: Vec<DiscussionEntry> = self
            .client()
            .get(
                &format!(
                    "{}/discussion_topics/{}/entry_list",
                    space.prefix, p.topic_id
                ),
                &[("ids[]", &p.entry_id.to_string())],
            )
            .await
            .map_err(err)?;
        let entry = found
            .into_iter()
            .find(|e| e.id == p.entry_id)
            .ok_or_else(|| format!("Entry {} wasn't found in topic {}.", p.entry_id, p.topic_id))?;
        let body = self
            .render_entries(
                &space,
                p.topic_id,
                std::slice::from_ref(&entry),
                usize::MAX,
                p.include_replies,
            )
            .await;
        let parent = entry
            .parent_id
            .map_or(String::new(), |id| format!(" (a reply to entry {id})"));
        Ok(format!(
            "Entry {} in topic {} ({}){parent}:\n{}",
            entry.id,
            p.topic_id,
            space.label,
            fence_untrusted(&body, "discussion post by a course participant")
        ))
    }

    /// Read every post of a discussion in full, never cut, optionally with all replies.
    /// Returns the topic title (get_discussion_topic_details has the prompt text).
    #[tool(annotations(
        title = "Full discussion",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_discussion_with_replies(
        &self,
        Parameters(p): Parameters<WithRepliesParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let space = self.space(&course, p.group_id).await?;
        let (entries, truncated) = self.entries(&space, p.topic_id).await?;
        let title = self
            .topic(&space, p.topic_id)
            .await
            .map(|t| inline(t.title()))
            .unwrap_or_else(|_| format!("topic {}", p.topic_id));
        if entries.is_empty() {
            return Ok(format!("{title} has no posts yet."));
        }
        let body = self
            .render_entries(&space, p.topic_id, &entries, usize::MAX, p.include_replies)
            .await;
        Ok(format!(
            "Discussion {title} in {} ({} top-level posts):\n{}{}",
            space.label,
            entries.len(),
            fence_untrusted(&body, "discussion posts by course participants"),
            more_note(truncated, entries.len())
        ))
    }
}
