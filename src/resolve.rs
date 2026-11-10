//! Turning whatever the model calls a course ("cs101", "Organic Chem", "12345") into a Canvas
//! path ID. Unmatched names are refused with suggestions, never forwarded to Canvas as a guess.
use std::{sync::Arc, time::Duration};

use thiserror::Error;

use crate::{
    cache::TtlCache,
    canvas::{CanvasClient, models::Course},
    error::CanvasError,
};

const COURSE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_COURSES: usize = 500;
const MAX_SUGGESTIONS: usize = 5;
const SIS_PREFIX: &str = "sis_course_id:";

/// A course ready to drop into a Canvas path (`courses/{path_id}/…`), plus a label for output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCourse {
    pub path_id: String,
    pub label: String,
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error(
        "No course given. Pass a course code (e.g. \"CS 101\"), part of the course name, or a numeric course ID."
    )]
    Empty,
    #[error("No active course matches {query:?}. {}", suggestion_text(.suggestions))]
    NotFound {
        query: String,
        suggestions: Vec<String>,
    },
    #[error(
        "{query:?} matches more than one course: {}. Use the course ID or full course code.",
        .matches.join("; ")
    )]
    Ambiguous { query: String, matches: Vec<String> },
    #[error(transparent)]
    Canvas(#[from] CanvasError),
}

fn suggestion_text(suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        "You have no active courses; older courses need their numeric course ID.".into()
    } else {
        format!(
            "Did you mean: {}? Older courses need their numeric course ID.",
            suggestions.join("; ")
        )
    }
}

/// Resolves course identifiers against the user's active courses, cached for a few minutes.
#[derive(Debug)]
pub struct CourseDirectory {
    client: Arc<CanvasClient>,
    cache: TtlCache<Vec<Course>>,
}

impl CourseDirectory {
    pub fn new(client: Arc<CanvasClient>) -> Self {
        Self {
            client,
            cache: TtlCache::new(COURSE_CACHE_TTL),
        }
    }

    /// The user's active courses, with terms.
    pub async fn active_courses(&self) -> Result<Arc<Vec<Course>>, CanvasError> {
        self.cache
            .get_or_try_refresh(|| async {
                let paged = self
                    .client
                    .get_all::<Course>(
                        "courses",
                        &[("enrollment_state", "active"), ("include[]", "term")],
                        MAX_COURSES,
                    )
                    .await?;
                Ok(paged.items)
            })
            .await
    }

    /// Accepts a numeric course ID or `sis_course_id:…` as-is (no lookup), otherwise matches a
    /// course code or name among active courses, ignoring case, spaces and punctuation.
    pub async fn resolve(&self, query: &str) -> Result<ResolvedCourse, ResolveError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ResolveError::Empty);
        }
        if let Some(direct) = resolve_direct(query) {
            return Ok(direct);
        }
        match_course(&self.active_courses().await?, query)
    }
}

fn resolve_direct(query: &str) -> Option<ResolvedCourse> {
    if query.bytes().all(|b| b.is_ascii_digit()) {
        return Some(ResolvedCourse {
            path_id: query.to_owned(),
            label: format!("course {query}"),
        });
    }
    let sis = query.strip_prefix(SIS_PREFIX)?.trim();
    (!sis.is_empty()).then(|| ResolvedCourse {
        path_id: format!("{SIS_PREFIX}{}", encode_path_segment(sis)),
        label: sis.to_owned(),
    })
}

/// Matches `query` against course codes, names and SIS IDs: exact first, then substring.
pub fn match_course(courses: &[Course], query: &str) -> Result<ResolvedCourse, ResolveError> {
    let wanted = normalize(query);
    if wanted.is_empty() {
        return Err(ResolveError::Empty);
    }
    let keyed: Vec<(&Course, Vec<String>)> = courses
        .iter()
        .map(|c| (c, match_keys(c)))
        .filter(|(_, keys)| !keys.is_empty())
        .collect();

    let exact: Vec<&Course> = keyed
        .iter()
        .filter(|(_, keys)| keys.contains(&wanted))
        .map(|(c, _)| *c)
        .collect();
    let candidates = if exact.is_empty() {
        keyed
            .iter()
            .filter(|(_, keys)| keys.iter().any(|k| k.contains(&wanted)))
            .map(|(c, _)| *c)
            .collect()
    } else {
        exact
    };

    match candidates.as_slice() {
        [course] => Ok(ResolvedCourse {
            path_id: course.id.to_string(),
            label: course.label(),
        }),
        [] => Err(ResolveError::NotFound {
            query: query.to_owned(),
            suggestions: suggestions(&keyed, &wanted),
        }),
        many => Err(ResolveError::Ambiguous {
            query: query.to_owned(),
            matches: many.iter().map(|c| describe(c)).collect(),
        }),
    }
}

