use std::collections::BTreeMap;

use futures::future::join_all;
use rmcp::{handler::server::wrapper::Parameters, schemars, tool, tool_router};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err};
use crate::{
    canvas::models::{Assignment, Course, PeerReview, PlannerItem, Profile},
    untrusted::inline,
};

const MAX_ASSIGNMENTS: usize = 300;
const MAX_REVIEWS: usize = 500;
const MAX_PLANNER_ITEMS: usize = 500;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PeerReviewParams {
    /// Limit to one course (code, name, ID or sis_course_id:<id>). Omit for all active courses.
    #[serde(default)]
    pub course_identifier: Option<String>,
    /// Limit to one assignment (needs course_identifier).
    #[serde(default)]
    pub assignment_identifier: Option<u64>,
}

/// A pending review, keyed by Canvas's assessment-request ID.
struct Pending {
    course: String,
    assignment: String,
    assignment_id: Option<u64>,
    reviewee: Option<String>,
    link: Option<String>,
}

/// The assignment ID in a planner link like `/courses/5/assignments/66/submissions/7`.
fn assignment_id_from_link(link: &str) -> Option<u64> {
    let mut parts = link.split('/');
    parts.find(|p| *p == "assignments")?;
    parts.next()?.parse().ok()
}

impl CanvasServer {
    /// Pending reviews the caller owes on one assignment, via the assignment's peer_reviews.
    async fn assignment_reviews(
        &self,
        course_path: &str,
        a: &Assignment,
        me: u64,
    ) -> Result<Vec<(u64, Pending)>, String> {
        let reviews = self
            .client()
            .get_all::<PeerReview>(
                &format!("courses/{course_path}/assignments/{}/peer_reviews", a.id),
                &[("include[]", "user")],
                MAX_REVIEWS,
            )
            .await
            .map_err(err)?;
        Ok(reviews
            .items
            .into_iter()
            .filter(|r| {
                r.assessor_id == Some(me) && r.workflow_state.as_deref() != Some("completed")
            })
            .map(|r| {
                (
                    r.id,
                    Pending {
                        course: String::new(),
                        assignment: a.title().to_owned(),
                        assignment_id: Some(a.id),
                        reviewee: r.user.and_then(|u| u.display_name),
                        link: a.html_url.clone(),
                    },
                )
            })
            .collect())
    }
}

