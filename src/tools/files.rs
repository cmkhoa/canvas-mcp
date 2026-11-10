use std::{
    io::Write,
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, ResourceContents},
    schemars, tool, tool_router,
};
use serde::Deserialize;

use super::{CanvasServer, ToolResult, err, hidden_tab, more_note};
use crate::{
    canvas::models::File,
    format::{html_to_text, truncate},
    resolve::ResolvedCourse,
    untrusted::{fence_untrusted, inline},
};

const MAX_FILES: usize = 300;
const DEFAULT_READ_MB: f64 = 25.0;
const MAX_READ_MB: f64 = 50.0;
const MAX_DOWNLOAD_BYTES: u64 = 500 * 1_048_576;
/// Text files beyond this many characters are cut (and the cut is stated).
const MAX_TEXT_CHARS: usize = 200_000;

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileSort {
    #[default]
    Name,
    Size,
    CreatedAt,
    UpdatedAt,
    ContentType,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FileOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListFilesParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadFileParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Canvas file ID (from list_course_files, a module item, or a submission).
    pub file_id: u64,
    /// Largest file to read, in MB (default 25, at most 50).
    #[serde(default = "default_read_mb")]
    pub max_size_mb: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DownloadFileParams {
    /// Course code (e.g. "CS 101"), part of the course name, numeric Canvas course ID, or sis_course_id:<id>.
    pub course_identifier: String,
    /// Canvas file ID (from list_course_files, a module item, or a submission).
    pub file_id: u64,
    /// Existing local folder to save into. Default: ~/Downloads (or the temp folder).
    #[serde(default)]
    pub save_directory: Option<String>,
}

fn default_read_mb() -> f64 {
    DEFAULT_READ_MB
}

fn size_text(bytes: Option<u64>) -> String {
    match bytes {
        None => "size unknown".into(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1_048_576 => format!("{:.0} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / 1_048_576.0),
    }
}

fn is_text(mime: &str) -> bool {
    mime.starts_with("text/")
        || [
            "json",
            "xml",
            "csv",
            "javascript",
            "markdown",
            "yaml",
            "x-sh",
            "x-python",
            "sql",
        ]
        .iter()
        .any(|t| mime.contains(t))
}

/// A safe file name: no path separators, control characters or leading dots.
fn safe_file_name(name: &str, file_id: u64) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned
        .trim()
        .trim_start_matches('.')
        .chars()
        .take(200)
        .collect::<String>();
    if cleaned.is_empty() {
        format!("canvas-file-{file_id}")
    } else {
        cleaned
    }
}

fn default_save_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| Path::new(&home).join("Downloads"))
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Writes `bytes` to a new file in `dir`, never replacing an existing path (owner-only on Unix).
fn write_new_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = dir
        .canonicalize()
        .map_err(|_| format!("Folder does not exist: {}", dir.display()))?;
    if !dir.is_dir() {
        return Err(format!("Not a folder: {}", dir.display()));
    }
    let path = dir.join(name);
    if path.parent() != Some(dir.as_path()) {
        return Err(format!("Refusing an unsafe file name: {name}"));
    }
    let mut options = std::fs::OpenOptions::new();
    // create_new refuses any existing path, including a symlink planted there.
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!(
            "{} already exists; nothing was overwritten. Remove it or pass a different save_directory.",
            path.display()
        ),
        _ => format!("Could not create {}: {e}", path.display()),
    })?;
    file.write_all(bytes)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path)
}

impl CanvasServer {
    async fn file_meta(&self, course: &ResolvedCourse, file_id: u64) -> Result<File, String> {
        let file: File = self
            .client()
            .get(&format!("courses/{}/files/{file_id}", course.path_id), &[])
            .await
            .map_err(|e| hidden_tab(e, "Files"))?;
        if file.locked_for_user == Some(true) {
            let why = file
                .lock_explanation
                .as_deref()
                .map(inline)
                .unwrap_or_default();
            return Err(format!("{} is locked for you {why}", inline(file.name())));
        }
        Ok(file)
    }

    fn file_line(&self, f: &File) -> String {
        let mut line = format!(
            "- [file {}] {} · {}",
            f.id,
            inline(f.name()),
            size_text(f.size)
        );
        if let Some(mime) = &f.content_type {
            line.push_str(&format!(" · {mime}"));
        }
        line.push_str(&format!(
            " · updated {}",
            self.date(f.updated_at, "unknown")
        ));
        if f.locked_for_user == Some(true) {
            line.push_str(" · locked");
        }
        line
    }
}

