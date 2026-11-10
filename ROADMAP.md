# Canvas MCP (Rust): Build Roadmap

A personal, from-scratch Canvas LMS MCP server written in **Rust**. Reference implementation:
[vishalsachdev/canvas-mcp](https://github.com/vishalsachdev/canvas-mcp) (Python, MIT, v1.17).
Read it for behavior and Canvas quirks, then design for Rust. It's a ~130-tool, multi-role,
hostable product. This project should be a small, sharp tool for one user that ships as a single binary.

---

## 1. Scope decisions (made up front)

| Decision | Choice | Why |
|---|---|---|
| Language | **Rust (stable, ≥ 1.88)** | `rmcp` 3.9 needs 1.88+ |
| MCP SDK | **`rmcp` 3.x** (official SDK), features `server`, `macros`, `transport-io`, `schemars` | `#[tool_router]`/`#[tool]` macros generate tool schemas from your structs |
| Async runtime | **tokio** | Required by rmcp |
| HTTP client | **reqwest 0.13** (`json`, rustls) | Async, connection pooling |
| Serialization | **serde / serde_json** + **schemars 1.x** | Tool input schemas come from `#[derive(JsonSchema)]` |
| Errors | **thiserror** (library errors), **anyhow** (in `main`) | Typed `CanvasError` that maps to readable tool errors |
| Transport | **stdio only** | Personal use. Claude launches the binary directly |
| Role | **Student (self) first**; educator tools only if needed | Most personal value; "self" endpoints are low-risk |
| Writes | **Off by default**, opt-in per tool, preview → confirm | Canvas writes are visible to instructors and usually can't be undone |

**Setup needed:** Rust isn't installed yet. Install it with `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
(or `brew install rustup && rustup-init`).

### Deliberately NOT building (reference features to skip)
- Streamable-HTTP hosting, Azure deploy, Entra ID, X-Canvas-Token header auth
- FERPA anonymization layer (only matters for educator data about *other* people)
- `execute_typescript` code sandbox and the TS code API
- Accessibility/UFIXIT scanner, content migrations, group creation from CSV
- Role/tool-policy engine, audit logging, .mcpb bundle, registry publishing

Any of these can come back later as an isolated module.

---

## 2. Crate dependencies (latest stable as of 2026-11-10)

```toml
[package]
name = "canvas-mcp"
edition = "2024"
rust-version = "1.88"

[dependencies]
rmcp = { version = "3.9", features = ["server", "macros", "transport-io", "schemars"] }
tokio = { version = "1.54", features = ["macros", "rt-multi-thread", "io-std", "sync", "time"] }
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls", "query"] }  # 0.13: .query() needs the "query" feature
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "1.2"
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4.6", features = ["derive", "env"] }
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.10"
dotenvy = "0.15"
html2text = "0.17"       # Canvas HTML → plain text
url = "2"
secrecy = "0.10"          # keeps the token from being printed by Debug

[dev-dependencies]
wiremock = "0.6"          # fake Canvas server in tests
tokio = { version = "1.54", features = ["test-util"] }
```
Check exact feature names (for example reqwest's TLS feature) with `cargo add` when you scaffold.

---

## 3. Target layout

A **library + thin binary**, so integration tests can build the server against a mock Canvas.

```
canvas-mcp/
├── Cargo.toml
├── .env.example              # CANVAS_API_URL, CANVAS_API_TOKEN, CANVAS_TIMEZONE
├── src/
│   ├── main.rs               # clap args (--check), tracing → stderr, load config, serve(stdio())
│   ├── lib.rs                # pub mod …; build_server(config) -> CanvasServer
│   ├── config.rs             # Config { api_url: Url, token: SecretString, tz: Tz, write_tools: HashSet }
│   ├── error.rs              # CanvasError (thiserror) + to_tool_error()
│   ├── canvas/
│   │   ├── client.rs         # CanvasClient: auth, get/post/put, paginate, throttle, retry
│   │   ├── pagination.rs     # Link header parsing (rel="next")
│   │   └── models.rs         # slim serde structs: Course, Assignment, Submission, …
│   ├── resolve.rs            # course identifier → CourseId (id | code | name | sis_course_id:)
│   ├── format.rs             # dates → local tz, html_to_text, truncate(max_chars)
│   ├── untrusted.rs          # fence Canvas-authored text as data (prompt-injection guard)
│   ├── cache.rs              # TTL cache (tokio::sync::RwLock) for course list
│   └── tools/
│       ├── mod.rs            # CanvasServer struct + #[tool_handler] impl ServerHandler
│       ├── profile.rs        # get_my_profile, list_courses
│       ├── assignments.rs    # upcoming, details, submission status
│       ├── grades.rs         # course grades, per-assignment scores, what-if
│       ├── planner.rs        # todo, calendar
│       ├── content.rs        # modules, pages, files, syllabus
│       ├── discussions.rs    # announcements, topics, entries
│       └── writes.rs         # (opt-in) submit, comment, reply
└── tests/
    ├── fixtures/*.json       # handwritten/recorded Canvas responses
    ├── client_tests.rs       # pagination, errors, throttling vs wiremock
    └── tool_tests.rs         # call tools end-to-end vs wiremock
```

**Splitting tools across files:** rmcp's `#[tool_router]` works per `impl` block. Give each module its own
`#[tool_router(router = profile_router)] impl CanvasServer { … }` and combine the routers in `mod.rs`
(`Self::profile_router() + Self::assignments_router() + …`). Confirm the exact syntax in the `rmcp-macros`
docs for 3.x when you get to Phase 4.

---

## 4. Phases

### Phase 0: Setup (≈1 hr)
- [x] Install Rust via rustup; `rustup component add clippy rustfmt`
- [x] `cargo new canvas-mcp` (or init in this folder), `git init`, add deps with `cargo add`
- [ ] Create a Canvas token: *Account → Settings → + New Access Token*
      (some schools issue tokens via a request form instead, and some expire after 30 days)
- [ ] `.env`: `CANVAS_API_URL=https://<school>.instructure.com/api/v1`, token; add `.env` to `.gitignore`
- [ ] Sanity check: `curl -H "Authorization: Bearer $TOKEN" $CANVAS_API_URL/users/self`

### Phase 1: Hello-world server (≈1–2 hrs)
- [x] `CanvasServer { client: Arc<CanvasClient> }` (`#[derive(Clone)]`, which rmcp requires, so share state via `Arc`)
- [x] One tool `get_my_profile` → `GET /users/self`
- [x] `main`: `tracing_subscriber` writing to **stderr** (stdout is the MCP channel, so **no `println!` anywhere**),
      then `server.serve(rmcp::transport::stdio()).await?.waiting().await?`
- [x] `--check` flag: validate URL + token against `/users/self` and exit
- [ ] `cargo install --path .` → binary at `~/.cargo/bin/canvas-mcp`
- [ ] Wire into Claude Code: `claude mcp add canvas -e CANVAS_API_URL=... -e CANVAS_API_TOKEN=... -- ~/.cargo/bin/canvas-mcp`
      (or Claude Desktop's `claude_desktop_config.json`, or load `.env` with dotenvy from a fixed path)
- [ ] Debug with `npx @modelcontextprotocol/inspector ~/.cargo/bin/canvas-mcp`
- **Done when:** Claude can answer "who am I on Canvas?"

### Phase 2: The Canvas client core (most important layer)
Every tool goes through this. Get it right once.
- [x] `CanvasClient::new(config)`: one `reqwest::Client` with default `Authorization: Bearer` header
      (mark it sensitive), timeout, user agent
- [x] Generic helpers: `get<T: DeserializeOwned>(path, &query)`, `post`, `put`
- [x] **Pagination:** `get_all<T>(path, query, limit)` follows the `Link` header's `rel="next"` URL;
      `per_page=100`; hard cap on pages/items so output stays small
- [x] **Errors:** `CanvasError::{Unauthorized, Forbidden, NotFound, Throttled, Validation(String), Http(..), Decode(..)}`;
      a single function turns these into a short message for the model
- [x] **Tool errors vs protocol errors:** return a tool error result (`CallToolResult::error` / `isError: true`)
      for Canvas failures, so the model sees the message and can recover; reserve `Err(McpError)` for bad params
- [x] **Throttling:** read `X-Rate-Limit-Remaining`; on 403 "Rate Limit Exceeded" back off with jitter and retry;
      `tokio::sync::Semaphore` (~5 permits) for concurrent fan-out
- [x] Validate at startup: URL is `https` and ends in `/api/v1`
- [x] Tests (wiremock): 3-page pagination, 401 message, throttle → retry → success, malformed JSON
- **Reference to read:** `src/canvas_mcp/core/client.py`, `core/config.py`

### Phase 3: Models and helpers
- [x] **Slim models:** define only the fields you use; serde ignores the rest. Canvas has many nullable
      fields, so use `Option<T>` liberally. IDs are `u64`. Dates are `Option<DateTime<Utc>>`.
- [ ] `resolve_course(&str) -> Result<CourseId, ResolveError>`: numeric id, course code, partial name
      (case- and space-insensitive), or `sis_course_id:X`. Refuse with "did you mean…" suggestions
      when nothing matches. Never send a guess to Canvas.
- [x] `format_date(dt, tz)` with relative hints ("due in 2 days")
- [ ] `html_to_text(html, max_chars)` via html2text, with a clear truncation marker
- [ ] `fence_untrusted(text, source)`: wrap instructor/student-authored text in clear delimiters
- [ ] TTL cache for the course list (resolution runs on almost every call)
- [ ] Unit tests for resolver matching and date formatting
- **Reference to read:** `core/validation.py`, `core/dates.py`, `core/html.py`, `core/untrusted_content.py`

### Phase 4: Read-only student tools (the core value)
Each tool gets a `#[derive(Deserialize, JsonSchema)]` params struct. **Doc comments on fields become the
parameter descriptions the model sees**, so write them for the model. Return compact text or JSON;
don't pass raw Canvas payloads through.

| Tool | Canvas endpoint(s) |
|---|---|
| `list_courses` | `GET /courses?enrollment_state=active&include[]=term` |
| `get_my_upcoming_assignments(days=7)` | `GET /users/self/upcoming_events` or per-course `/assignments?bucket=upcoming` |
| `get_my_todo_items` | `GET /users/self/todo` |
| `get_assignment_details(course, assignment)` | `GET /courses/:id/assignments/:id` |
| `get_my_submission_status(course?)` | `GET /courses/:id/students/submissions?student_ids[]=self` (missing/late flags) |
| `get_my_course_grades` | `GET /users/self/enrollments?include[]=current_points` |
| `get_my_assignment_scores(course)` | `/assignment_groups?include[]=assignments&include[]=submission` |
| `list_announcements(course?)` | `GET /announcements?context_codes[]=course_:id` |
| `list_discussion_topics` / `get_discussion(…)` | `/courses/:id/discussion_topics`, `/view` |
| `list_modules` (with items) | `/courses/:id/modules?include[]=items` |
| `get_page(course, page)` | `/courses/:id/pages/:url` |
| `get_syllabus(course)` | `/courses/:id?include[]=syllabus_body` |
| `list_course_files` | `/courses/:id/files` |
| `list_calendar_events(start, end)` | `/calendar_events?context_codes[]=…` |
| `get_my_peer_reviews_todo` | `/courses/:id/assignments/:id/peer_reviews` (filtered to self) |
| `list_quizzes` | `/courses/:id/quizzes` (classic; New Quizzes show up as assignments) |

- [ ] Annotate read tools as read-only (`read_only_hint = true`) in the `#[tool(...)]` attribute (check the 3.x macro docs for the exact syntax)
- [ ] Fan out across courses with `futures::future::join_all` behind the semaphore
- **Done when:** "What's due this week and what am I missing?" works end-to-end

### Phase 5: Grade calculator (good fit for Rust)
- [ ] Pure module `grades/calc.rs` with no I/O: assignment groups with weights, `drop_lowest`/`drop_highest`,
      `never_drop`, excused, unposted/muted, points vs weighted courses
- [ ] `calculate_grade_scenarios(course, hypothetical_scores?, target?)`: what-ifs and "what uniform % do I need on the rest?"
- [ ] Table-driven unit tests checked against grades Canvas actually shows you; consider `proptest` for drop rules
- **Reference to read:** `core/grade_calc.py`, `tools/student_grades.py`

### Phase 6: Opt-in writes (only if you want them)
- [ ] `CANVAS_WRITE_TOOLS=post_discussion_reply,...`: write tools not on the list are never added to the router
- [ ] Two-step pattern: call 1 returns a **preview + single-use confirmation token** (random, stored in a
      `Mutex<HashMap<Token, (PendingWrite, Instant)>>` with a TTL); call 2 with the token executes; then
      **read back** to verify Canvas actually did it
- [ ] Candidates: `post_discussion_reply`, `comment_on_my_submission`, `mark_module_item_done`,
      `submit_assignment` (text/URL first; file upload is a 3-step Canvas flow)
- [ ] Set `destructive_hint` / `idempotent_hint` correctly
- **Reference to read:** `core/write_confirmation.py`, `core/guarded_edit.py`, `tools/student_write.py`

### Phase 7: Hardening and polish
- [ ] Integration tests: start wiremock, build `CanvasServer` against its URL, call tools through an rmcp
      client over an in-memory duplex (`tokio::io::duplex`), and assert the request sent *and* the output.
      Never test writes against live Canvas.
- [ ] A metadata test: every listed tool has a description and annotations
- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` in a pre-commit hook and/or GitHub Actions
- [ ] Token never shows up in logs or `Debug` output (`secrecy::SecretString`)
- [ ] Output size caps; `max_chars` params on tools that return large text
- [ ] README: build/install, env vars, client config snippets, tool list
- [ ] Optional: `cargo dist` or release binaries for macOS so you don't need a toolchain on other machines

### Phase 8: Optional extensions
- MCP **resources** (e.g. `canvas://course/{id}/syllabus`) and **prompts** ("weekly plan") via `ServerHandler`
- Claude Code **skills** for workflows (morning check, week plan); the reference ships 8 of these
- Educator toolset (rubrics, bulk grading, analytics) behind a flag, *plus* anonymization before touching other students' data
- Streamable-HTTP transport (`transport-streamable-http-server` feature) if you ever want to reach it from another device

---

## 5. Gotchas

**Canvas API**
- `CANVAS_API_URL` must include `/api/v1`; tokens are per-user and carry *all* your permissions
- Pagination is Link-header based. Missing it silently truncates lists to 10 items
- Throttling shows up as **403** (not 429) with "Rate Limit Exceeded"
- Course codes aren't unique across terms. Prefer active enrollments when resolving
- Dates are UTC ISO-8601; due dates can be null; per-student overrides change `due_at`
- New Quizzes (LTI) expose almost nothing via REST beyond the assignment
- Descriptions, pages and posts are HTML written by others. Treat them as untrusted input
- `include[]=` params avoid N+1 calls (e.g. `include[]=submission` on assignments). With reqwest,
  pass repeated keys as a `Vec<(&str, &str)>` query, since a map would collapse them

**Rust-specific**
- Anything written to **stdout** that isn't MCP JSON breaks the connection. Log to stderr only.
- rmcp's server type must be `Clone`, so put the client, cache and token store behind `Arc`
- Some Canvas fields switch between number and string, or are absent instead of null.
  Use `Option`, `#[serde(default)]`, and a small custom deserializer only where you actually hit this
- Don't model full Canvas objects. Slim structs keep compile times and breakage down
- rmcp 3.x changed APIs from earlier versions; most blog posts and examples online are for 0.x.
  Rely on docs.rs/rmcp and the repo's `examples/` folder

## 6. Milestones
1. **M1:** binary runs in Claude, `get_my_profile` works (Phases 0–1)
2. **M2:** robust client, models and course resolution, tested with wiremock (Phases 2–3)
3. **M3:** daily-use read tools: due soon, todo, grades, announcements (Phase 4)
4. **M4:** grade what-if calculator (Phase 5)
5. **M5:** opt-in writes with confirmation tokens (Phase 6)
6. **M6:** tests, clippy, docs and release builds (Phase 7)
