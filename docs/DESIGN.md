# Design decisions worth knowing

Why Slon works the way it does. The current state and the to-do list are in [STATUS.md](STATUS.md).

**Simple query protocol, text values.** Statements run through the simple query protocol, so the
server formats every value exactly as `psql` would (dates, numerics, arrays, JSON, ...) and
scripts with several statements work. Column *types* come from a `prepare` beforehand, which is
only possible for a single statement; for scripts the grid falls back to inferring numeric
columns from their values.

**Client-side sorting, bounded.** Sorting happens in the UI on the rows received, using
the column type: numbers compare numerically and exactly (a `bigint` above 2^53 sorts correctly),
booleans `f < t`, NULL last when ascending like PostgreSQL. Results are capped at 100,000 rows per
statement and the status bar says so when a result is truncated, because a sort over a truncated
result is only a sort of what was fetched. Sorting is applied when the query finishes.

**Cancellation.** Each tab owns its connection, so cancelling one never touches another. Cancel
sends a real PostgreSQL cancel request. When a *single* statement hits the row cap, the server is
cancelled so it stops producing rows; scripts are never cancelled at the cap, because the error
would roll back the implicit transaction around the whole script (there is a test for exactly
that).

**Completion is ordered with the data.** A Tauri command's return value and its channel messages
travel on different paths, so a command can resolve before the last row batch is processed. The
backend therefore sends explicit `finished` / `failed` events on the same channel as the rows.

**TLS.** `prefer` / `require` encrypt the connection without verifying the server certificate,
like libpq's modes of the same name (so self-signed servers work). `verify-ca` checks that the
certificate chains to a trusted CA; `verify-full` also checks that it was issued for the host name you
typed (use the name in the certificate, not an IP, unless the certificate lists the IP). Trusted CAs
come from the PEM file you choose (needed for private CAs and AWS RDS: `global-bundle.pem`), or, when the
field is empty, from the operating system's store (Windows certificate store on Windows), like libpq's
`sslrootcert=system`. Both verifying modes refuse to fall back to plain text. Errors say what is wrong
("not signed by a CA you trust", "not issued for '<host>'", expired) and what to do about it.
Not supported yet: client certificates, CRL/OCSP revocation checks.

**Table properties.** Expanding a table runs a handful of catalog queries (columns with types,
defaults, identity/generated, comments; indexes; constraints; triggers; the sequences behind `serial` and
identity columns) and shows them under the table. Triggers show timing, events and level (`BEFORE INSERT OR
UPDATE · ROW`) and are flagged when disabled; sequences show their column and type, with last value, step and
range in the tooltip (with `CACHE` above 1 the last value can run ahead of the rows). The *CREATE script* is
rebuilt from the catalog, quoted by the server's own `quote_ident`: `serial` sequences are created before the
table and `OWNED BY` the column afterwards, identity options are kept when they differ from the defaults,
triggers follow the indexes (with `DISABLE` / `ENABLE REPLICA` / `ENABLE ALWAYS` when needed). Integration
tests replay the script on an empty slate and check that the result describes itself identically. Known
limits: grants, storage options, rules, policies, table inheritance / partition children are not included;
a foreign table's server and options are not reproduced; the trigger function is not part of the script.

**Explain.** *Explain* shows the plan the server would use without running anything; *Explain analyze*
really runs the statement and shows actual timings, row counts and buffers. Both ask for
`EXPLAIN (FORMAT JSON)` and the UI works from that document (the *Copy JSON* button hands it to tools
such as explain.dalibo.com). Analyze is wrapped in a transaction that is always rolled back, or, when you
have a transaction of your own open in the tab, in a savepoint so yours survives; effects PostgreSQL does
not roll back (sequence values, calls to the outside world) still happen. Only one statement can be
explained: a script is refused before anything runs, because `EXPLAIN` would otherwise execute whatever
follows the first statement. The tree shows, per node, its *own* time or cost (children subtracted) as a
share of the whole, and flags row estimates that are 10x off, sorts and hashes that spilled to disk,
filters that discarded 90% or more of the rows read, parallel workers that did not start, and index-only
scans with heap fetches. Self time is summed over all loops; for parallel nodes it is an approximation.

**Server messages.** `RAISE NOTICE`, warnings (`there is no transaction in progress`), `NOTICE: ... does
not exist, skipping`, `VACUUM VERBOSE` output and the like arrive in a **Messages** tab next to Results,
with severity, detail, hint, the function call stack (`CONTEXT`) and the time since the run started.
The tab opens by itself for a statement that returns no rows, and messages show up while the statement is
still running (a `DO` block reporting progress). A run that fails keeps the messages raised before the
error. *Copy all* produces psql-style text. The engine drives the connection through `poll_message`
because merely awaiting the connection discards notices. Limits: messages are not tied to a particular
statement of a script (the connection can run ahead of the reader, so only their order and time are
reliable), at most 10,000 are shown per run (a closing line says how many were left out), and
`LISTEN/NOTIFY` notifications are not shown.

**Sidebar width.** Drag the border between the sidebar and the query area (double-click or Enter resets it;
arrow keys, Shift+arrows, Home and End work when it has focus). The width is kept between 180 px and 800 px,
never leaves the query area under 360 px, and is remembered in the webview's local storage (if storage is
unavailable the app starts at the default width).

