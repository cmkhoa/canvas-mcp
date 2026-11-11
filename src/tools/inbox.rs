use std::collections::HashMap;

use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err, more_note};
use crate::{
    canvas::models::{Conversation, Profile, Recipient},
    format::html_to_text,
    untrusted::{fence_untrusted, inline},
};

const MAX_LIMIT: usize = 200;
const MAX_RECIPIENTS: usize = 100;
const PREVIEW_CHARS: usize = 200;

#[derive(Debug, Default, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Your inbox (default).
    #[default]
    Inbox,
    Unread,
    Starred,
    Archived,
    Sent,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    /// Match any filter (default).
    #[default]
    Or,
    /// Match all filters.
    And,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConversationsParams {
    /// inbox (default), unread, starred, archived or sent.
    #[serde(default)]
    pub scope: Scope,
    /// Only conversations tied to these, e.g. ["course_59112", "user_27096"].
    #[serde(default)]
    pub filter_ids: Vec<String>,
    /// How several filter_ids combine: or (default) or and.
    #[serde(default)]
    pub filter_mode: FilterMode,
    /// List who is in each conversation (default true).
    #[serde(default = "default_true")]
    pub include_participants: bool,
    /// Maximum conversations, newest first (1-200, default 30).
    #[serde(default = "default_limit")]
    pub limit: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConversationParams {
    /// Conversation ID, from list_conversations.
    pub conversation_id: u64,
    /// Include every message in full (default true).
    #[serde(default = "default_true")]
    pub include_messages: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecipientRole {
    /// Anyone in the course (default).
    #[default]
    Any,
    Teacher,
    Ta,
    Student,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecipientsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Part of the person's name.
    #[serde(default)]
    pub search: Option<String>,
    /// any (default), teacher, ta or student.
    #[serde(default)]
    pub role: RecipientRole,
    /// Maximum people (1-100, default 20).
    #[serde(default = "default_recipients")]
    pub limit: usize,
}

fn default_true() -> bool {
    true
}
fn default_limit() -> usize {
    30
}
fn default_recipients() -> usize {
    20
}

/// `course_123`, `group_4` or `user_5`, nothing else (these go into the query string).
fn valid_filter(id: &str) -> bool {
    let Some((kind, num)) = id.split_once('_') else {
        return false;
    };
    matches!(kind, "course" | "group" | "user")
        && !num.is_empty()
        && num.bytes().all(|b| b.is_ascii_digit())
}

/// At most 8 names, then how many more (class-wide messages include everyone).
fn name_list(names: &[String]) -> String {
    const SHOWN: usize = 8;
    if names.len() <= SHOWN {
        return names.join(", ");
    }
    format!(
        "{} and {} more",
        names[..SHOWN].join(", "),
        names.len() - SHOWN
    )
}

/// `TeacherEnrollment` → `teacher`, `TaEnrollment` → `ta`; custom role names pass through.
fn role_name(role: &str) -> String {
    role.strip_suffix("Enrollment")
        .map_or_else(|| role.to_owned(), str::to_lowercase)
}

#[tool_router(router = inbox_router, vis = "pub(super)")]
impl CanvasServer {
    /// Your Canvas Inbox conversations, newest first: subject, participants, unread state and a
    /// preview of the latest message. Doesn't mark anything read.
    #[tool(annotations(title = "Inbox", read_only_hint = true, open_world_hint = true))]
    pub async fn list_conversations(
        &self,
        Parameters(p): Parameters<ConversationsParams>,
    ) -> ToolResult {
        if !(1..=MAX_LIMIT).contains(&p.limit) {
            return Err(format!("limit must be between 1 and {MAX_LIMIT}."));
        }
        if let Some(bad) = p.filter_ids.iter().find(|id| !valid_filter(id)) {
            return Err(format!(
                "filter_ids entries look like course_123, group_4 or user_5; got {bad:?}."
            ));
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        let scope = match p.scope {
            Scope::Inbox => None,
            Scope::Unread => Some("unread"),
            Scope::Starred => Some("starred"),
            Scope::Archived => Some("archived"),
            Scope::Sent => Some("sent"),
        };
        if let Some(scope) = scope {
            query.push(("scope", scope));
        }
        query.extend(p.filter_ids.iter().map(|id| ("filter[]", id.as_str())));
        if p.filter_ids.len() > 1 {
            query.push((
                "filter_mode",
                match p.filter_mode {
                    FilterMode::Or => "or",
                    FilterMode::And => "and",
                },
            ));
        }
        let conversations = self
            .client()
            .get_all::<Conversation>("conversations", &query, p.limit)
            .await
            .map_err(err)?;
        if conversations.items.is_empty() {
            return Ok("No conversations.".into());
        }
        let lines: Vec<String> = conversations
            .items
            .iter()
            .map(|c| {
                let mut line = format!(
                    "[conversation {}] {} · {}",
                    c.id,
                    self.date(c.last_message_at, "undated"),
                    inline(
                        c.subject
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .unwrap_or("(no subject)")
                    )
                );
                if c.workflow_state.as_deref() == Some("unread") {
                    line.push_str(" · unread");
                }
                if c.starred == Some(true) {
                    line.push_str(" · starred");
                }
                if let Some(n) = c.message_count {
                    line.push_str(&format!(" · {n} messages"));
                }
                if let Some(ctx) = &c.context_name {
                    line.push_str(&format!(" · {}", inline(ctx)));
                }
                if p.include_participants && !c.participants.is_empty() {
                    let names: Vec<String> = c
                        .participants
                        .iter()
                        .map(|u| inline(u.name.as_deref().unwrap_or("someone")))
                        .collect();
                    line.push_str(&format!(" · with {}", name_list(&names)));
                }
                if let Some(last) = c.last_message.as_deref().filter(|m| !m.trim().is_empty()) {
                    line.push_str(&format!(
                        "\n  {}",
                        crate::format::truncate(last.trim(), PREVIEW_CHARS).replace('\n', "\n  ")
                    ));
                }
                line
            })
            .collect();
        Ok(format!(
            "Conversations ({}):\n{}{}",
            lines.len(),
            fence_untrusted(&lines.join("\n"), "inbox"),
            more_note(conversations.truncated, conversations.items.len())
        ))
    }

    /// One Inbox conversation with its messages, oldest first. Doesn't mark it read.
    #[tool(annotations(title = "Conversation", read_only_hint = true, open_world_hint = true))]
    pub async fn get_conversation_details(
        &self,
        Parameters(p): Parameters<ConversationParams>,
    ) -> ToolResult {
        // Canvas marks a conversation read when it's fetched unless told not to.
        let c: Conversation = self
            .client()
            .get(
                &format!("conversations/{}", p.conversation_id),
                &[("auto_mark_as_read", "false")],
            )
            .await
            .map_err(err)?;
        let me = self
            .client()
            .get::<Profile>("users/self/profile", &[])
            .await
            .ok()
            .map(|m| m.id);
        let names: HashMap<u64, String> = c
            .participants
            .iter()
            .map(|u| {
                let name = inline(u.name.as_deref().unwrap_or("someone"));
                (
                    u.id,
                    if Some(u.id) == me {
                        format!("{name} (you)")
                    } else {
                        name
                    },
                )
            })
            .collect();
        let mut out = format!(
            "{} (conversation {})",
            inline(
                c.subject
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("(no subject)")
            ),
            c.id
        );
        if let Some(ctx) = &c.context_name {
            out.push_str(&format!(" · {}", inline(ctx)));
        }
        // Canvas's participant order, with you first.
        let mut ordered: Vec<String> = c
            .participants
            .iter()
            .filter_map(|u| names.get(&u.id).cloned())
            .collect();
        ordered.sort_by_key(|n| !n.ends_with("(you)"));
        out.push_str(&format!("\nParticipants: {}\n", name_list(&ordered)));
        if !p.include_messages {
            out.push_str(&format!(
                "{} messages (pass include_messages=true to read them)",
                c.messages.len()
            ));
            return Ok(out);
        }
        let mut messages = c.messages.clone();
        messages.sort_by_key(|m| m.created_at);
        let body: Vec<String> = messages
            .iter()
            .map(|m| {
                let author = m
                    .author_id
                    .and_then(|id| names.get(&id).cloned())
                    .unwrap_or_else(|| "someone".into());
                let mut block = format!(
                    "[message {}] {author} · {}",
                    m.id,
                    self.date(m.created_at, "undated")
                );
                if m.generated == Some(true) {
                    block.push_str(" · system message");
                }
                block.push('\n');
                block.push_str(&html_to_text(m.body.as_deref().unwrap_or(""), usize::MAX));
                for f in &m.attachments {
                    block.push_str(&format!(
                        "\n  attachment: {} (file id {})",
                        inline(f.name()),
                        f.id
                    ));
                }
                block
            })
            .collect();
        out.push_str(&fence_untrusted(&body.join("\n\n"), "inbox conversation"));
        Ok(out)
    }

    /// How many Inbox conversations are unread.
    #[tool(annotations(title = "Unread count", read_only_hint = true, open_world_hint = true))]
    pub async fn get_unread_count(&self) -> ToolResult {
        let v: serde_json::Value = self
            .client()
            .get("conversations/unread_count", &[])
            .await
            .map_err(err)?;
        // Canvas sends the count as a string.
        let count = match &v["unread_count"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => return Err("Canvas didn't return an unread count.".into()),
        };
        Ok(format!("Unread conversations: {count}"))
    }

    /// Find people you can message in a course (instructors, TAs, classmates) with their Canvas
    /// user IDs, optionally by name or role. Individuals only.
    #[tool(annotations(
        title = "Find message recipients",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn find_message_recipients(
        &self,
        Parameters(p): Parameters<RecipientsParams>,
    ) -> ToolResult {
        if !(1..=MAX_RECIPIENTS).contains(&p.limit) {
            return Err(format!("limit must be between 1 and {MAX_RECIPIENTS}."));
        }
        let course = self.resolve(&p.course_identifier).await?;
        let course_id = self.numeric_course_id(&course).await?;
        let context = match p.role {
            RecipientRole::Any => format!("course_{course_id}"),
            RecipientRole::Teacher => format!("course_{course_id}_teachers"),
            RecipientRole::Ta => format!("course_{course_id}_tas"),
            RecipientRole::Student => format!("course_{course_id}_students"),
        };
        let mut query = vec![("context", context.as_str()), ("type", "user")];
        if let Some(search) = p.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            query.push(("search", search));
        }
        let found = self
            .client()
            .get_all::<Recipient>("search/recipients", &query, p.limit)
            .await
            .map_err(err)?;
        let people: Vec<&Recipient> = found
            .items
            .iter()
            .filter(|r| r.kind.as_deref().is_none_or(|k| k == "user") && r.id.is_u64())
            .collect();
        if people.is_empty() {
            return Ok(format!("No matching people in {}.", course.label));
        }
        let mut out = format!("People you can message in {}:\n", course.label);
        for r in &people {
            out.push_str(&format!(
                "- {} (user id {})",
                inline(r.name.as_deref().unwrap_or("unknown")),
                r.id
            ));
            if let Some(roles) = r
                .common_courses
                .get(&course_id.to_string())
                .filter(|r| !r.is_empty())
            {
                let mut roles: Vec<String> = roles.iter().map(|role| role_name(role)).collect();
                roles.sort();
                roles.dedup();
                out.push_str(&format!(" · {}", roles.join(", ")));
            }
            out.push('\n');
        }
        out.push_str(&more_note(found.truncated, people.len()));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_must_be_plain_context_ids() {
        assert!(valid_filter("course_123") && valid_filter("user_5") && valid_filter("group_4"));
        assert!(
            !valid_filter("course_")
                && !valid_filter("account_1")
                && !valid_filter("course_1&scope=x")
        );
    }

    #[test]
    fn long_participant_lists_are_capped() {
        let names: Vec<String> = (1..=12).map(|i| format!("P{i}")).collect();
        assert_eq!(
            name_list(&names),
            "P1, P2, P3, P4, P5, P6, P7, P8 and 4 more"
        );
        assert_eq!(name_list(&names[..2]), "P1, P2");
    }

    #[test]
    fn enrollment_roles_read_naturally() {
        assert_eq!(role_name("TeacherEnrollment"), "teacher");
        assert_eq!(role_name("TaEnrollment"), "ta");
        assert_eq!(role_name("Grader"), "Grader");
    }
}
