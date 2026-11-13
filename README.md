# canvas-mcp

[![CI](https://github.com/cmkhoa/canvas-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/cmkhoa/canvas-mcp/actions/workflows/ci.yml)

A [Model Context Protocol](https://modelcontextprotocol.io) server for **Canvas LMS**, written in Rust. It lets Claude (or any MCP client) read your own Canvas data: deadlines, grades, announcements, course content and more.

It's built for students and ships as a single binary. Reading is the default: nothing is submitted or changed in Canvas.

> **Note:** This is a personal, experimental project for educational purposes. It isn't affiliated with Instructure or any school. Use it only with your own account, and in line with your institution's policies.

## Features

47 read-only tools, including an additive structured `get_planning_snapshot` for agent clients:

| Area | Tools |
|---|---|
| Courses | `list_courses`, `get_my_enrollments`, `get_course_details`, `get_syllabus` |
| Deadlines | `get_my_upcoming_assignments`, `get_my_todo_items`, `get_my_submission_status`, `get_assignment_details`, `get_my_submission` |
| Grades | `get_my_course_grades`, `get_my_assignment_scores`, `calculate_grade_scenarios` (what-if scores and "what do I need" targets, using Canvas's own math) |
| Feed | `list_announcements`, `list_my_announcements`, `get_my_activity_stream` |
| Content | modules, pages, front page, course structure, discussions (including group spaces), files, quizzes |
| Other | peer reviews, groups, calendar, planner notes, inbox (reading never marks messages read) |

Courses can be named loosely: `"CS 101"`, `"organic chem"`, a numeric ID, or `sis_course_id:…`.

## Setup

Requires Rust 1.88+.

```bash
git clone https://github.com/cmkhoa/canvas-mcp && cd canvas-mcp
cp .env.example .env    # then edit .env (see below)
cargo install --path .
canvas-mcp --check      # verifies your login
```

In `.env`, set your Canvas URL (ending in `/api/v1`) and **one** way to sign in:

- **`CANVAS_API_TOKEN`**: from Canvas → Account → Settings → *New Access Token*.
- **`CANVAS_SESSION_COOKIE`**: if your school doesn't let students create tokens. Copy the `Cookie` request header from your browser's DevTools and wrap it in single quotes. Only use this where your school's rules allow it. It expires when your Canvas session does.

`CANVAS_TIMEZONE` is optional and defaults to your computer's time zone.

### Connect a client

Any MCP client that can launch a local program works. First put your settings where every client can find them, and make sure `~/.cargo/bin` is on your `PATH` (or use the full path below):

```bash
mkdir -p ~/.config/canvas-mcp
mv .env ~/.config/canvas-mcp/.env && chmod 600 ~/.config/canvas-mcp/.env
```

The server reads settings from `--env-file PATH` (or `CANVAS_MCP_ENV_FILE`), else `./.env`, else `~/.config/canvas-mcp/.env`. `canvas-mcp --check` shows which file it used.

**Claude Code**

```bash
claude mcp add canvas -- canvas-mcp
```

**OpenAI Codex** (MCP servers live in `~/.codex/config.toml`)

```bash
codex mcp add canvas -- canvas-mcp
```

or add this to `~/.codex/config.toml`, using the full path if Codex can't find `canvas-mcp` on its `PATH`:

```toml
[mcp_servers.canvas]
command = "/Users/you/.cargo/bin/canvas-mcp"
args = ["--env-file", "/Users/you/.config/canvas-mcp/.env"]
```

Check it with `codex mcp list`.

**Google Antigravity** (Gemini-based; tool schemas are already simplified for it)

Add an entry to `~/.gemini/antigravity/mcp_config.json`, using absolute paths because the editor may not share your shell's `PATH`:

```json
{
  "mcpServers": {
    "canvas": {
      "command": "/Users/you/.cargo/bin/canvas-mcp",
      "args": ["--env-file", "/Users/you/.config/canvas-mcp/.env"]
    }
  }
}
```

If the file already has other servers, add `"canvas"` next to them inside `mcpServers`. Reload the editor's MCP servers (or restart it) afterwards.

## Safety

- **Credentials stay local.** They're sent only to your Canvas host, never logged, and file downloads use signed links without them.
- **Untrusted text is marked.** Text written by instructors and classmates is wrapped in `<untrusted>` blocks, so the model treats it as content, not instructions.
- **Write tools are opt-in.** The planned write tools (Phase 6) will be off by default, with a preview-and-confirm step for each action.

## Development

```bash
cargo test                           # runs against a mock Canvas (wiremock); no account needed
cargo clippy --all-targets -- -D warnings
cargo deny check                     # advisories, licenses, sources (needs cargo-deny)
```

See [ROADMAP.md](ROADMAP.md) for the plan and progress. Tool design follows [vishalsachdev/canvas-mcp](https://github.com/vishalsachdev/canvas-mcp) (Python); this is an independent Rust implementation.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
