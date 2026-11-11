//! Deadline tools: what's due soon and the Canvas to-do list.
use chrono::{Duration, Utc};
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err, more_note};
use crate::{
    canvas::models::{PlannerItem, TodoItem},
    format::points,
    untrusted::inline,
};

const MAX_PLANNER_ITEMS: usize = 500;
const MAX_TODO_ITEMS: usize = 200;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpcomingParams {
    /// How many days ahead to look, from now. Default 7, at most 365.
    #[serde(default = "default_days")]
    pub days: u32,
}

fn default_days() -> u32 {
    7
}

#[tool_router(router = deadlines_router, vis = "pub(super)")]
impl CanvasServer {
    /// Your assignments, quizzes and graded discussions due in the next N days (default 7)
    /// across all courses, soonest first, with whether you've submitted each.
    #[tool(annotations(
        title = "Upcoming assignments",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_upcoming_assignments(
        &self,
        Parameters(p): Parameters<UpcomingParams>,
    ) -> ToolResult {
        if !(1..=365).contains(&p.days) {
            return Err("days must be between 1 and 365.".into());
        }
        // The planner honors an explicit window (upcoming_events is fixed at 7 days) and
        // carries submission state, saving a request per assignment.
        let start = Utc::now();
        let end = start + Duration::days(i64::from(p.days));
        let (start_s, end_s) = (start.to_rfc3339(), end.to_rfc3339());
        let items = self
            .client()
            .get_all::<PlannerItem>(
                "planner/items",
                &[("start_date", &start_s), ("end_date", &end_s)],
                MAX_PLANNER_ITEMS,
            )
            .await
            .map_err(err)?;
        let labels = self.course_labels().await.unwrap_or_default();

        let mut due: Vec<_> = items
            .items
            .iter()
            .filter_map(|item| {
                let plannable = item.plannable.as_ref()?;
                let kind = item.plannable_type.as_deref()?;
                let due_at = match kind {
                    "assignment" | "quiz" => plannable.due_at.or(item.plannable_date),
                    // Only graded discussions carry due_at; ungraded ones have todo_date.
                    "discussion_topic" => plannable.due_at,
                    _ => None,
                }?;
                (due_at <= end).then_some((due_at, kind, plannable, item))
            })
            .collect();
        due.sort_by_key(|(at, ..)| *at);
        if due.is_empty() {
            return Ok(format!("Nothing due in the next {} days.", p.days));
        }

        let mut out = format!("Due in the next {} days:\n", p.days);
        for (due_at, kind, plannable, item) in due {
            let course = item
                .course_id
                .and_then(|id| labels.get(&id).cloned())
                .or_else(|| item.context_name.clone())
                .unwrap_or_else(|| "unknown course".into());
            let mut line = format!(
                "- {} · {} · {}",
                self.date(Some(due_at), ""),
                course,
                inline(plannable.title.as_deref().unwrap_or("Untitled"))
            );
            if kind != "assignment" {
                line.push_str(&format!(" ({})", kind.replace('_', " ")));
            }
            if let Some(pts) = plannable.points_possible {
                line.push_str(&format!(" · {} pts", points(pts)));
            }
            let state = match &item.submissions {
                Some(s) if s.excused => "excused",
                Some(s) if s.missing => "MISSING",
                Some(s) if s.submitted && s.late => "submitted late",
                Some(s) if s.submitted => "submitted",
                Some(s) if s.graded => "graded",
                Some(_) => "not submitted",
                None => "no submission needed",
            };
            line.push_str(&format!(" · {state}"));
            if let Some(id) = plannable.id.or(item.plannable_id) {
                line.push_str(&format!(" · id {id}"));
            }
            out.push_str(&line);
            out.push('\n');
        }
        out.push_str(&more_note(items.truncated, items.items.len()));
        Ok(out)
    }

    /// Your Canvas to-do list: work Canvas thinks you still need to submit.
    #[tool(annotations(title = "To-do list", read_only_hint = true, open_world_hint = true))]
    pub async fn get_my_todo_items(&self) -> ToolResult {
        let todos = self
            .client()
            .get_all::<TodoItem>("users/self/todo", &[], MAX_TODO_ITEMS)
            .await
            .map_err(err)?;
        if todos.items.is_empty() {
            return Ok("Your Canvas to-do list is empty.".into());
        }
        let labels = self.course_labels().await.unwrap_or_default();
        // Canvas mixes "submitting" (your own work) with "grading" (courses you teach or TA).
        let (grading, submitting): (Vec<&TodoItem>, Vec<&TodoItem>) = todos
            .items
            .iter()
            .partition(|t| t.kind.as_deref() == Some("grading"));
        let mut out = String::from("Your Canvas to-do list:\n");
        for (title, items) in [
            ("To submit", submitting),
            ("To grade (courses you teach or TA)", grading),
        ] {
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("\n{title} ({}):\n", items.len()));
            for t in items {
                let course = t
                    .course_id
                    .and_then(|id| labels.get(&id).cloned())
                    .or_else(|| t.context_name.clone())
                    .unwrap_or_else(|| "unknown course".into());
                match &t.assignment {
                    Some(a) => out.push_str(&self.assignment_line(&course, a)),
                    None => out.push_str(&format!(
                        "- {course} · {}",
                        t.kind.as_deref().unwrap_or("item")
                    )),
                }
                if let Some(n) = t.needs_grading_count.filter(|n| *n > 0) {
                    out.push_str(&format!(" · {n} to grade"));
                }
                out.push('\n');
            }
        }
        out.push_str(&more_note(todos.truncated, todos.items.len()));
        Ok(out)
    }
}
