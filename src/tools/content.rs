use futures::future::join_all;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, CourseParams, ToolResult, err, more_note};
use crate::{
    canvas::models::{Course, Module, ModuleItem, Page},
    error::CanvasError,
    format::{html_to_text, path_segment, points},
    resolve::ResolvedCourse,
    untrusted::{fence_untrusted, inline},
};

const MAX_MODULES: usize = 200;
const MAX_ITEMS: usize = 500;
const MAX_PAGES: usize = 300;
const PREVIEW_CHARS: usize = 300;
const OVERVIEW_SYLLABUS_CHARS: usize = 600;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ModulesParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Also list each module's items (default false).
    #[serde(default)]
    pub include_items: bool,
    /// Only modules whose name contains this text.
    #[serde(default)]
    pub search_term: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ModuleItemsParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Module ID (from list_modules).
    pub module_id: u64,
    /// Include due dates, points and lock status for each item (default true).
    #[serde(default = "default_true")]
    pub include_content_details: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OverviewParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Include the list of pages (default true).
    #[serde(default = "default_true")]
    pub include_pages: bool,
    /// Include modules and their items (default true).
    #[serde(default = "default_true")]
    pub include_modules: bool,
    /// Include a syllabus preview (default true).
    #[serde(default = "default_true")]
    pub include_syllabus: bool,
}

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

fn default_true() -> bool {
    true
}

/// Explains Canvas's 401/404 for a tab the instructor hid from students.
fn hidden_tab(e: CanvasError, tab: &str) -> String {
    match e {
        CanvasError::Unauthorized | CanvasError::NotFound(_) | CanvasError::Forbidden(_) => {
            format!(
                "{e}\nThe {tab} tab may be hidden from students in this course; try list_modules instead."
            )
        }
        other => other.to_string(),
    }
}

impl CanvasServer {
    fn module_line(&self, m: &Module) -> String {
        let mut line = format!(
            "[module {}] {}",
            m.id,
            inline(m.name.as_deref().unwrap_or("Untitled module"))
        );
        if let Some(state) = &m.state {
            line.push_str(&format!(" · {state}"));
        }
        if let Some(n) = m.items_count {
            line.push_str(&format!(" · {n} items"));
        }
        if m.unlock_at.is_some() {
            line.push_str(&format!(" · unlocks {}", self.date(m.unlock_at, "")));
        }
        if m.require_sequential_progress == Some(true) {
            line.push_str(" · complete items in order");
        }
        if !m.prerequisite_module_ids.is_empty() {
            let ids: Vec<String> = m
                .prerequisite_module_ids
                .iter()
                .map(u64::to_string)
                .collect();
            line.push_str(&format!(" · requires module {}", ids.join(", ")));
        }
        line
    }

    fn item_line(&self, item: &ModuleItem) -> String {
        let kind = item.kind.as_deref().unwrap_or("Item");
        let pad = "  ".repeat(item.indent.unwrap_or(0) as usize + 1);
        if kind == "SubHeader" {
            return format!("{pad}— {} —", inline(item.title.as_deref().unwrap_or("")));
        }
        let mut line = format!(
            "{pad}- {kind}: {}",
            inline(item.title.as_deref().unwrap_or("Untitled"))
        );
        match (kind, item.content_id, &item.page_url) {
            ("Page", _, Some(slug)) => line.push_str(&format!(" (page {slug})")),
            ("ExternalUrl", ..) => {
                if let Some(url) = &item.external_url {
                    line.push_str(&format!(" ({url})"));
                }
            }
            (_, Some(id), _) => line.push_str(&format!(" ({} id {id})", kind.to_lowercase())),
            _ => {}
        }
        line.push_str(&format!(" [item {}]", item.id));
        if let Some(d) = &item.content_details {
            if let Some(p) = d.points_possible {
                line.push_str(&format!(" · {} pts", points(p)));
            }
            if d.due_at.is_some() {
                line.push_str(&format!(" · due {}", self.date(d.due_at, "")));
            }
            if d.locked_for_user == Some(true) {
                line.push_str(" · locked");
                if d.unlock_at.is_some() {
                    line.push_str(&format!(" until {}", self.date(d.unlock_at, "")));
                }
            }
        }
        if let Some(req) = &item.completion_requirement {
            let what = match req.kind.as_deref() {
                Some("must_view") => "view".to_owned(),
                Some("must_submit") => "submit".to_owned(),
                Some("must_contribute") => "contribute".to_owned(),
                Some("must_mark_done") => "mark done".to_owned(),
                Some("min_score") => format!(
                    "score at least {}",
                    req.min_score.map_or("?".into(), points)
                ),
                Some(other) => other.replace('_', " "),
                None => "complete".to_owned(),
            };
            let done = if req.completed == Some(true) {
                "done"
            } else {
                "not done"
            };
            line.push_str(&format!(" · to complete: {what} ({done})"));
        }
        line
    }

