# canvas-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) server for **Canvas LMS**, written in Rust. It lets Claude (or any MCP client) read your own Canvas data: deadlines, grades, announcements, course content and more.

It's built for students and ships as a single binary. Reading is the default: nothing is submitted or changed in Canvas.

## Features

46 read-only tools, including:

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
cp .env.example .env    # then edit .env
cargo install --path .
canvas-mcp --check      # verifies your login
```

In `.env`, set your Canvas URL (ending in `/api/v1`) and **one** way to sign in:

- **`CANVAS_API_TOKEN`**: from Canvas → Account → Settings → *New Access Token*.
- **`CANVAS_SESSION_COOKIE`**: if your school doesn't let students create tokens. Copy the `Cookie` request header from your browser's DevTools and wrap it in single quotes. Only use this where your school's rules allow it. It expires when your Canvas session does.

`CANVAS_TIMEZONE` is optional and defaults to your computer's time zone.

### Add to Claude Code

```bash
claude mcp add canvas -- bash -c 'cd /path/to/canvas-mcp && exec ~/.cargo/bin/canvas-mcp'
```

The server reads `.env` from its working directory, so the command changes into the project folder first.

## Safety

- **Credentials stay local.** They're sent only to your Canvas host, never logged, and file downloads use signed links without them.
- **Untrusted text is marked.** Text written by instructors and classmates is wrapped in `<untrusted>` blocks, so the model treats it as content, not instructions.
- **Write tools are opt-in.** The planned write tools (Phase 6) will be off by default, with a preview-and-confirm step for each action.

## Development

```bash
cargo test                           # runs against a mock Canvas (wiremock); no account needed
cargo clippy --all-targets -- -D warnings
```

See [ROADMAP.md](ROADMAP.md) for the plan and progress. Tool design follows [vishalsachdev/canvas-mcp](https://github.com/vishalsachdev/canvas-mcp) (Python); this is an independent Rust implementation.
