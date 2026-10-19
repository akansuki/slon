# Slon: status and decisions (2026-10-15)

Source of truth: this file in the repo. Why things work the way they do: [DESIGN.md](DESIGN.md). A copy is kept in the claude.ai Project "Rewrite pgAdmin in Rust" (`claude/slon-status.md`).

Formerly codenamed pgrs; renamed to **Slon** (Serbian for "elephant", the Postgres mascot) on 2026-10-15. Name checked only against
GitHub / npm / crates.io: no GUI client with that name; NinoFloris/Slon is a .NET Postgres protocol library, npm has an empty
`slon` package. No trademark search done.

Lightweight pgAdmin alternative: Rust backend, TypeScript (Svelte 5) frontend, Tauri 2, targets Windows (WebView2).

## Rename notes (pgrs -> slon)
- Renamed: app/product name, binary and lib (`slon`, `slon_lib`), bundle identifier `dev.slon.app`, Credential Manager service
  `slon`, `SLON_*` environment variables (`SLON_MEMORY_SECRETS`, `SLON_EXPORT_DIR`, `SLON_TEST_*`, `SLON_REQUIRE_DB`), localStorage keys
  `slon.*`, PostgreSQL `application_name`. Crates `pg-core` / `pg-profiles` keep their names.
- No migration, on purpose: saved connections lived in `%APPDATA%\dev.pgrs.app` and passwords under the Credential Manager
  service `pgrs`; the renamed app starts empty (old data can be deleted or copied to `%APPDATA%\dev.slon.app\`; old passwords must be re-entered).
- E2E: fixed a race in the TLS check (first `idle()` can win against the Run click), now waits for the result row.

## State
Vertical slice is built, verified and confirmed running on Windows: connect, lazy tree, CodeMirror 6 SQL editor, streamed
results in a virtualized grid with client-side type-aware sorting, server-side cancel, multiple query tabs (one connection each).

Confirmed working on Windows (before the rename): saved connections, grid copy/selection, resizable columns, schema-aware
autocomplete. On 2026-10-15 the user reported "everything is working" for the later additions (table properties, TLS verify modes,
EXPLAIN, messages, sidebar, triggers/sequences, export, history, restored tabs) without itemizing what was tried. The renamed build has not been run on Windows.

Features: connect (incl. TLS verify-ca/verify-full with PEM or OS store), lazy tree with table properties (columns, indexes,
constraints, triggers, sequences, CREATE script), SQL editor with schema-aware autocomplete, streamed sortable grid with copy and
resizable columns, cancel, multiple tabs, EXPLAIN / EXPLAIN ANALYZE plan view, server messages tab, resizable sidebar,
export (CSV/TSV/JSON/JSONL), query history (Ctrl+H), remembered tabs per server, version in the sidebar footer
(click to copy bug-report details) and the connection in the title bar.

Verified on Linux (PostgreSQL 16): 7 pg-core unit + 24 integration tests, 22 pg-profiles store tests + 12 history/workspace
tests (+1 ignored real credential store), 2 src-tauri tests, 84 UI unit tests, svelte-check clean, 139/139 end-to-end checks.
Not verified: Excel opening the exported CSV, Windows certificate store as trust source, real RDS / company CA, plans from other
server versions (parser written against PG 16 JSON; PG 18 adds fields it ignores).

## Releases
`.github/workflows/release.yml` builds on a Windows runner when a `v*` tag is pushed and attaches both installers to a
**draft** release, so the assets are reviewed before anything is public. A guard step fails the run when the tag does not
match the version in `src-tauri/tauri.conf.json`, which is what names the installer files; keep `package.json` and
`src-tauri/Cargo.toml` in step with it when bumping. Releases are unsigned (see the Next list).

## Screenshots
`e2e/screenshots.mjs` drives the real app through tauri-driver and writes `docs/img/*.png`; `e2e/demo.sql` is the
schema they are taken against. The committed set was rendered from a release build on Linux/WebKitGTK, whose fallback monospace font is
wider than the one WebView2 uses, so a timestamp column ellipsizes there that fits on Windows. Re-run the script on
Windows (without xvfb) for the rendering users actually see.

## Layout
- crates/pg-core: engine (tokio-postgres + rustls/ring), no UI deps; ddl.rs builds CREATE scripts; explain.rs runs EXPLAIN;
  notice.rs collects server messages
- crates/pg-profiles: saved connections (profiles.json + OS credential store via keyring); history.rs, workspace.rs and
  jsonfile.rs (atomic JSON files for history and tabs)
- src-tauri: thin command layer; results stream over a Tauri Channel; export.rs (save dialog, chunked file writes)
- src: Svelte UI (plan.ts, export.ts, history.ts, layout.ts pure helpers with tests; PlanView, MessagesView, ExportDialog,
  HistoryPanel components); e2e: WebDriver test; scripts/test-db.sh: throwaway Postgres (private CA, TLS) for tests

## Decisions
- Simple query protocol (text values, multi-statement scripts); column types via a prior `prepare` (single statement only).
- Client-side sorting on a 100,000-row cap per statement; UI warns when truncated. Sorting applies after the query finishes.
- Cap on a single statement cancels the server; scripts are never cancelled at the cap (would roll back the implicit transaction).
- Completion is signalled by finished/failed events on the same channel as rows (command return value can overtake channel messages).
- Custom virtualized grid instead of TanStack Table/Virtual.
- TLS: prefer/require encrypt without verification (libpq semantics). verify-ca = chain only; verify-full = chain + host name.
  Verifying modes never fall back to plain text. CA file replaces the system store when given. No client certificates, no revocation.
- Saved connections: settings in `%APPDATA%\dev.slon.app\profiles.json` (no secrets); password only if "Remember the password" is
  ticked, stored in Windows Credential Manager (service `slon`, key `connection:<id>`); a blank password uses the saved one only
  while host/port/user match; saves are atomic across file + credential store.
- Debug builds accept `SLON_MEMORY_SECRETS=1` (E2E never touches a real vault) and `SLON_EXPORT_DIR` (skips the native save dialog).
- Copy format: tab-separated, quoted when needed; single cell verbatim; NULL copies as empty. Grid drags use mouse events.
- Autocomplete: one catalog query per tab/database (cap 5,000 relations); bare columns from tables in the current statement.
- Table properties: CREATE script uses server `quote_ident`; gaps: grants, rules, policies, storage options, inheritance/partition
  children, foreign table options. Sequences from pg_depend (serial/OWNED BY and identity); numbers as text; `last_value` can run
  ahead of the rows with CACHE > 1.
- EXPLAIN: always `FORMAT JSON`. Analyze runs in BEGIN..ROLLBACK, or SAVEPOINT..ROLLBACK TO when a user transaction is open.
  One statement only (`prepare`d first). Not rolled back by Postgres: sequence values, outside-world calls.
- Server messages: connection driven through `poll_message`, queued in a NoticeBox; only order and elapsed time are reliable;
  cap 10,000 per run; LISTEN/NOTIFY not shown.
- Export: rows the grid holds (<=100,000), in displayed sort order. CSV per RFC 4180, NULL = empty field, empty string = `""`;
  JSON numbers keep server digits. The page may only write to a path the Rust-side save dialog returned.
- History/tabs: plain-text JSON files in the config folder, atomic writes; a damaged file is moved to `*.bad`. Password check on
  statements is best effort. Results are not persisted.

## Next
Copy as INSERT/JSON, export of all rows of a large result (needs a re-run), client certificates, import/export of saved
connections, EXPLAIN options (VERBOSE, SETTINGS, text view), comparing two plans, grants/rules/policies in table properties,
autocomplete for CTEs/functions/types, code signing for the installers.