    async fn modules(
        &self,
        course: &ResolvedCourse,
        with_items: bool,
        search: Option<&str>,
    ) -> Result<(Vec<Module>, bool), CanvasError> {
        let mut query = Vec::new();
        if with_items {
            query.extend([("include[]", "items"), ("include[]", "content_details")]);
        }
        if let Some(term) = search {
            query.push(("search_term", term));
        }
        let mut paged = self
            .client()
            .get_all::<Module>(
                &format!("courses/{}/modules", course.path_id),
                &query,
                MAX_MODULES,
            )
            .await?;
        if with_items {
            // Canvas leaves `items` out of very large modules; fetch those separately.
            let missing: Vec<usize> = paged
                .items
                .iter()
                .enumerate()
                .filter(|(_, m)| m.items.is_none())
                .map(|(i, _)| i)
                .collect();
            let fetched = join_all(
                missing
                    .iter()
                    .map(|&i| self.module_items(course, paged.items[i].id, true)),
            )
            .await;
            for (i, result) in missing.into_iter().zip(fetched) {
                if let Ok((items, _)) = result {
                    paged.items[i].items = Some(items);
                }
            }
        }
        Ok((paged.items, paged.truncated))
    }

    async fn module_items(
        &self,
        course: &ResolvedCourse,
        module_id: u64,
        details: bool,
    ) -> Result<(Vec<ModuleItem>, bool), CanvasError> {
        let query: &[(&str, &str)] = if details {
            &[("include[]", "content_details")]
        } else {
            &[]
        };
        let paged = self
            .client()
            .get_all::<ModuleItem>(
                &format!("courses/{}/modules/{module_id}/items", course.path_id),
                query,
                MAX_ITEMS,
            )
            .await?;
        Ok((paged.items, paged.truncated))
    }

    fn render_modules(&self, modules: &[Module]) -> String {
        let mut out = String::new();
        for m in modules {
            out.push_str(&self.module_line(m));
            out.push('\n');
            for item in m.items.iter().flatten() {
                out.push_str(&self.item_line(item));
                out.push('\n');
            }
        }
        out
    }

