// End-to-end test of the real app through tauri-driver (WebDriver).
//
//   cargo install tauri-driver --locked          (once)
//   sudo apt install webkit2gtk-driver xvfb      (Linux)
//   cd .. && npx tauri build --debug --no-bundle
//   eval "$(../scripts/test-db.sh start)"
//   xvfb-run -a node run.mjs
//
// Needs the SLON_TEST_* variables from scripts/test-db.sh.
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { remote } from 'webdriverio';

const env = (k) => process.env[k] ?? (console.error(`missing ${k}`), process.exit(2));
const HOST = env('SLON_TEST_HOST');
const PORT = env('SLON_TEST_PORT');
const USER = env('SLON_TEST_USER');
const PASS = env('SLON_TEST_PASSWORD');
const DB = env('SLON_TEST_DB');
const APP = process.env.SLON_APP ?? new URL('../target/debug/slon', import.meta.url).pathname;
const SHOTS = process.env.SLON_SHOTS ?? '/tmp/e2e-shots';
mkdirSync(SHOTS, { recursive: true });

const results = [];
function check(name, ok, info = '') {
  results.push({ name, ok });
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok || !info ? '' : `\n        ${info}`}`);
}

// A private config folder, and passwords kept in memory: the test must never touch the
// real saved connections or the real credential store of whoever runs it.
const CONFIG_HOME = mkdtempSync(join(tmpdir(), 'slon-e2e-config-'));
// Where the app writes exports instead of opening a native save dialog (debug builds only).
const EXPORT_DIR = mkdtempSync(join(tmpdir(), 'slon-e2e-export-'));
const driver = spawn('tauri-driver', [], {
  stdio: ['ignore', 'inherit', 'inherit'],
  env: {
    ...process.env,
    XDG_CONFIG_HOME: CONFIG_HOME,
    // Fresh webview storage too: the form remembers the last connection in localStorage.
    XDG_DATA_HOME: mkdtempSync(join(tmpdir(), 'slon-e2e-data-')),
    XDG_CACHE_HOME: mkdtempSync(join(tmpdir(), 'slon-e2e-cache-')),
    SLON_MEMORY_SECRETS: '1',
    SLON_EXPORT_DIR: EXPORT_DIR,
  },
});
await sleep(1500);

const browser = await remote({
  hostname: '127.0.0.1',
  port: 4444,
  logLevel: 'warn',
  capabilities: { 'tauri:options': { application: APP } },
});

const ACTIVE = '.bodies > .body:not([hidden])';
const shot = (n) => browser.saveScreenshot(`${SHOTS}/${n}.png`);
const text = async (sel) => (await (await browser.$(sel)).getText()).replace(/\s+/g, ' ').trim();
const until = (fn, msg, timeout = 20000) =>
  browser.waitUntil(fn, { timeout, interval: 150, timeoutMsg: msg });
const click = async (sel) => (await browser.$(sel)).click();

// WebKitGTK's WebDriver drops doubled keystrokes ("::" arrives as ":"), so apart from one
// genuine typing check below, SQL goes in as a single text-input event (what a paste does).
async function setSql(sql) {
  // Focus by script: an open autocomplete popup can sit over the editor and swallow a click.
  await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .cm-content').focus());
  await browser.keys('Escape');
  await browser.keys(['Control', 'a']);
  await browser.execute((q) => document.execCommand('insertText', false, q), sql);
  await browser.keys('Escape');
}
const run = async (sql) => {
  await setSql(sql);
  await click(`${ACTIVE} .toolbar button.primary`);
};
const idle = () =>
  until(async () => (await (await browser.$(`${ACTIVE} .toolbar button.primary`)).isEnabled()), 'query did not finish');
const idleNow = async () => (await (await browser.$(`${ACTIVE} .toolbar button.primary`)).isEnabled());
const firstRowCells = () =>
  browser.execute(() =>
    [...document.querySelectorAll('.bodies > .body:not([hidden]) .grid .row')][0]
      ?.innerText.split('\n').map((s) => s.trim()).filter(Boolean),
  );

let failed = false;
try {
  /* ---------- connection form, error path first */
  await until(async () => (await text('h1')).includes('Connect to PostgreSQL'), 'no connect form');
  const fill = async (i, v) => (await browser.$$('input'))[i].setValue(v);
  // inputs: 0 name, 1 host, 2 port, 3 database, 4 user, 5 password, then two checkboxes
  await fill(1, HOST);
  await fill(2, PORT);
  await fill(3, DB);
  await fill(4, USER);
  await fill(5, 'definitely-wrong');
  await click('button.primary');
  await until(async () => (await browser.$('.error[role=alert]')).isExisting(), 'no error shown');
  const err = await text('.error[role=alert]');
  check('wrong password shows the server error', /password authentication failed/i.test(err) && err.includes('28P01'), err);
  await shot('01-connect-error');

  /* ---------- successful connect + tree */
  await fill(5, PASS);
  await click('button.primary');
  await until(async () => (await browser.$('.sidebar')).isExisting(), 'did not connect');
  check('connects with correct password', true);
  const dbRow = `//div[@role="treeitem"][contains(., "${DB}")][.//*[contains(@class,"label")][normalize-space()="${DB}"]]`;
  await until(async () => (await browser.$(dbRow)).isExisting(), 'database not listed in tree');
  check('tree lists databases lazily after connect', true);
  const footNow = await browser.execute(() => document.querySelector('[data-testid=about]')?.textContent ?? '');
  check('server version shown', /PostgreSQL \d+/.test(footNow), footNow);

  /* ---------- typed query with Ctrl+Enter */
  await click(`${ACTIVE} .cm-content`);
  await browser.keys('SELECT 40 + 2 AS answer');
  await browser.keys(['Control', 'Enter']);
  await idle();
  const c1 = await firstRowCells();
  check('typed query runs with Ctrl+Enter', c1?.includes('42'), JSON.stringify(c1));

  /* ---------- big result, sorting */
  await run("SELECT g AS n, md5(g::text) AS h, (g % 3 = 0) AS flag, NULLIF(g % 5, 0) AS maybe FROM generate_series(1, 2500) g");
  await idle();
  check('2,500 rows delivered', (await text(`${ACTIVE} .status`)).startsWith('2,500 rows'), await text(`${ACTIVE} .status`));
  await shot('02a-grid');
  const headerText = await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .grid .header')?.textContent);
  check('column types in header', /int4/.test(headerText ?? ''), JSON.stringify(headerText));
  const clickHead = (name) =>
    browser.execute((n) => {
      const h = [...document.querySelectorAll('.bodies > .body:not([hidden]) .grid .head')].find(
        (b) => b.querySelector('.name')?.textContent === n,
      );
      h.click();
    }, name);
  await clickHead('n');
  await clickHead('n');
  await sleep(300);
  let cells = await firstRowCells();
  check('descending numeric sort (2500 before 999)', cells?.[1] === '2500', JSON.stringify(cells));
  await clickHead('maybe');
  await sleep(300);
  cells = await firstRowCells();
  check('NULLs sort last ascending', cells?.[4] === '1', JSON.stringify(cells));
  await shot('02-grid-sorted');

  /* ---------- syntax error underline */
  await run('SELECT 1;\nSELEC 2');
  await idle();
  const banner = await text(`${ACTIVE} .banner`);
  check('syntax error banner with SQLSTATE', /syntax error/i.test(banner) && banner.includes('42601'), banner);
  check('error position underlined in editor', await (await browser.$(`${ACTIVE} .cm-lintRange-error`)).isExisting());
  await shot('03-syntax-error');

  /* ---------- cancel */
  await run('SELECT pg_sleep(60)');
  await until(async () => (await text(`${ACTIVE} .status`)).startsWith('Running'), 'never started running');
  const t0 = Date.now();
  await click(`${ACTIVE} .toolbar button:nth-of-type(2)`);
  await idle();
  const cancelBanner = await text(`${ACTIVE} .banner`);
  check('cancel stops a running query quickly', /cancel/i.test(cancelBanner) && Date.now() - t0 < 8000, `${cancelBanner} (${Date.now() - t0} ms)`);
  await run('SELECT 1 AS still_alive');
  await idle();
  check('tab is usable after cancel', (await firstRowCells())?.includes('1'));

  /* ---------- row cap */
  await run('SELECT g FROM generate_series(1, 150000) g');
  await idle();
  // textContent, not getText: WebKit's getText omits text inside ellipsized elements.
  const capStatus = (await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .status').textContent)).replace(/\s+/g, ' ');
  check('row cap: 100,000 rows with truncation warning', capStatus.startsWith('100,000 rows') && capStatus.includes('Showing only the first'), capStatus);
  await shot('04-truncated');

  /* ---------- tabs: independent connections */
  await browser.keys(['Control', 't']);
  await until(async () => (await browser.$$('.tab')).length === 2, 'second tab not opened');
  await run("SELECT current_setting('application_name') AS app, pg_backend_pid() AS pid");
  await idle();
  const cells2 = await firstRowCells(); // [row number, app, pid]
  const pid2 = cells2?.[2];
  await click('.tab:nth-of-type(1)');
  await run('SELECT pg_backend_pid() AS pid');
  await idle();
  const pid1 = (await firstRowCells())?.[1];
  check('each tab has its own backend connection', pid1 && pid2 && pid1 !== pid2, `${pid1} vs ${pid2}`);
  check('connections identify themselves as application_name=slon', cells2?.[1] === 'slon', JSON.stringify(cells2));

  /* ---------- browse a table from the tree */
  await run("DROP TABLE IF EXISTS public.e2e_demo; CREATE TABLE public.e2e_demo AS SELECT g AS id, 'row ' || g AS label FROM generate_series(1, 5) g; ALTER TABLE public.e2e_demo ADD PRIMARY KEY (id); CREATE INDEX e2e_demo_label_idx ON public.e2e_demo (label); COMMENT ON TABLE public.e2e_demo IS 'e2e demo'; CREATE SEQUENCE public.e2e_demo_ticket_seq START 50 OWNED BY public.e2e_demo.id; CREATE OR REPLACE FUNCTION public.e2e_touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$; CREATE TRIGGER e2e_demo_audit BEFORE INSERT OR UPDATE ON public.e2e_demo FOR EACH ROW EXECUTE FUNCTION public.e2e_touch(); CREATE TRIGGER e2e_demo_quiet AFTER DELETE ON public.e2e_demo FOR EACH STATEMENT EXECUTE FUNCTION public.e2e_touch(); ALTER TABLE public.e2e_demo DISABLE TRIGGER e2e_demo_quiet");
  await idle();
  await click('.sidebar .row .refresh');
  const treeRow = (label) => `//div[@role="treeitem"][.//span[contains(@class,"label")][normalize-space()="${label}"]]`;
  await until(async () => (await browser.$(treeRow(DB))).isExisting(), 'tree did not reload');
  await (await browser.$(treeRow(DB))).click();
  await until(async () => (await browser.$(treeRow('public'))).isExisting(), 'schemas not shown');
  await (await browser.$(treeRow('public'))).click();
  await until(async () => (await browser.$(`//div[@role="treeitem"][contains(., "Tables (")]`)).isExisting(), 'no Tables group');
  await (await browser.$(`//div[@role="treeitem"][contains(., "Tables (")]`)).click();
  await until(async () => (await browser.$(treeRow('e2e_demo'))).isExisting(), 'table not listed');
  check('tree drills down database > schema > tables', true);
  await shot('05-tree');
  await (await browser.$(treeRow('e2e_demo'))).doubleClick();
  await until(async () => (await browser.$$('.tab')).length === 3, 'browse tab not opened');
  await until(async () => (await text(`${ACTIVE} .status`)).startsWith('5 rows'), 'browse query did not run');
  const browseCells = await firstRowCells();
  check('double-click table opens a tab with its rows', browseCells?.includes('row 1'), JSON.stringify(browseCells));
  await shot('06-browse-table');


  /* ---------- table properties in the tree */
  const paste = async () => {
    await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .cm-content').focus());
    await browser.keys('Escape');
    await browser.keys(['Control', 'a']);
    await browser.keys(['Control', 'v']);
    await sleep(300);
    return browser.execute(() =>
      [...document.querySelectorAll('.bodies > .body:not([hidden]) .cm-line')].map((l) => l.textContent).join('\n'),
    );
  };

  // WebKit's getText omits ellipsized text, so read textContent.
  const content = async (sel) => (await browser.execute((el) => el.textContent.replace(/\s+/g, ' ').trim(), await browser.$(sel)));
  const itemByText = (t) => `//div[@role="treeitem"][contains(., "${t}")]`;
  await (await browser.$(`${treeRow('e2e_demo')}//span[contains(@class,"chev")]`)).click();
  await until(async () => (await browser.$(itemByText('Columns ('))).isExisting(), 'table did not expand');
  await until(async () => (await browser.$(treeRow('label'))).isExisting(), 'columns not listed');
  const colRow = await content(treeRow('id'));
  check('expanding a table lists Columns (2) with types and PK badge', (await content(itemByText('Columns ('))).includes('Columns (2)') && colRow.includes('integer') && colRow.includes('PK'), colRow);
  await (await browser.$(itemByText('Indexes ('))).click();
  await (await browser.$(itemByText('Constraints ('))).click();
  await until(async () => (await browser.$(treeRow('e2e_demo_label_idx'))).isExisting(), 'indexes not listed');
  const pkRow = await content(treeRow('e2e_demo_pkey'));
  check('indexes and constraints are listed with their definitions', pkRow.length > 0 && (await content(treeRow('e2e_demo_label_idx'))).includes('btree (label)'), pkRow);
  await (await browser.$(itemByText('Triggers ('))).click();
  await (await browser.$(itemByText('Sequences ('))).click();
  await until(async () => (await browser.$(treeRow('e2e_demo_ticket_seq'))).isExisting(), 'sequences not listed');
  const trigRow = await content(treeRow('e2e_demo_audit'));
  check('triggers are listed with timing, events and level', trigRow.includes('BEFORE INSERT OR UPDATE') && trigRow.includes('ROW'), trigRow);
  const quietRow = await content(treeRow('e2e_demo_quiet'));
  check('a disabled trigger is flagged', quietRow.includes('disabled') && quietRow.includes('AFTER DELETE') && quietRow.includes('STATEMENT'), quietRow);
  const seqRow = await content(treeRow('e2e_demo_ticket_seq'));
  check('sequences are listed with their column and type', seqRow.includes('id') && seqRow.includes('bigint'), seqRow);
  await shot('14-table-properties');

  await (await browser.$(treeRow('e2e_demo'))).click({ button: 'right' });
  await until(async () => (await browser.$('[data-testid=context-menu]')).isExisting(), 'no context menu on the table');
  const menuText = await text('[data-testid=context-menu]');
  check('table context menu offers browse, SELECT, CREATE script, copy name', ['Browse first 1000 rows', 'Open SELECT in new tab', 'Copy CREATE script', 'Open CREATE script in new tab', 'Copy name'].every((t) => menuText.includes(t)), menuText);
  await shot('15-context-menu');
  await (await browser.$('//*[@data-testid="context-menu"]//button[contains(., "Copy CREATE script")]')).click();
  await until(async () => (await browser.$('[data-testid=notice]')).isExisting(), 'no copy notice');
  check('copy CREATE script is confirmed', (await content('[data-testid=notice]')).includes('Copied CREATE script'), await content('[data-testid=notice]'));
  const ddlText = await paste();
  check('the copied script describes the table', ddlText.includes('CREATE TABLE public.e2e_demo') && ddlText.includes('PRIMARY KEY (id)') && ddlText.includes("COMMENT ON TABLE public.e2e_demo IS 'e2e demo'") && ddlText.includes('CREATE INDEX e2e_demo_label_idx'), ddlText);
  check('the script also carries triggers and sequences', ddlText.includes('CREATE SEQUENCE public.e2e_demo_ticket_seq') && ddlText.includes('OWNED BY public.e2e_demo.id') && ddlText.includes('CREATE TRIGGER e2e_demo_audit') && ddlText.includes('DISABLE TRIGGER e2e_demo_quiet'), ddlText);

  await (await browser.$(treeRow('e2e_demo'))).click({ button: 'right' });
  await (await browser.$('//*[@data-testid="context-menu"]//button[contains(., "Open CREATE script in new tab")]')).click();
  await until(async () => (await browser.$$('.tab')).length === 4, 'DDL tab not opened');
  const ddlTab = await browser.execute(() => document.querySelector('.tab.active .title')?.textContent);
  check('CREATE script opens in its own tab', ddlTab === 'e2e_demo DDL', String(ddlTab));
  await (await browser.$(treeRow('e2e_demo'))).click({ button: 'right' });
  await (await browser.$('//*[@data-testid="context-menu"]//button[contains(., "Open SELECT in new tab")]')).click();
  await until(async () => (await browser.$$('.tab')).length === 5, 'SELECT tab not opened');
  const selText = await browser.execute(() => [...document.querySelectorAll('.bodies > .body:not([hidden]) .cm-line')].map((l) => l.textContent).join('\n'));
  check('SELECT template lists the columns', selText.includes('SELECT') && selText.includes('    id,') && selText.includes('FROM public.e2e_demo'), selText);

  /* ---------- autocomplete knows the schema */
  await click('.tab:nth-of-type(1)'); // this tab ran the CREATE TABLE: names must have refreshed on their own
  const typeAndComplete = async (sql) => {
    await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .cm-content').focus());
    await browser.keys('Escape');
    await browser.keys(['Control', 'a']);
    await browser.execute((q) => document.execCommand('insertText', false, q), sql);
    await sleep(900);
    return browser.execute(() => document.querySelector('.cm-tooltip-autocomplete')?.textContent ?? '');
  };
  let popup = await typeAndComplete('SELECT * FROM e2e_d');
  check('table names are completed (and picked up after CREATE TABLE)', popup.includes('e2e_demo'), popup);
  await shot('10-complete-table');
  popup = await typeAndComplete('SELECT * FROM e2e_demo WHERE la');
  check('column names of the FROM table are completed', popup.includes('label') && popup.includes('text'), popup);
  await shot('11-complete-column');
  popup = await typeAndComplete('SELECT * FROM e2e_demo d WHERE d.');
  check('alias.column completes the aliased table columns', popup.includes('label') && popup.includes('id'), popup);
  popup = await typeAndComplete('SELECT * FROM public.e2e_d');
  check('schema-qualified table names are completed', popup.includes('e2e_demo'), popup);
  popup = await typeAndComplete('SELECT * FROM no_such_table WHERE la');
  check('unknown tables offer no invented columns', !popup.includes('e2e_demo'), popup);
  await browser.keys('Escape');

  /* ---------- grid: selection and copy */
  const GRID = `${ACTIVE} .grid`;
  await run("SELECT g AS n, 'v' || g AS s, NULLIF(g, 2) AS maybe FROM generate_series(1, 5) g");
  await idle();
  await click(`${GRID} .row:nth-child(2) .cell:nth-child(3)`); // row 2, column "s"
  await browser.keys(['Control', 'c']);
  await until(async () => (await browser.$('[data-testid=copy-toast]')).isExisting(), 'no copy confirmation');
  check('Ctrl+C reports what was copied', (await text('[data-testid=copy-toast]')).includes('1 cell'), await text('[data-testid=copy-toast]'));
  const one = await paste();
  check('a single cell is copied verbatim', one === 'v2', JSON.stringify(one));

  await click(`${GRID} .row:nth-child(2) .cell:nth-child(2)`);
  await browser.keys(['Shift', 'ArrowDown']);
  await browser.keys(['Shift', 'ArrowDown']);
  await browser.keys(['Shift', 'ArrowRight']);
  await browser.keys(['Shift', 'ArrowRight']);
  const selCount = await browser.execute((g) => document.querySelectorAll(`${g} .cell.sel`).length, GRID);
  check('shift+arrows extend the selection to a 3x3 block', selCount === 9, String(selCount));
  await shot('12-selection');
  await browser.keys(['Control', 'c']);
  await sleep(200);
  const block = await paste();
  check('a block is copied as tab-separated rows with NULL empty', block === '2\tv2\t\n3\tv3\t3\n4\tv4\t4', JSON.stringify(block));

  await click(`${GRID} .row:nth-child(1) .cell.num`); // row-number click selects the whole row
  await browser.keys(['Control', 'Shift', 'c']);
  await sleep(200);
  const withHead = await paste();
  check('row-number click + Ctrl+Shift+C copies the row with headers', withHead === 'n\ts\tmaybe\n1\tv1\t1', JSON.stringify(withHead));

  await click(`${GRID} .row:nth-child(1) .cell:nth-child(2)`);
  await browser.keys(['Control', 'a']);
  const allSel = await browser.execute((g) => document.querySelectorAll(`${g} .cell.sel`).length, GRID);
  check('Ctrl+A selects every cell', allSel === 15, String(allSel));

  await (await browser.$(`${GRID} .row:nth-child(2) .cell:nth-child(3)`)).click({ button: 'right' });
  await until(async () => (await browser.$('[data-testid=ctx-copy-headers]')).isExisting(), 'no context menu');
  check('right-click offers Copy and Copy with headers', await (await browser.$('[data-testid=ctx-copy]')).isExisting());
  await click('[data-testid=ctx-copy-headers]');
  await sleep(200);
  const viaMenu = await paste();
  check('context menu copy works', viaMenu.startsWith('n\ts\tmaybe') , JSON.stringify(viaMenu));

  await browser.releaseActions();

  /* ---------- grid: resizable columns */
  await run("SELECT 1 AS id, repeat('x', 120) AS wide");
  await idle();
  const headWidth = (i) =>
    browser.execute((g, k) => document.querySelectorAll(`${g} .hcell`)[k].getBoundingClientRect().width, GRID, i);
  const w0 = await headWidth(0);
  const handle = await browser.$(`${GRID} .hcell:nth-child(2) .resizer`);
  // Several small moves with pauses: a single jump can be coalesced by the driver.
  const dragBy = async (el, dx) => {
    const steps = 8;
    let a = browser.action('pointer').move({ origin: el }).down().pause(60);
    for (let i = 0; i < steps; i++) a = a.move({ origin: 'pointer', x: Math.round(dx / steps), y: 0, duration: 40 }).pause(30);
    await a.up().perform();
    await browser.releaseActions();
  };
  await dragBy(handle, 72);
  const w1 = await headWidth(0);
  check('dragging the border widens the column', Math.abs(w1 - (w0 + 72)) <= 4, `${w0} -> ${w1}`);
  const bodyCellW = await browser.execute((g) => document.querySelector(`${g} .row .cell:nth-child(2)`).getBoundingClientRect().width, GRID);
  check('cells follow the header width', Math.abs(bodyCellW - w1) <= 1, `${bodyCellW} vs ${w1}`);
  await dragBy(await browser.$(`${GRID} .hcell:nth-child(2) .resizer`), -400);
  check('a column cannot be dragged below the minimum', (await headWidth(0)) >= 40, String(await headWidth(0)));
  const wideBefore = await headWidth(1);
  await (await browser.$(`${GRID} .hcell:nth-child(3) .resizer`)).doubleClick();
  await sleep(200);
  const wideAfter = await headWidth(1);
  check('double-click on the border fits the column to its content', wideAfter > wideBefore && wideAfter > 600, `${wideBefore} -> ${wideAfter}`);
  await shot('13-resized');
  await run('SELECT 1 AS id, repeat(\'x\', 120) AS wide');
  await idle();
  check('a new result starts with automatic widths again', Math.abs((await headWidth(1)) - wideBefore) <= 1, `${await headWidth(1)} vs ${wideBefore}`);

  /* ---------- explain view */
  await run('DROP TABLE IF EXISTS e2e_plan; CREATE TABLE e2e_plan AS SELECT g AS n, md5(g::text) AS h FROM generate_series(1, 5000) g; ANALYZE e2e_plan');
  await idle();
  const explainWith = async (sql, button) => {
    await setSql(sql);
    await click(`${ACTIVE} [data-testid=${button}]`);
    await idle();
  };
  const planLabels = () =>
    browser.execute(() =>
      [...document.querySelectorAll('.bodies > .body:not([hidden]) [data-testid=plan-label]')].map((e) => e.textContent.trim()),
    );
  await explainWith('SELECT * FROM e2e_plan WHERE n = 7;', 'explain');
  await until(async () => (await browser.$(`${ACTIVE} [data-testid=plan-view]`)).isExisting(), 'plan view did not appear');
  let labels = await planLabels();
  check('Explain shows the plan tree', labels.length === 1 && labels[0] === 'Seq Scan on e2e_plan', JSON.stringify(labels));
  check('estimated plan says so', /estimated total cost/.test(await text(`${ACTIVE} [data-testid=plan-summary]`)) && /Plan \(estimated\)/.test(await text(`${ACTIVE} [data-testid=view-plan]`)));
  await shot('19-explain');

  await explainWith('DELETE FROM e2e_plan WHERE n <= 4000', 'explain-analyze');
  labels = await planLabels();
  check('Explain analyze shows the whole tree', labels[0] === 'Delete on e2e_plan' && labels[1] === 'Seq Scan on e2e_plan', JSON.stringify(labels));
  const sum = await text(`${ACTIVE} [data-testid=plan-summary]`);
  check('analysed plan reports execution and planning time', /execution/.test(sum) && /planning/.test(sum), sum);
  await click(`${ACTIVE} [data-testid=plan-node]:nth-child(2) .row`);
  await until(async () => (await browser.$(`${ACTIVE} [data-testid=plan-detail]`)).isExisting(), 'node details did not open');
  const detail = await text(`${ACTIVE} [data-testid=plan-detail]`);
  check('a node opens its details (filter, rows, time)', /Filter/.test(detail) && /n <= 4000/.test(detail) && /actual/.test(detail) && /per loop/.test(detail), detail);
  await sleep(500);
  check('opened node details stay open', await (await browser.$(`${ACTIVE} [data-testid=plan-detail]`)).isExisting());
  await shot('20-explain-analyze');

  await run('SELECT count(*) AS left_over FROM e2e_plan');
  await idle();
  check('Explain analyze rolled the DELETE back', (await firstRowCells())?.includes('5000'), JSON.stringify(await firstRowCells()));
  check('results and plan are both kept, results shown after a run', await (await browser.$(`${ACTIVE} .grid`)).isExisting() && await (await browser.$(`${ACTIVE} [data-testid=view-plan]`)).isExisting());
  await click(`${ACTIVE} [data-testid=view-plan]`);
  check('the plan tab brings the plan back', (await planLabels()).length === 2);
  await click(`${ACTIVE} [data-testid=view-results]`);

  await explainWith('SELECT nope FROM e2e_plan', 'explain');
  const explainErr = await text(`${ACTIVE} .banner`);
  check('Explain reports a bad statement like Run does', explainErr.includes('nope') && explainErr.includes('42703'), explainErr);
  await explainWith('SELECT 1; SELECT 2', 'explain');
  check('Explain refuses scripts', /one statement/.test(await text(`${ACTIVE} .banner`)), await text(`${ACTIVE} .banner`));

  await setSql('SELECT pg_sleep(60)');
  await click(`${ACTIVE} [data-testid=explain-analyze]`);
  await until(async () => (await text(`${ACTIVE} .status`)).startsWith('Running'), 'explain analyze never started');
  const tx0 = Date.now();
  await click(`${ACTIVE} .toolbar button:nth-of-type(2)`);
  await idle();
  check('Explain analyze can be cancelled', /cancel/i.test(await text(`${ACTIVE} .banner`)) && Date.now() - tx0 < 8000, await text(`${ACTIVE} .banner`));
  await run('DROP TABLE e2e_plan');
  await idle();
  check('tab still works after explain', !(await (await browser.$(`${ACTIVE} .banner`)).isExisting()));

  /* ---------- server messages (RAISE NOTICE) */
  const msgRows = () =>
    browser.execute(() =>
      [...document.querySelectorAll('.bodies > .body:not([hidden]) [data-testid=message-row]')].map((r) => ({
        sev: r.querySelector('[data-testid=message-severity]').textContent.trim(),
        text: r.querySelector('[data-testid=message-text]').textContent.trim(),
        all: r.textContent.replace(/\s+/g, ' ').trim(),
      })),
    );
  await run("DO $$ BEGIN RAISE NOTICE 'hello from e2e'; RAISE WARNING 'careful' USING HINT = 'mind the hint'; END $$");
  await idle();
  await until(async () => (await browser.$(`${ACTIVE} [data-testid=messages-view]`)).isExisting(), 'messages view did not open for a statement that only talks');
  await until(async () => (await msgRows()).length === 2 && (await idleNow()), 'both messages did not arrive');
  let mrows = await msgRows();
  check('a statement without rows opens the Messages view', mrows.length === 2 && mrows[0].sev === 'NOTICE' && mrows[0].text === 'hello from e2e' && mrows[1].sev === 'WARNING', JSON.stringify(mrows));
  check('hint is shown with the message', mrows[1].all.includes('mind the hint'), mrows[1].all);
  check('tab badge counts the messages and flags warnings', (await text(`${ACTIVE} [data-testid=messages-badge]`)) === '2' && (await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) [data-testid=messages-badge]').classList.contains('warn'))));
  // textContent: WebKitGTK's getText leaves out text that is cut off with an ellipsis.
  const statusText = await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .status').textContent);
  check('status bar still says the command completed', statusText.includes('Command completed successfully'), statusText);
  await shot('21-messages');

  await run("SELECT 7 AS seven; DO $$ BEGIN RAISE NOTICE 'with rows'; END $$; SELECT 8 AS eight");
  await idle();
  check('a script that returns rows stays on the grid and counts messages', (await firstRowCells())?.includes('8') && (await text(`${ACTIVE} [data-testid=messages-badge]`)) === '1');
  await click(`${ACTIVE} [data-testid=view-messages]`);
  check('the Messages tab shows them', (await msgRows())[0]?.text === 'with rows');
  await click(`${ACTIVE} [data-testid=view-results]`);

  await run("DO $$ BEGIN RAISE NOTICE 'first'; PERFORM pg_sleep(2.5); RAISE NOTICE 'second'; END $$");
  await until(async () => (await msgRows()).length === 1, 'the first message did not show up while the statement was still running');
  const stillRunning = !(await (await browser.$(`${ACTIVE} .toolbar button.primary`)).isEnabled());
  check('messages appear while the statement is still running', stillRunning);
  await idle();
  check('and the rest follow', (await msgRows()).map((r) => r.text).join() === 'first,second');

  await run("DO $$ BEGIN RAISE NOTICE 'before boom'; PERFORM 1/0; END $$");
  await idle();
  check('a failing statement keeps the messages raised before it failed', /22012/.test(await text(`${ACTIVE} .banner`)) && (await text(`${ACTIVE} [data-testid=messages-badge]`)) === '1');

  await run('SELECT 1 AS quiet');
  await idle();
  check('the next run starts without old messages', !(await (await browser.$(`${ACTIVE} [data-testid=messages-badge]`)).isExisting()));

  /* ---------- resizable sidebar */
  const sidebarW = () => browser.execute(() => Math.round(document.querySelector('.sidebar').getBoundingClientRect().width));
  const mainLeft = () => browser.execute(() => Math.round(document.querySelector('main').getBoundingClientRect().left));
  const storedW = () => browser.execute(() => localStorage.getItem('slon.sidebarWidth'));
  const splitter = '[data-testid=sidebar-splitter]';
  const sw0 = await sidebarW();
  check('sidebar starts at its default width', sw0 === 290, String(sw0));
  await dragBy(await browser.$(splitter), 110);
  const sw1 = await sidebarW();
  check('dragging the splitter widens the sidebar', Math.abs(sw1 - (sw0 + 110)) <= 4, `${sw0} -> ${sw1}`);
  check('the query area follows the sidebar edge', Math.abs((await mainLeft()) - sw1) <= 2, `${await mainLeft()} vs ${sw1}`);
  check('the new width is remembered', (await storedW()) === String(sw1), String(await storedW()));
  await shot('22-sidebar-wide');

  await dragBy(await browser.$(splitter), -(sw1 - 40)); // as far left as the window allows
  check('the sidebar cannot be made narrower than the minimum', (await sidebarW()) === 180, String(await sidebarW()));
  await dragBy(await browser.$(splitter), 800);
  const swMax = await sidebarW();
  const winW = await browser.execute(() => window.innerWidth);
  check('nor so wide that the query area disappears', swMax <= winW - 360 && swMax > 300, `${swMax} of ${winW}`);

  await (await browser.$(splitter)).doubleClick();
  await sleep(150);
  check('double-click restores the default width', (await sidebarW()) === 290, String(await sidebarW()));

  await browser.execute((sel) => document.querySelector(sel).focus(), splitter);
  await browser.keys('ArrowRight');
  await browser.keys('ArrowRight');
  check('arrow keys resize in steps of 16 px', (await sidebarW()) === 322, String(await sidebarW()));
  check('keyboard changes are remembered', (await storedW()) === '322', String(await storedW()));
  await browser.keys('Home');
  check('Home goes to the minimum', (await sidebarW()) === 180, String(await sidebarW()));
  const nowValue = await (await browser.$(splitter)).getAttribute('aria-valuenow');
  check('the separator reports its width to assistive tech', nowValue === '180', String(nowValue));
  await browser.keys('Enter');
  check('Enter resets to the default', (await sidebarW()) === 290, String(await sidebarW()));

  /* ---------- export of results */
  const exportedFiles = () => readdirSync(EXPORT_DIR).sort();
  const clearExports = () => exportedFiles().forEach((f) => rmSync(join(EXPORT_DIR, f)));
  const doExport = async (configure) => {
    clearExports();
    await click(`${ACTIVE} [data-testid=export]`);
    await until(async () => (await browser.$('[data-testid=export-dialog]')).isExisting(), 'export dialog did not open');
    if (configure) await configure();
    await click('[data-testid=export-save]');
    await until(async () => !(await (await browser.$('[data-testid=export-dialog]')).isExisting()), 'export dialog did not close');
    const files = exportedFiles();
    return files.length === 1 ? { name: files[0], body: readFileSync(join(EXPORT_DIR, files[0]), 'utf8') } : { name: files.join(','), body: '' };
  };
  const choose = (testid, value) => async () => (await browser.$(`[data-testid=${testid}]`)).selectByAttribute('value', value);

  await run(`SELECT 2 AS id, E'two\nlines' AS "t,x", NULL AS n, '' AS e, 'say "hi"' AS q, 'čćšđž' AS c, '{"a": 1}'::jsonb AS j, true AS b, 12345678901234567890.5::numeric AS big
             UNION ALL SELECT 1, 'plain', 'v', 'w', 'z', 'ž', NULL, false, 0.10`);
  await idle();
  check('Export is available for a result with rows', await (await browser.$(`${ACTIVE} [data-testid=export]`)).isEnabled());
  const csv = await doExport();
  check('CSV file is named result-<time>.csv', /^result-\d{8}-\d{6}\.csv$/.test(csv.name), csv.name);
  check('CSV starts with a UTF-8 byte order mark', csv.body.charCodeAt(0) === 0xfeff);
  const lines = csv.body.slice(1).split('\r\n');
  check('CSV header quotes names that need it', lines[0] === 'id,"t,x",n,e,q,c,j,b,big', lines[0]);
  check('CSV quotes embedded quotes and line breaks, NULL is empty, empty string is ""',
    csv.body.includes('2,"two\nlines",,"","say ""hi""",čćšđž,"{""a"": 1}",t,12345678901234567890.5\r\n'), JSON.stringify(csv.body));
  check('CSV keeps arrival order and ends with a line break', lines[2].startsWith('1,plain,v,w,z,ž,,f,0.10') && lines.at(-1) === '', JSON.stringify(lines));

  const semi = await doExport(async () => {
    await choose('export-delimiter', ';')();
    await click('[data-testid=export-header]');
    await click('[data-testid=export-bom]');
  });
  check('semicolon CSV without header and byte order mark', semi.name.endsWith('.csv') && semi.body.startsWith('2;') && !semi.body.includes('id;'), JSON.stringify(semi.body.slice(0, 40)));
  const tsv = await doExport(choose('export-delimiter', '\t'));
  check('tab delimiter gives a .tsv file', tsv.name.endsWith('.tsv') && tsv.body.startsWith('2\t'), tsv.name);

  const json = await doExport(choose('export-format', 'json'));
  let parsed = null;
  try { parsed = JSON.parse(json.body); } catch (e) { parsed = String(e); }
  check('JSON export parses as an array of objects', Array.isArray(parsed) && parsed.length === 2, json.body.slice(0, 200));
  check('JSON keeps NULL, booleans, embedded json and exact numbers',
    parsed?.[0]?.n === null && parsed[0].b === true && parsed[0].j?.a === 1 && parsed[0].e === '' && json.body.includes('"big": 12345678901234567890.5') || json.body.includes('"big":12345678901234567890.5'), json.body.slice(0, 400));
  const jl = await doExport(choose('export-format', 'jsonl'));
  check('JSON Lines has one object per line', jl.name.endsWith('.jsonl') && jl.body.trim().split('\n').length === 2 && JSON.parse(jl.body.split('\n')[1]).id === 1, jl.body);

  // The dialog remembers its settings.
  await click(`${ACTIVE} [data-testid=export]`);
  await until(async () => (await browser.$('[data-testid=export-dialog]')).isExisting(), 'dialog did not reopen');
  check('the dialog remembers the last format', (await (await browser.$('[data-testid=export-format]')).getValue()) === 'jsonl');
  await browser.keys('Escape');
  await until(async () => !(await (await browser.$('[data-testid=export-dialog]')).isExisting()), 'Escape did not close the dialog');

  // Export follows the sort order shown in the grid.
  await clickHead('id');
  await sleep(200);
  // (settings are remembered: tab delimiter, no header, no byte order mark from the checks above)
  const plainCsv = async () => {
    await choose('export-format', 'csv')();
    await choose('export-delimiter', ',')();
    await click('[data-testid=export-header]');
    await click('[data-testid=export-bom]');
  };
  const sorted = await doExport(plainCsv);
  check('export follows the sorted order of the grid', sorted.body.slice(1).split('\r\n')[1].startsWith('1,plain'), JSON.stringify(sorted.body));

  // Many rows are written in several chunks and arrive complete.
  await run('SELECT g AS n, repeat(\'x\', 20) AS pad FROM generate_series(1, 12345) g');
  await idle();
  const big = await doExport();
  // header and byte order mark are on again, remembered from the previous export
  const bigLines = big.body.slice(1).split('\r\n');
  check('a 12,345-row result is written completely', bigLines.length === 12347 && bigLines[12345] === '12345,xxxxxxxxxxxxxxxxxxxx', `${bigLines.length}`);

  await run('SELECT 1 WHERE false');
  await idle();
  check('Export is disabled when there are no rows', !(await (await browser.$(`${ACTIVE} [data-testid=export]`)).isEnabled()));

  /* ---------- query history */
  const historyEntries = () =>
    browser.execute(() => [...document.querySelectorAll('[data-testid=history-entry]')].map((e) => ({
      text: e.textContent.replace(/\s+/g, ' ').trim(),
      failed: e.classList.contains('failed'),
    })));
  const openHistory = async () => {
    if (!(await (await browser.$('[data-testid=history-panel]')).isExisting())) await click('[data-testid=history-open]');
    await until(async () => (await browser.$('[data-testid=history-panel]')).isExisting(), 'history panel did not open');
  };
  const searchHistory = async (q) => {
    const input = await browser.$('[data-testid=history-search]');
    await input.setValue(q || ' '); // blank is trimmed away: no filter
    await sleep(500); // debounce + round trip
  };
  const editorText = () => browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .cm-content').textContent);
  const tabCount = () => browser.execute(() => document.querySelectorAll('.tabs .tab').length);

  await run(`SELECT 'hist-marker-one' AS m`);
  await idle();
  await run(`SELECT 'hist-fail-marker', no_such_col`);
  await idle();
  await run(`SELECT 'hist-secret' AS m -- ALTER ROLE x PASSWORD 'hunter2'`);
  await idle();

  await openHistory();
  await searchHistory('hist-');
  let he = await historyEntries();
  check('history lists the statements just run, newest first', he.length === 2 && he[0].text.includes('hist-fail-marker') && he[1].text.includes('hist-marker-one'), JSON.stringify(he));
  check('a failed statement is marked and shows the server error', he[0]?.failed && /no_such_col/.test(he[0].text) && /does not exist/.test(he[0].text), he[0]?.text);
  check('a succeeded statement shows rows and duration', he[1]?.text.includes('1 row') && /\d+ ms/.test(he[1].text), he[1]?.text);
  await searchHistory('hist-secret');
  check('a statement containing a password is not recorded', (await historyEntries()).length === 0);
  check('the empty list explains itself', (await text('[data-testid=history-empty]')).includes('hist-secret'));
  await shot('23-history');

  await searchHistory('hist-marker');
  const tabsBefore = await tabCount();
  await click('[data-testid=history-new-tab]');
  await until(async () => (await tabCount()) === tabsBefore + 1, 'no new tab from history');
  check('"New tab" opens the statement in a new tab and closes the panel',
    (await editorText()).includes(`SELECT 'hist-marker-one'`) && !(await (await browser.$('[data-testid=history-panel]')).isExisting()));

  await setSql('SELECT 42');
  await openHistory();
  await searchHistory('hist-marker');
  await click('[data-testid=history-use]');
  await sleep(300);
  check('"Use here" replaces the text of the current tab', (await editorText()) === `SELECT 'hist-marker-one' AS m` && (await tabCount()) === tabsBefore + 1, await editorText());

  await openHistory();
  await searchHistory('hist-marker');
  await click('[data-testid=history-delete]');
  await sleep(400);
  check('an entry can be removed', (await historyEntries()).length === 0);

  await click('[data-testid=history-record]');
  await browser.keys('Escape');
  await run(`SELECT 'hist-off-marker' AS m`);
  await idle();
  await openHistory();
  await searchHistory('hist-off');
  check('nothing is recorded while recording is switched off', (await historyEntries()).length === 0);
  await click('[data-testid=history-record]');

  await searchHistory('');
  check('the unfiltered history is not empty', (await historyEntries()).length > 3);
  await click('[data-testid=history-clear]');
  check('clearing asks for a second click', (await text('[data-testid=history-clear]')).includes('Click again'));
  await click('[data-testid=history-clear]');
  await sleep(400);
  check('clearing the history empties it', (await historyEntries()).length === 0);
  const historyFile = readdirSync(CONFIG_HOME, { recursive: true }).find((f) => String(f).endsWith('history.json'));
  check('the history file never saw the password', !readFileSync(join(CONFIG_HOME, String(historyFile)), 'utf8').includes('hunter2'));
  await browser.keys('Escape');
  await until(async () => !(await (await browser.$('[data-testid=history-panel]')).isExisting()), 'Escape did not close the history');

  /* ---------- tabs come back after reconnecting */
  await setSql('SELECT 1 AS restored_a');
  await click('.tabs .add');
  await until(async () => (await idleNow()), 'new tab not ready');
  await setSql('SELECT 2 AS restored_b');
  const snapshot = () =>
    browser.execute(() => ({
      titles: [...document.querySelectorAll('.tabs .tab .title')].map((t) => t.textContent),
      texts: [...document.querySelectorAll('.bodies > .body')].map((b) => b.querySelector('.cm-content').textContent),
      active: [...document.querySelectorAll('.tabs .tab')].findIndex((t) => t.classList.contains('active')),
    }));
  const before = await snapshot();
  await sleep(1200); // saved shortly after the last change
  await click('.sidebar .head button');
  await until(async () => (await text('h1')).includes('Connect to PostgreSQL'), 'did not return to the connect form');
  await (await browser.$('[data-testid=password]')).setValue(PASS);
  await click('button.primary');
  await until(async () => (await browser.$('.sidebar')).isExisting(), 'could not reconnect');
  await until(async () => (await tabCount()) === before.titles.length, 'tabs were not restored');
  const after = await snapshot();
  check('tab titles, texts and the active tab are restored', JSON.stringify(after) === JSON.stringify(before), `${JSON.stringify(before)}\n${JSON.stringify(after)}`);
  const toast = await browser.execute(() => document.querySelector('[data-testid=notice]')?.textContent ?? null);
  check('restored tabs are announced', String(toast).startsWith('Restored'), String(toast));
  await run('SELECT 2 AS restored_b');
  await idle();
  check('a restored tab connects when it is first used', (await firstRowCells())?.includes('2'), JSON.stringify(await firstRowCells()));
  const wsFile = readdirSync(CONFIG_HOME, { recursive: true }).find((f) => String(f).endsWith('workspaces.json'));
  check('remembered tabs are kept per server in the config folder', !!wsFile && readFileSync(join(CONFIG_HOME, String(wsFile)), 'utf8').includes('restored_b'));

  /* ---------- version in the footer, connection in the title bar */
  const footText = () => browser.execute(() => document.querySelector('[data-testid=about]')?.textContent.replace(/\s+/g, ' ').trim());
  const foot = await footText();
  check('the footer names the build and the server version', /^Slon \d+\.\d+\.\d+ \(debug\) · PostgreSQL \d+/.test(foot), String(foot));
  const titleConnected = await browser.getTitle();
  check('the title bar carries the connection', titleConnected === `${USER}@${HOST}:${PORT} — Slon`, titleConnected);

  await click('[data-testid=about]');
  await until(async () => (await browser.$('[data-testid=notice]')).isExisting(), 'no copy feedback');
  const copied = await browser.execute(() => document.querySelector('[data-testid=notice]')?.textContent);
  check('clicking the footer copies the version details', String(copied).includes('Copied version details'), String(copied));

  /* ---------- disconnect */
  await click('.sidebar .head button');
  await until(async () => (await text('h1')).includes('Connect to PostgreSQL'), 'did not return to connect form');
  check('disconnect returns to the connect form', true);
  await until(async () => (await browser.getTitle()) === 'Slon', 'title did not go back to the app name');
  check('the title bar drops the connection on disconnect', true);

  /* ---------- saved connections */
  const profilesFile = () => {
    const files = readdirSync(CONFIG_HOME, { recursive: true }).filter((f) => String(f).endsWith('profiles.json'));
    return files.length ? readFileSync(join(CONFIG_HOME, String(files[0])), 'utf8') : null;
  };
  const connectedNow = async () => (await browser.$('.sidebar')).isExisting();
  const disconnect = async () => {
    await click('.sidebar .head button');
    await until(async () => (await text('h1')).length > 0 && !(await connectedNow()), 'did not return to the connect form');
  };
  const profileRows = () => browser.execute(() => [...document.querySelectorAll('[data-testid=profile]')].map((b) => b.textContent.replace(/\s+/g, ' ').trim()));
  const tick = async (id, want) => {
    const box = await browser.$(`[data-testid=${id}]`);
    if ((await box.isSelected()) !== want) await box.click();
  };
  const placeholder = () => browser.execute(() => document.querySelector('[data-testid=password]').placeholder);

  check('no saved connections to begin with', (await profileRows()).length === 0);
  check('nothing is written before the user saves', profilesFile() === null);
  await fill(0, 'e2e server');
  await fill(5, PASS);
  await tick('save-connection', true);
  await tick('remember-password', true);
  await shot('07-save-form');
  await click('button.primary');
  await until(connectedNow, 'did not connect while saving');
  const saved = profilesFile();
  check('profile is written to profiles.json', !!saved && saved.includes('e2e server') && saved.includes(HOST), String(saved));
  check('the password is not in profiles.json', !!saved && !saved.includes(PASS), String(saved));
  check('the file marks the password as saved', !!saved && saved.includes('"hasPassword": true'), String(saved));
  await disconnect();

  let rows = await profileRows();
  check('saved connection is listed after disconnect', rows.length === 1 && rows[0].includes('e2e server'), JSON.stringify(rows));
  check('latest profile is preselected with its saved password', (await placeholder()).includes('Saved'), await placeholder());
  check('fields are filled from the profile', (await (await browser.$$('input'))[1].getValue()) === HOST);
  await shot('08-saved-list');
  await click('button.primary'); // blank password: the saved one must be used
  await until(connectedNow, 'saved password was not used');
  check('connects with the saved password, none typed', true);
  await disconnect();

  // Edited host: the saved password must NOT follow the user to a different server.
  const otherHost = HOST === 'localhost' ? '127.0.0.1' : 'localhost';
  await tick('save-connection', false);
  await fill(1, otherHost);
  await click('button.primary');
  await until(async () => (await browser.$('.error[role=alert]')).isExisting(), 'no error for changed host');
  const leakErr = await text('.error[role=alert]');
  check('saved password is not sent to an edited host', !(await connectedNow()) && /password/i.test(leakErr), leakErr);

  // Forget the password, keep the profile.
  await fill(1, HOST);
  await tick('save-connection', true);
  await tick('remember-password', false);
  await click('[data-testid=save-only]');
  await until(async () => (profilesFile() ?? '').includes('"hasPassword": false'), 'forgetting was not saved');
  check('forgetting clears the flag in the file', (profilesFile() ?? '').includes('"hasPassword": false'), String(profilesFile()));
  await (await browser.$$('input'))[5].clearValue();
  await click('button.primary');
  await until(async () => (await browser.$('.error[role=alert]')).isExisting(), 'no error after forgetting');
  check('after forgetting, connecting without a password fails', !(await connectedNow()), '');

  // Delete (two-step).
  await click('[data-testid=delete-connection]');
  check('delete asks for a second click', (await text('[data-testid=delete-connection]')).includes('again'));
  await click('[data-testid=delete-connection]');
  await until(async () => (await profileRows()).length === 0, 'profile not deleted');
  check('deleting removes the profile from the list and the file', !(profilesFile() ?? '').includes('e2e server'));
  await shot('09-deleted');

  /* ---------- certificate verification (needs the CA made by scripts/test-db.sh) */
  const CA = process.env.SLON_TEST_CA;
  const OTHER_CA = process.env.SLON_TEST_OTHER_CA;
  if (!CA || !OTHER_CA) {
    console.log('SKIP  certificate verification (SLON_TEST_CA / SLON_TEST_OTHER_CA not set)');
  } else {
    const setMode = (mode) =>
      browser.execute((m) => {
        const el = document.querySelector('[data-testid=ssl-mode]');
        el.value = m;
        el.dispatchEvent(new Event('change', { bubbles: true }));
      }, mode);
    const connectErr = async () => {
      await click('button.primary');
      await until(async () => (await browser.$('.error[role=alert]')).isExisting() || (await connectedNow()), 'neither error nor connection');
      return (await connectedNow()) ? '' : text('.error[role=alert]');
    };
    await tick('save-connection', false);
    await (await browser.$$('input'))[1].setValue('localhost');
    await (await browser.$('[data-testid=password]')).setValue(PASS);
    await setMode('verify-full');
    await until(async () => (await browser.$('[data-testid=root-cert]')).isExisting(), 'CA file field did not appear');
    check('verifying modes show the CA certificate field', true);
    await shot('16-verify-form');

    await (await browser.$('[data-testid=root-cert]')).setValue(OTHER_CA);
    let e = await connectErr();
    check('verify-full with the wrong CA is refused with advice', /unknown ?issuer/i.test(e) && e.includes('not signed by a CA you trust'), e);
    await shot('17-verify-wrong-ca');

    await (await browser.$('[data-testid=root-cert]')).setValue(CA);
    await (await browser.$$('input'))[1].setValue('127.0.0.1');
    e = await connectErr();
    check('verify-full refuses a host name the certificate does not cover', /not valid for name/i.test(e) && e.includes('Verify CA'), e);

    await setMode('verify-ca');
    await click('button.primary');
    await until(connectedNow, 'verify-ca did not connect');
    check('verify-ca connects to the same host (name not checked)', true);
    await disconnect();

    await (await browser.$$('input'))[1].setValue('localhost');
    await setMode('verify-full');
    await (await browser.$('[data-testid=root-cert]')).setValue(CA);
    await (await browser.$('[data-testid=password]')).setValue(PASS);
    await click('button.primary');
    await until(connectedNow, 'verify-full did not connect with the right CA and name');
    check('verify-full connects with a trusted CA and matching name', true);
    await run('SELECT ssl::text AS ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()');
    await idle();
    await until(async () => (await firstRowCells()) != null, 'no result row'); // the first idle() can win the race with the click
    check('the verified connection is really encrypted', (await firstRowCells())?.includes('true'), JSON.stringify(await firstRowCells()));
    await shot('18-verified');
  }
} catch (e) {
  failed = true;
  console.error('\nE2E aborted:', e.message);
  try { await shot('99-failure'); } catch {}
} finally {
  try { await browser.deleteSession(); } catch {}
  driver.kill();
}

const bad = results.filter((r) => !r.ok).length;
console.log(`\n${results.length - bad}/${results.length} checks passed${failed ? ' (run aborted early)' : ''}`);
process.exit(bad || failed ? 1 : 0);