fn match_keys(course: &Course) -> Vec<String> {
    [&course.course_code, &course.name, &course.sis_course_id]
        .into_iter()
        .flatten()
        .map(|s| normalize(s))
        .filter(|k| !k.is_empty())
        .collect()
}

fn suggestions(keyed: &[(&Course, Vec<String>)], wanted: &str) -> Vec<String> {
    let mut scored: Vec<(f64, &Course)> = keyed
        .iter()
        .map(|(c, keys)| {
            let best = keys
                .iter()
                .map(|k| strsim::normalized_levenshtein(wanted, k))
                .fold(0.0, f64::max);
            (best, *c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, c)| describe(c))
        .collect()
}

fn describe(course: &Course) -> String {
    match (&course.course_code, &course.name) {
        (Some(code), Some(name)) if code != name => format!("{code} — {name} (id {})", course.id),
        _ => format!("{} (id {})", course.label(), course.id),
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Percent-encodes everything except RFC 3986 unreserved characters.
fn encode_path_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn courses() -> Vec<Course> {
        serde_json::from_value(json!([
            { "id": 101, "course_code": "CS 101", "name": "Introduction to Computer Science" },
            { "id": 210, "course_code": "CHEM 210", "name": "Organic Chemistry I" },
            { "id": 150, "course_code": "MATH 150", "name": "Introduction to Statistics" },
            { "id": 330, "course_code": "HIST 330", "name": "Modern Europe", "sis_course_id": "2026FA-HIST-330" },
            { "id": 999, "access_restricted_by_date": true }
        ]))
        .unwrap()
    }

    fn resolved(query: &str) -> ResolvedCourse {
        match_course(&courses(), query).unwrap()
    }

    #[test]
    fn matches_codes_ignoring_case_spacing_and_punctuation() {
        assert_eq!(resolved("cs101").path_id, "101");
        assert_eq!(resolved("Chem-210").path_id, "210");
        assert_eq!(resolved("CS 101").label, "CS 101");
    }

    #[test]
    fn matches_partial_names_and_sis_ids() {
        assert_eq!(resolved("organic chem").path_id, "210");
        assert_eq!(resolved("2026FA-HIST-330").path_id, "330");
    }

    #[test]
    fn exact_match_beats_substring() {
        let mut list = courses();
        list.push(serde_json::from_value(json!({ "id": 102, "course_code": "CS 1010" })).unwrap());
        assert_eq!(match_course(&list, "CS 101").unwrap().path_id, "101");
    }

    #[test]
    fn ambiguous_query_lists_the_choices() {
        let err = match_course(&courses(), "introduction").unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, ResolveError::Ambiguous { .. }));
        assert!(
            msg.contains("CS 101 — Introduction to Computer Science (id 101)"),
            "{msg}"
        );
        assert!(msg.contains("MATH 150"), "{msg}");
    }

    #[test]
    fn unknown_query_is_refused_with_ranked_suggestions() {
        let err = match_course(&courses(), "CHM 201").unwrap_err();
        let ResolveError::NotFound { suggestions, .. } = &err else {
            panic!("{err}")
        };
        assert!(suggestions[0].starts_with("CHEM 210"), "{suggestions:?}");
        assert!(suggestions.len() <= MAX_SUGGESTIONS);
        assert!(err.to_string().contains("Did you mean"));
    }

    #[test]
    fn restricted_courses_never_match_by_name() {
        assert!(match_course(&courses(), "course 999").is_err());
    }

    #[test]
    fn ids_and_sis_ids_resolve_without_a_lookup() {
        assert_eq!(resolve_direct("12345").unwrap().path_id, "12345");
        let sis = resolve_direct("sis_course_id:2026 FA/BIO 1").unwrap();
        assert_eq!(sis.path_id, "sis_course_id:2026%20FA%2FBIO%201");
        assert_eq!(sis.label, "2026 FA/BIO 1");
        assert!(resolve_direct("sis_course_id: ").is_none());
        assert!(resolve_direct("cs101").is_none());
    }

    #[test]
    fn punctuation_only_query_is_empty() {
        assert!(matches!(
            match_course(&courses(), " - "),
            Err(ResolveError::Empty)
        ));
    }
}