    fn page_header(&self, page: &Page) -> String {
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

#[tool_router(router = content_router, vis = "pub(super)")]
impl CanvasServer {
    /// List a course's modules in order, with your progress (locked/unlocked/started/completed),
    /// prerequisites and, with include_items, each module's items.
    #[tool(annotations(title = "Modules", read_only_hint = true, open_world_hint = true))]
    pub async fn list_modules(&self, Parameters(p): Parameters<ModulesParams>) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let (modules, truncated) = self
            .modules(&course, p.include_items, p.search_term.as_deref())
            .await
            .map_err(|e| hidden_tab(e, "Modules"))?;
        if modules.is_empty() {
            return Ok(format!("{} has no modules visible to you.", course.label));
        }
        Ok(format!(
            "Modules in {}:\n{}{}",
            course.label,
            self.render_modules(&modules),
            more_note(truncated, modules.len())
        ))
    }

    /// List the items in one module: pages, assignments, quizzes, files, links, with due
    /// dates, points, lock status and what you must do to complete each.
    #[tool(annotations(title = "Module items", read_only_hint = true, open_world_hint = true))]
    pub async fn list_module_items(
        &self,
        Parameters(p): Parameters<ModuleItemsParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let module: Module = self
            .client()
            .get(
                &format!("courses/{}/modules/{}", course.path_id, p.module_id),
                &[],
            )
            .await
            .map_err(err)?;
        let (items, truncated) = self
            .module_items(&course, p.module_id, p.include_content_details)
            .await
            .map_err(err)?;
        let mut out = format!("{} in {}:\n", self.module_line(&module), course.label);
        if items.is_empty() {
            out.push_str("  (no items)\n");
        }
        for item in &items {
            out.push_str(&self.item_line(item));
            out.push('\n');
        }
        out.push_str(&more_note(truncated, items.len()));
        Ok(out)
    }

    /// The whole module → item tree of a course in one call, with due dates and completion
    /// requirements. Good for "what's in this course" or planning study order.
    #[tool(annotations(
        title = "Course structure",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_course_structure(
        &self,
        Parameters(p): Parameters<CourseParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let (modules, truncated) = self
            .modules(&course, true, None)
            .await
            .map_err(|e| hidden_tab(e, "Modules"))?;
        if modules.is_empty() {
            return Ok(format!("{} has no modules visible to you.", course.label));
        }
        let items: usize = modules
            .iter()
            .map(|m| m.items.as_ref().map_or(0, Vec::len))
            .sum();
        Ok(format!(
            "Structure of {} ({} modules, {items} items):\n{}{}",
            course.label,
            modules.len(),
            self.render_modules(&modules),
            more_note(truncated, modules.len())
        ))
    }

    /// An overview of a course's content: syllabus preview, pages and modules with their
    /// items, in one call. Sections the course hides are reported, not fatal.
    #[tool(annotations(
        title = "Course content overview",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_course_content_overview(
        &self,
        Parameters(p): Parameters<OverviewParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let mut out = format!("Content overview for {}\n", course.label);

        if p.include_syllabus {
            let syllabus = self
                .client()
                .get::<Course>(
                    &format!("courses/{}", course.path_id),
                    &[("include[]", "syllabus_body")],
                )
                .await;
            out.push_str("\n== Syllabus\n");
            match syllabus.map(|c| c.syllabus_body.unwrap_or_default()) {
                Ok(html) if html.trim().is_empty() => out.push_str("(empty)\n"),
                Ok(html) => {
                    out.push_str(&fence_untrusted(
                        &html_to_text(&html, OVERVIEW_SYLLABUS_CHARS),
                        &format!("syllabus: {}", course.label),
                    ));
                    out.push_str("\n(get_syllabus has the full text)\n");
                }
                Err(e) => out.push_str(&format!("(unavailable: {e})\n")),
            }
        }
        if p.include_pages {
            out.push_str("\n== Pages\n");
            match self
                .client()
                .get_all::<Page>(
                    &format!("courses/{}/pages", course.path_id),
                    &[("sort", "title")],
                    MAX_PAGES,
                )
                .await
            {
                Ok(pages) if pages.items.is_empty() => out.push_str("(none)\n"),
                Ok(pages) => {
                    for page in &pages.items {
                        out.push_str(&format!("- {}\n", self.page_header(page)));
                    }
                    out.push_str(&more_note(pages.truncated, pages.items.len()));
                }
                Err(e) => out.push_str(&format!("(unavailable: {})\n", hidden_tab(e, "Pages"))),
            }
        }
        if p.include_modules {
            out.push_str("\n== Modules\n");
            match self.modules(&course, true, None).await {
                Ok((modules, _)) if modules.is_empty() => out.push_str("(none)\n"),
                Ok((modules, truncated)) => {
                    out.push_str(&self.render_modules(&modules));
                    out.push_str(&more_note(truncated, modules.len()));
                }
                Err(e) => out.push_str(&format!("(unavailable: {e})\n")),
            }
        }
        Ok(out)
    }

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
