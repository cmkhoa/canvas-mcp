//! Course content: files, discussions, modules, pages, quizzes.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Assignment, Author};

/// A Canvas file (course/group files, submission attachments).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct File {
    pub id: u64,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default, rename = "content-type")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Signed download link; may point at a different host than the API.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
    #[serde(default)]
    pub lock_explanation: Option<String>,
}

impl File {
    pub fn name(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.filename.as_deref())
            .unwrap_or("unnamed file")
    }
}

/// A discussion topic or announcement (`…/discussion_topics`, `/announcements`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscussionTopic {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    /// HTML body.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub posted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub delayed_post_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_reply_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub author: Option<Author>,
    /// `/announcements` only: e.g. `course_123`.
    #[serde(default)]
    pub context_code: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub discussion_subentry_count: Option<u64>,
    #[serde(default)]
    pub read_state: Option<String>,
    #[serde(default)]
    pub unread_count: Option<u64>,
    /// Graded discussions have one.
    #[serde(default)]
    pub assignment_id: Option<u64>,
    #[serde(default)]
    pub assignment: Option<Assignment>,
    #[serde(default)]
    pub locked: Option<bool>,
    #[serde(default)]
    pub pinned: Option<bool>,
    /// Students must post before seeing others' posts.
    #[serde(default)]
    pub require_initial_post: Option<bool>,
    #[serde(default)]
    pub group_category_id: Option<u64>,
    /// In a group space: the course-level topic this is a copy of.
    #[serde(default)]
    pub root_topic_id: Option<u64>,
    #[serde(default)]
    pub todo_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub discussion_type: Option<String>,
}

impl DiscussionTopic {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("Untitled")
    }

    pub fn author_name(&self) -> Option<&str> {
        self.author
            .as_ref()
            .and_then(|a| a.display_name.as_deref())
            .or(self.user_name.as_deref())
    }
}

/// A post in a discussion (`…/entries`, `…/replies`, `…/entry_list`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiscussionEntry {
    pub id: u64,
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub parent_id: Option<u64>,
    #[serde(default)]
    pub read_state: Option<String>,
    #[serde(default)]
    pub deleted: Option<bool>,
    /// The newest few replies Canvas embeds on top-level entries.
    #[serde(default)]
    pub recent_replies: Vec<DiscussionEntry>,
    #[serde(default)]
    pub has_more_replies: Option<bool>,
}

/// `GET /courses/:id/modules`. `state` is the caller's own progress.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Module {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<u32>,
    #[serde(default)]
    pub unlock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub require_sequential_progress: Option<bool>,
    #[serde(default)]
    pub prerequisite_module_ids: Vec<u64>,
    /// `locked`, `unlocked`, `started` or `completed`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub items_count: Option<u32>,
    /// With `include[]=items` (Canvas omits them for very large modules).
    #[serde(default)]
    pub items: Option<Vec<ModuleItem>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleItem {
    pub id: u64,
    #[serde(default)]
    pub module_id: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    /// `File`, `Page`, `Discussion`, `Assignment`, `Quiz`, `SubHeader`, `ExternalUrl`, `ExternalTool`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub content_id: Option<u64>,
    #[serde(default)]
    pub page_url: Option<String>,
    #[serde(default)]
    pub external_url: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub indent: Option<u32>,
    #[serde(default)]
    pub completion_requirement: Option<CompletionRequirement>,
    /// With `include[]=content_details`.
    #[serde(default)]
    pub content_details: Option<ContentDetails>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CompletionRequirement {
    /// `must_view`, `must_submit`, `must_contribute`, `min_score`, `must_mark_done`.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub min_score: Option<f64>,
    #[serde(default)]
    pub completed: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ContentDetails {
    #[serde(default)]
    pub points_possible: Option<f64>,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unlock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub lock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
    #[serde(default)]
    pub lock_explanation: Option<String>,
}

/// A wiki page (`/courses/:id/pages`, `/front_page`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Page {
    #[serde(default)]
    pub page_id: Option<u64>,
    /// The slug used in page URLs.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Only on single-page reads.
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub front_page: Option<bool>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
    #[serde(default)]
    pub lock_explanation: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub last_edited_by: Option<Author>,
    #[serde(default)]
    pub todo_date: Option<DateTime<Utc>>,
}

impl Page {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("Untitled page")
    }
}

/// A Classic Quiz (`/courses/:id/quizzes`).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Quiz {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    /// `assignment` (graded), `practice_quiz`, `graded_survey`, `survey`.
    #[serde(default)]
    pub quiz_type: Option<String>,
    #[serde(default)]
    pub assignment_id: Option<u64>,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub unlock_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub lock_at: Option<DateTime<Utc>>,
    /// Minutes.
    #[serde(default)]
    pub time_limit: Option<u32>,
    /// -1 means unlimited.
    #[serde(default)]
    pub allowed_attempts: Option<i64>,
    #[serde(default)]
    pub points_possible: Option<f64>,
    #[serde(default)]
    pub question_count: Option<u32>,
    /// `keep_highest`, `keep_latest` or `keep_average`.
    #[serde(default)]
    pub scoring_policy: Option<String>,
    #[serde(default)]
    pub one_question_at_a_time: Option<bool>,
    #[serde(default)]
    pub cant_go_back: Option<bool>,
    #[serde(default)]
    pub has_access_code: Option<bool>,
    #[serde(default)]
    pub locked_for_user: Option<bool>,
    #[serde(default)]
    pub lock_explanation: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    /// HTML instructions.
    #[serde(default)]
    pub description: Option<String>,
}

/// `GET /courses/:id/quizzes/:id/submission` wraps these in `quiz_submissions`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuizSubmissions {
    #[serde(default)]
    pub quiz_submissions: Vec<QuizSubmission>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QuizSubmission {
    #[serde(default)]
    pub attempt: Option<u32>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub kept_score: Option<f64>,
    /// `untaken`, `pending_review`, `complete`, `settings_only`, `preview`.
    #[serde(default)]
    pub workflow_state: Option<String>,
    #[serde(default)]
    pub attempts_left: Option<i64>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    /// When an in-progress attempt times out.
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
}
