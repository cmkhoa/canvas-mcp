//! Page tools: list pages, read a page, page details, the front page.
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, CourseParams, ToolResult, hidden_tab, more_note};
use crate::{
    canvas::models::Page,
    error::CanvasError,
    format::{html_to_text, path_segment},
    resolve::ResolvedCourse,
    untrusted::{fence_untrusted, inline},
};

pub(super) const MAX_PAGES: usize = 300;
const PREVIEW_CHARS: usize = 300;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PageSort {
    #[default]
    Title,
    CreatedAt,
    UpdatedAt,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PagesParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Sort by title (default), created_at or updated_at.
    #[serde(default)]
    pub sort: PageSort,
    /// asc (default) or desc.
    #[serde(default)]
    pub order: SortOrder,
    /// Only pages whose title contains this text.
    #[serde(default)]
    pub search_term: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PageParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// The page's URL slug (e.g. "week-3-notes", from list_pages or a module item) or its numeric page ID.
    pub page_url_or_id: String,
}

impl CanvasServer {
    pub(super) fn page_header(&self, page: &Page) -> String {
        let mut line = inline(page.title());
        if let Some(slug) = &page.url {
            line.push_str(&format!(" (page {slug})"));
        }
        if page.front_page == Some(true) {
            line.push_str(" · front page");
        }
        line.push_str(&format!(
            " · updated {}",
            self.date(page.updated_at, "unknown")
        ));
        if let Some(editor) = page
            .last_edited_by
            .as_ref()
            .and_then(|a| a.display_name.as_deref())
        {
            line.push_str(&format!(" by {}", inline(editor)));
        }
        if page.locked_for_user == Some(true) {
            line.push_str(" · locked for you");
            if let Some(why) = &page.lock_explanation {
                line.push_str(&format!(" {}", inline(why)));
            }
        }
        line
    }

    async fn page(&self, course: &ResolvedCourse, url_or_id: &str) -> Result<Page, String> {
        let slug = url_or_id.trim();
        if slug.is_empty() {
            return Err("page_url_or_id is empty.".into());
        }
        self.client()
            .get(
                &format!("courses/{}/pages/{}", course.path_id, path_segment(slug)),
                &[],
            )
            .await
            .map_err(|e| hidden_tab(e, "Pages"))
    }

    fn page_body(&self, page: &Page, max_chars: usize) -> String {
        let text = html_to_text(page.body.as_deref().unwrap_or(""), max_chars);
        fence_untrusted(
            if text.is_empty() {
                "(empty page)"
            } else {
                &text
            },
            &format!("page: {}", page.title()),
        )
    }
}

#[tool_router(router = pages_router, vis = "pub(super)")]
impl CanvasServer {
    /// List a course's pages (title, slug, last update). Read one with get_page_content.
    #[tool(annotations(title = "Pages", read_only_hint = true, open_world_hint = true))]
    pub async fn list_pages(&self, Parameters(p): Parameters<PagesParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let sort = match p.sort {
            PageSort::Title => "title",
            PageSort::CreatedAt => "created_at",
            PageSort::UpdatedAt => "updated_at",
        };
        let order = match p.order {
            SortOrder::Asc => "asc",
            SortOrder::Desc => "desc",
        };
        let mut query = vec![("sort", sort), ("order", order)];
        if let Some(term) = &p.search_term {
            query.push(("search_term", term));
        }
        let pages = self
            .client()
            .get_all::<Page>(
                &format!("courses/{}/pages", course.path_id),
                &query,
                MAX_PAGES,
            )
            .await
            .map_err(|e| hidden_tab(e, "Pages"))?;
        if pages.items.is_empty() {
            return Ok(format!("No pages found in {}.", course.label));
        }
        let mut out = format!("Pages in {}:\n", course.label);
        for page in &pages.items {
            out.push_str(&format!("- {}\n", self.page_header(page)));
        }
        out.push_str(&more_note(pages.truncated, pages.items.len()));
        Ok(out)
    }

    /// The full text of one course page.
    #[tool(annotations(title = "Page content", read_only_hint = true, open_world_hint = true))]
    pub async fn get_page_content(&self, Parameters(p): Parameters<PageParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let page = self.page(&course, &p.page_url_or_id).await?;
        Ok(format!(
            "{}\n{}",
            self.page_header(&page),
            self.page_body(&page, usize::MAX)
        ))
    }

    /// A page's details (title, slug, dates, editor, lock status, link) plus a short preview.
    #[tool(annotations(title = "Page details", read_only_hint = true, open_world_hint = true))]
    pub async fn get_page_details(&self, Parameters(p): Parameters<PageParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let page = self.page(&course, &p.page_url_or_id).await?;
        let mut out = format!("{}\n", self.page_header(&page));
        out.push_str(&format!(
            "Created: {}\n",
            self.date(page.created_at, "unknown")
        ));
        if page.todo_date.is_some() {
            out.push_str(&format!("To-do date: {}\n", self.date(page.todo_date, "")));
        }
        if let Some(url) = &page.html_url {
            out.push_str(&format!("Link: {url}\n"));
        }
        out.push_str("Preview (get_page_content has the full text):\n");
        out.push_str(&self.page_body(&page, PREVIEW_CHARS));
        Ok(out)
    }

    /// The course's front (home) page, in full.
    #[tool(annotations(title = "Front page", read_only_hint = true, open_world_hint = true))]
    pub async fn get_front_page(&self, Parameters(p): Parameters<CourseParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let page: Page = self
            .client()
            .get(&format!("courses/{}/front_page", course.path_id), &[])
            .await
            .map_err(|e| match e {
                CanvasError::NotFound(_) => format!(
                    "{} has no front page; its home page may be modules, the syllabus or an activity stream.",
                    course.label
                ),
                other => hidden_tab(other, "Pages"),
            })?;
        Ok(format!(
            "{}\n{}",
            self.page_header(&page),
            self.page_body(&page, usize::MAX)
        ))
    }
}
