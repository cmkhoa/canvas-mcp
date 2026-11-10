use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{
    CanvasServer, OptionalCourseParams, ToolResult, err,
    files::{FileOrder, FileSort},
    more_note,
};
use crate::{
    canvas::models::{File, Group, User},
    untrusted::inline,
};

const MAX_MEMBERS: usize = 300;
const MAX_FILES: usize = 300;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GroupParams {
    /// Canvas group ID, from list_my_groups.
    pub group_id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GroupFilesParams {
    /// Canvas group ID, from list_my_groups.
    pub group_id: u64,
    /// Only files whose name contains this text (at least 2 characters).
    #[serde(default)]
    pub search_term: Option<String>,
    /// Sort by name (default), size, created_at, updated_at or content_type.
    #[serde(default)]
    pub sort: FileSort,
    /// asc (default) or desc.
    #[serde(default)]
    pub order: FileOrder,
}

impl CanvasServer {
    /// One of the caller's own groups; anything else is refused before reading it.
    async fn my_group(&self, group_id: u64) -> Result<Group, String> {
        self.my_groups()
            .await?
            .into_iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| {
                format!(
                    "You are not a member of group {group_id}. list_my_groups shows your groups."
                )
            })
    }
}

#[tool_router(router = groups_router, vis = "pub(super)")]
impl CanvasServer {
    /// The groups you belong to (project teams, study groups), with course, group set
    /// (category) ID and member count. Read a group's discussions with list_discussion_topics
    /// or get_discussion_with_replies using the course and group_id.
    #[tool(annotations(title = "My groups", read_only_hint = true, open_world_hint = true))]
    pub async fn list_my_groups(
        &self,
        Parameters(p): Parameters<OptionalCourseParams>,
    ) -> ToolResult {
        let course_filter = match &p.course_identifier {
            Some(ident) => {
                let course = self.resolve(ident).await?;
                Some(self.numeric_course_id(&course).await?)
            }
            None => None,
        };
        let groups: Vec<Group> = self
            .my_groups()
            .await?
            .into_iter()
            .filter(|g| course_filter.is_none_or(|c| g.course_id == Some(c)))
            .collect();
        if groups.is_empty() {
            return Ok("You aren't in any groups.".into());
        }
        let labels = self.course_labels().await.unwrap_or_default();
        let mut out = String::from("Your groups:\n");
        for g in &groups {
            let mut line = format!(
                "- [group {}] {}",
                g.id,
                inline(g.name.as_deref().unwrap_or("Unnamed group"))
            );
            match g.course_id {
                Some(c) => line.push_str(&format!(
                    " · {} (course {c})",
                    labels.get(&c).map_or("course", String::as_str)
                )),
                None => line.push_str(&format!(
                    " · {} group",
                    g.context_type.as_deref().unwrap_or("non-course")
                )),
            }
            if let Some(set) = g.group_category_id {
                line.push_str(&format!(" · set {set}"));
            }
            if let Some(n) = g.members_count {
                line.push_str(&format!(" · {n} members"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        Ok(out)
    }

    /// The members of one of your groups: names and Canvas user IDs (never emails).
    #[tool(annotations(title = "Group members", read_only_hint = true, open_world_hint = true))]
    pub async fn get_group_members(&self, Parameters(p): Parameters<GroupParams>) -> ToolResult {
        let group = self.my_group(p.group_id).await?;
        let members = self
            .client()
            .get_all::<User>(&format!("groups/{}/users", group.id), &[], MAX_MEMBERS)
            .await
            .map_err(err)?;
        let mut out = format!(
            "Members of {} ({}):\n",
            inline(group.name.as_deref().unwrap_or("group")),
            members.items.len()
        );
        for u in &members.items {
            let name = u
                .name
                .as_deref()
                .or(u.short_name.as_deref())
                .unwrap_or("unknown");
            out.push_str(&format!("- {} (user id {})\n", inline(name), u.id));
        }
        out.push_str(&more_note(members.truncated, members.items.len()));
        Ok(out)
    }

    /// The files stored in one of your groups.
    #[tool(annotations(title = "Group files", read_only_hint = true, open_world_hint = true))]
    pub async fn list_group_files(
        &self,
        Parameters(p): Parameters<GroupFilesParams>,
    ) -> ToolResult {
        let group = self.my_group(p.group_id).await?;
        if p.search_term
            .as_ref()
            .is_some_and(|t| t.trim().chars().count() < 2)
        {
            return Err("search_term needs at least 2 characters.".into());
        }
        let mut query = vec![("sort", p.sort.as_param()), ("order", p.order.as_param())];
        if let Some(term) = &p.search_term {
            query.push(("search_term", term.trim()));
        }
        let files = self
            .client()
            .get_all::<File>(&format!("groups/{}/files", group.id), &query, MAX_FILES)
            .await
            .map_err(err)?;
        let name = inline(group.name.as_deref().unwrap_or("group"));
        if files.items.is_empty() {
            return Ok(format!("No files in {name}."));
        }
        let mut out = format!("Files in {name}:\n");
        for f in &files.items {
            out.push_str(&self.file_line(f));
            out.push('\n');
        }
        out.push_str(&more_note(files.truncated, files.items.len()));
        out.push_str("\n(Read group files in Canvas; read_course_file covers course files only.)");
        Ok(out)
    }
}
