//! Where discussions live (a course or one of your groups) and how posts are rendered.
use futures::future::join_all;

use super::{CanvasServer, err};
use crate::{
    canvas::models::{DiscussionEntry, DiscussionTopic, Group},
    error::CanvasError,
    format::html_to_text,
    resolve::ResolvedCourse,
    untrusted::inline,
};

const MAX_ENTRIES: usize = 500;
const MAX_REPLIES: usize = 500;
const MAX_GROUPS: usize = 200;

/// Where a discussion lives: the course, or one of the caller's groups in it.
pub(super) struct Space {
    pub prefix: String,
    pub label: String,
}

pub(super) fn topic_line(server: &CanvasServer, t: &DiscussionTopic) -> String {
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
    pub(super) async fn space(
        &self,
        course: &ResolvedCourse,
        group_id: Option<u64>,
    ) -> Result<Space, String> {
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

    pub(super) async fn topic(
        &self,
        space: &Space,
        topic_id: u64,
    ) -> Result<DiscussionTopic, String> {
        self.client()
            .get(
                &format!("{}/discussion_topics/{topic_id}", space.prefix),
                &[],
            )
            .await
            .map_err(err)
    }

    /// Top-level entries, explaining Canvas's "post before you can see replies" refusal.
    pub(super) async fn entries(
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
    pub(super) async fn render_entries(
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