#[tool_router(router = files_router, vis = "pub(super)")]
impl CanvasServer {
    /// List files in a course (name, size, type, last update), optionally filtered by name.
    /// Many courses hide the Files tab from students; module items often link files instead.
    #[tool(annotations(title = "Course files", read_only_hint = true, open_world_hint = true))]
    pub async fn list_course_files(
        &self,
        Parameters(p): Parameters<ListFilesParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        if p.search_term
            .as_ref()
            .is_some_and(|t| t.trim().chars().count() < 2)
        {
            return Err("search_term needs at least 2 characters.".into());
        }
        let sort = match p.sort {
            FileSort::Name => "name",
            FileSort::Size => "size",
            FileSort::CreatedAt => "created_at",
            FileSort::UpdatedAt => "updated_at",
            FileSort::ContentType => "content_type",
        };
        let order = match p.order {
            FileOrder::Asc => "asc",
            FileOrder::Desc => "desc",
        };
        let mut query = vec![("sort", sort), ("order", order)];
        if let Some(term) = &p.search_term {
            query.push(("search_term", term.trim()));
        }
        let files = self
            .client()
            .get_all::<File>(
                &format!("courses/{}/files", course.path_id),
                &query,
                MAX_FILES,
            )
            .await
            .map_err(|e| hidden_tab(e, "Files"))?;
        if files.items.is_empty() {
            return Ok(format!("No files found in {}.", course.label));
        }
        let mut out = format!("Files in {}:\n", course.label);
        for f in &files.items {
            out.push_str(&self.file_line(f));
            out.push('\n');
        }
        out.push_str(&more_note(files.truncated, files.items.len()));
        Ok(out)
    }

    /// Read a course file into the conversation: text files as text, images as images, and
    /// PDFs and other files as embedded content. Nothing is saved locally.
    #[tool(annotations(
        title = "Read course file",
        read_only_hint = true,
        open_world_hint = true
    ))]
    pub async fn read_course_file(
        &self,
        Parameters(p): Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, String> {
        if !(p.max_size_mb > 0.0 && p.max_size_mb <= MAX_READ_MB) {
            return Err(format!(
                "max_size_mb must be above 0 and at most {MAX_READ_MB}."
            ));
        }
        let max_bytes = (p.max_size_mb * 1_048_576.0) as u64;
        let course = self.resolve(&p.course_identifier).await?;
        let file = self.file_meta(&course, p.file_id).await?;
        if file.size.is_some_and(|s| s > max_bytes) {
            return Err(format!(
                "{} is {}, over the {} MB limit. Raise max_size_mb (up to {MAX_READ_MB}) or use download_course_file.",
                inline(file.name()),
                size_text(file.size),
                p.max_size_mb
            ));
        }
        let link = file
            .url
            .as_deref()
            .ok_or("Canvas gave no download link for this file.")?;
        let bytes = self.client().download(link, max_bytes).await.map_err(err)?;
        let mime = file
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        let header = format!(
            "{} ({}, {}) from {}",
            inline(file.name()),
            mime,
            size_text(Some(bytes.len() as u64)),
            course.label
        );
        let source = format!("file: {}", file.name());

        let body = if mime == "text/html" {
            ContentBlock::text(fence_untrusted(
                &html_to_text(&String::from_utf8_lossy(&bytes), MAX_TEXT_CHARS),
                &source,
            ))
        } else if is_text(&mime) {
            ContentBlock::text(fence_untrusted(
                &truncate(&String::from_utf8_lossy(&bytes), MAX_TEXT_CHARS),
                &source,
            ))
        } else if mime.starts_with("image/") {
            ContentBlock::image(STANDARD.encode(&bytes), mime.clone())
        } else {
            ContentBlock::resource(ResourceContents::BlobResourceContents {
                uri: format!("canvas://courses/{}/files/{}", course.path_id, file.id),
                mime_type: Some(mime.clone()),
                blob: STANDARD.encode(&bytes),
                meta: None,
            })
        };
        Ok(CallToolResult::success(vec![
            ContentBlock::text(header),
            body,
        ]))
    }

    /// Save a course file to a folder on this computer. Never overwrites an existing file;
    /// the saved copy is readable only by you. Use read_course_file to bring a file into the
    /// conversation instead.
    #[tool(annotations(
        title = "Download course file",
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = false,
        open_world_hint = true
    ))]
    pub async fn download_course_file(
        &self,
        Parameters(p): Parameters<DownloadFileParams>,
    ) -> ToolResult {
        let course = self.resolve(&p.course_identifier).await?;
        let file = self.file_meta(&course, p.file_id).await?;
        let dir = p
            .save_directory
            .as_deref()
            .map_or_else(default_save_dir, PathBuf::from);
        if !dir.is_dir() {
            return Err(format!("Folder does not exist: {}", dir.display()));
        }
        let link = file
            .url
            .as_deref()
            .ok_or("Canvas gave no download link for this file.")?;
        let bytes = self
            .client()
            .download(link, MAX_DOWNLOAD_BYTES)
            .await
            .map_err(err)?;
        let path = write_new_file(
            &dir,
            &safe_file_name(file.filename.as_deref().unwrap_or(file.name()), file.id),
            &bytes,
        )?;
        Ok(format!(
            "Saved {} ({}) to {}",
            inline(file.name()),
            size_text(Some(bytes.len() as u64)),
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_cannot_escape_the_folder() {
        assert_eq!(safe_file_name("../../etc/passwd", 1), "_.._etc_passwd");
        assert_eq!(safe_file_name("..hidden", 1), "hidden");
        assert_eq!(safe_file_name("  ", 9), "canvas-file-9");
        assert_eq!(safe_file_name("Lab 3: notes.pdf", 1), "Lab 3: notes.pdf");
    }

    #[test]
    fn write_new_file_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_new_file(dir.path(), "a.txt", b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        let err = write_new_file(dir.path(), "a.txt", b"two").unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn text_detection_covers_common_course_files() {
        assert!(is_text("text/plain") && is_text("application/json") && is_text("text/csv"));
        assert!(!is_text("application/pdf") && !is_text("image/png"));
    }
}