#[tool_router(router = peer_reviews_router, vis = "pub(super)")]
impl CanvasServer {
    /// Peer reviews you still need to complete, across courses or for one course/assignment.
    /// Combines Canvas's planner feed with each peer-reviewed assignment, and only says
    /// "nothing pending" when every source could be checked.
    #[tool(annotations(
        title = "Peer reviews to do",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn get_my_peer_reviews_todo(
        &self,
        Parameters(p): Parameters<PeerReviewParams>,
    ) -> ToolResult {
        let me: Profile = self.client().get("users/self", &[]).await.map_err(err)?;
        let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
        let mut unchecked: Vec<String> = Vec::new();

        if let Some(assignment_id) = p.assignment_identifier {
            let ident = p
                .course_identifier
                .as_deref()
                .ok_or("assignment_identifier needs course_identifier too.")?;
            let course = self.resolve(ident).await?;
            let a: Assignment = self
                .client()
                .get(
                    &format!("courses/{}/assignments/{assignment_id}", course.path_id),
                    &[],
                )
                .await
                .map_err(err)?;
            for (id, mut review) in self.assignment_reviews(&course.path_id, &a, me.id).await? {
                review.course = course.label.clone();
                pending.insert(id, review);
            }
        } else {
            let courses: Vec<Course> = match &p.course_identifier {
                Some(ident) => {
                    let course = self.resolve(ident).await?;
                    let id = self.numeric_course_id(&course).await?;
                    let cached = self.courses().active_courses().await.map_err(err)?;
                    match cached.iter().find(|c| c.id == id) {
                        Some(c) => vec![c.clone()],
                        None => vec![
                            self.client()
                                .get(&format!("courses/{id}"), &[])
                                .await
                                .map_err(err)?,
                        ],
                    }
                }
                None => self
                    .courses()
                    .active_courses()
                    .await
                    .map_err(err)?
                    .as_ref()
                    .clone(),
            };
            let labels: BTreeMap<u64, String> = courses.iter().map(|c| (c.id, c.label())).collect();

            // Source 1: the planner, which Canvas's own To Do list uses. Assessor IDs are often
            // absent there, so only an explicitly different assessor is skipped.
            let codes: Vec<String> = match &p.course_identifier {
                Some(_) => courses.iter().map(|c| format!("course_{}", c.id)).collect(),
                None => Vec::new(),
            };
            let mut query = vec![("filter", "incomplete_items")];
            query.extend(codes.iter().map(|c| ("context_codes[]", c.as_str())));
            match self
                .client()
                .get_all::<PlannerItem>("planner/items", &query, MAX_PLANNER_ITEMS)
                .await
            {
                Ok(items) => {
                    for item in items.items {
                        if item.plannable_type.as_deref() != Some("assessment_request") {
                            continue;
                        }
                        let Some(plannable) = item.plannable else {
                            continue;
                        };
                        if plannable.workflow_state.as_deref() == Some("completed")
                            || plannable.assessor_id.is_some_and(|a| a != me.id)
                        {
                            continue;
                        }
                        if p.course_identifier.is_some()
                            && item.course_id.is_none_or(|c| !labels.contains_key(&c))
                        {
                            continue;
                        }
                        let Some(id) = plannable.id.or(item.plannable_id) else {
                            continue;
                        };
                        pending.insert(
                            id,
                            Pending {
                                course: item
                                    .course_id
                                    .and_then(|c| labels.get(&c).cloned())
                                    .or(item.context_name)
                                    .unwrap_or_else(|| "unknown course".into()),
                                assignment: plannable
                                    .title
                                    .unwrap_or_else(|| "Untitled assignment".into()),
                                assignment_id: item
                                    .html_url
                                    .as_deref()
                                    .and_then(assignment_id_from_link),
                                reviewee: None,
                                link: item.html_url,
                            },
                        );
                    }
                }
                Err(e) => unchecked.push(format!("planner ({e})")),
            }

            // Source 2: every peer-reviewed assignment in each course.
            let per_course = self
                .fan_out(&courses, |c: &Course| {
                    let path = format!("courses/{}/assignments", c.id);
                    async move {
                        self.client()
                            .get_all::<Assignment>(&path, &[], MAX_ASSIGNMENTS)
                            .await
                    }
                })
                .await;
            for (course, e) in &per_course.failed {
                unchecked.push(format!("{} assignments ({e})", course.label()));
            }
            for (course, assignments) in &per_course.ok {
                let reviewed: Vec<&Assignment> = assignments
                    .items
                    .iter()
                    .filter(|a| a.peer_reviews == Some(true))
                    .collect();
                let path = course.id.to_string();
                let results = join_all(
                    reviewed
                        .iter()
                        .map(|a| self.assignment_reviews(&path, a, me.id)),
                )
                .await;
                for (a, result) in reviewed.iter().zip(results) {
                    match result {
                        Ok(found) => {
                            for (id, mut review) in found {
                                review.course = course.label();
                                // Prefer this richer record (it names the reviewee) over the planner's.
                                pending.insert(id, review);
                            }
                        }
                        Err(e) => unchecked.push(format!(
                            "{} in {} ({e})",
                            inline(a.title()),
                            course.label()
                        )),
                    }
                }
            }
        }

        let mut out = if pending.is_empty() {
            if unchecked.is_empty() {
                "No pending peer reviews.".to_owned()
            } else {
                "No pending peer reviews found, but some sources couldn't be checked, so there may be some.".to_owned()
            }
        } else {
            let mut out = format!("Peer reviews you need to complete ({}):\n", pending.len());
            for review in pending.values() {
                out.push_str(&format!(
                    "- {} · {}",
                    review.course,
                    inline(&review.assignment)
                ));
                if let Some(id) = review.assignment_id {
                    out.push_str(&format!(" (assignment {id})"));
                }
                match &review.reviewee {
                    Some(name) => out.push_str(&format!(" · reviewing {}", inline(name))),
                    None => out.push_str(" · reviewee not shown (anonymous or from the planner)"),
                }
                if let Some(link) = &review.link {
                    out.push_str(&format!(" · {link}"));
                }
                out.push('\n');
            }
            out
        };
        if !unchecked.is_empty() {
            out.push_str(&format!("\nCould not check: {}", unchecked.join("; ")));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_id_comes_from_planner_links() {
        assert_eq!(
            assignment_id_from_link("/courses/505/assignments/5066/submissions/2898"),
            Some(5066)
        );
        assert_eq!(
            assignment_id_from_link("/courses/505/discussion_topics/1"),
            None
        );
    }
}