**Saved connections.** Settings (name, host, port, database, user, SSL mode, last-used time) live in
`profiles.json` in the app's config folder (`%APPDATA%\dev.slon.app` on Windows). A password is only stored
if you tick *Remember the password*, and then it goes to the operating system's credential store
(Windows Credential Manager, entry `slon` / `connection:<id>`), never into the file. The UI only ever
learns *whether* a password is saved. A blank password field means "use the saved one", but only while
host, port and user still match the profile, so editing the host cannot send the saved password
somewhere else. Saving changes the password and the file together: if the file write fails the
credential is rolled back. A damaged or newer-format `profiles.json` is reported and left untouched,
never overwritten. Debug builds honour `SLON_MEMORY_SECRETS=1` to keep passwords in memory (used by
the end-to-end test so it never touches a real vault).

**Copying.** Click a cell, drag or Shift+click / Shift+arrows for a block, click a row number for whole
rows, Ctrl+A for everything. Ctrl+C copies tab-separated text that pastes straight into Excel (values with
tabs, line breaks or quotes are quoted); a single cell is copied verbatim; NULL becomes an empty field.
Ctrl+Shift+C adds the column names. Sorting clears the selection, because the rows move.

**Autocomplete.** When a tab opens, one catalog query fetches every relation and its columns (capped at
5,000 relations). Qualified names (`schema.table.`, `alias.`) come from CodeMirror's SQL schema support;
bare column names are suggested from the tables the *current statement* mentions (`FROM`, `JOIN`,
`UPDATE`, `INTO`), and names that need quoting are inserted quoted. The names refresh by themselves after
a statement containing `CREATE`/`ALTER`/`DROP`, and with the *Refresh names* button. Limits: comma-joined
tables (`FROM a, b`), CTE and subquery column names are not resolved.

**Export.** The *Export* button saves what the grid holds (at most the 100,000 rows it keeps, in the
sorted order currently shown) as CSV/TSV, JSON (array of objects) or JSON Lines. The file is chosen in
the native save dialog, which is opened by the Rust side; the page can only write to a path that dialog
just returned, and only `.csv/.tsv/.json/.jsonl/.txt`. CSV follows RFC 4180 (CRLF line ends); like
`COPY ... CSV`, NULL is an empty field and an empty string is `""`. A UTF-8 byte order mark is on by
default because Excel on Windows otherwise garbles č, ć, š, đ. In JSON, numbers keep the digits the server
sent (no float rounding of bigint/numeric), booleans become true/false, json/jsonb values are embedded as
JSON, and everything else is a string. Rows are written in 5,000-row chunks. Options are remembered.
The end-to-end test sets `SLON_EXPORT_DIR` (debug builds only) to skip the native dialog.

**Query history.** Every run (not Explain) is recorded in `history.json` in the config folder: the statement, server
(`user@host:port`), database, success or the error, duration, rows. The *History* button (Ctrl+H) opens a panel with
search, an *All servers* switch (default: this server only), *New tab* (also double-click) and *Use here* (replaces the
text of the current tab). Running the same statement again right away updates the entry (×N) instead of adding one.
Newest 1,000 are kept; statements over 100 KB are not recorded. Statements that look like they carry a password
(`PASSWORD 'x'`, `password=x`, `PASSWORD $$x$$`, also inside comments) are never stored: a best-effort check, so keep real
secrets out of SQL anyway. *Record history* in the panel switches recording off; *Clear* is two clicks. The file is plain
text. A damaged file is moved to `history.json.bad` and the history starts empty.

**Remembered tabs.** The open tabs of each server (title, database, text, which one was in front) are saved about 0.7 s
after the last change and on disconnect, in `workspaces.json`, and come back when you connect to the same
`user@host:port` ("Restored N tabs"). Restored tabs connect only when first used, so 20 tabs are not 20 connections.
Results are not saved. A fresh connection with one empty tab saves nothing. At most 50 tabs per server and 50 servers.
Closing all tabs forgets them.

**Version and title bar.** The sidebar footer reads `Slon <version> · PostgreSQL <server version>`; clicking it copies
those lines plus the OS and architecture, which is what a bug report needs. The copied text deliberately carries no host,
user or database name, so it can be pasted into a public issue. The version comes from `tauri.conf.json` through Tauri's
own package info, so the footer cannot drift from the version in the installer's file name, and a debug build says so.
There is no menu bar: a Help > About dialog would mean building that whole surface for one item. The title bar instead
carries `user@host:port — Slon` while connected and the app name alone when not, because which server a window talks
to is the thing you can get wrong. The native window title is set from Rust (`set_window_title`); the document title is
kept identical for the tools that read that instead.

**App icon.** Side-view elephant facing right on a blue tile. Source: `assets/icon.svg` (vector) and
`assets/icon-1024.png`; regenerate the set with `npx tauri icon assets/icon-1024.png`, then delete the
unused `android/`, `ios/`, `Square*` and `StoreLogo` files it also writes (only the files listed in
`tauri.conf.json` and `icon.png` are used).
