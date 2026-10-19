// Takes the README screenshots by driving the real app through tauri-driver.
//
//   cd .. && npx tauri build --debug --no-bundle
//   eval "$(../scripts/test-db.sh start)"
//   psql ... -f demo.sql               # the schema the shots are taken against
//   xvfb-run -a -s "-screen 0 1440x900x24" node screenshots.mjs
//
// Writes PNGs to SLON_SHOTS (default ../docs/img). On Windows run it without
// xvfb-run to get WebView2's own rendering, which is what users actually see:
// WebKitGTK falls back to a wider monospace font, so a timestamp column or two
// ellipsizes in the Linux renders that fits on Windows.
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync } from 'node:fs';
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
const SHOTS = process.env.SLON_SHOTS ?? new URL('../docs/img', import.meta.url).pathname;
mkdirSync(SHOTS, { recursive: true });

const driver = spawn('tauri-driver', [], {
  stdio: ['ignore', 'inherit', 'inherit'],
  env: {
    ...process.env,
    XDG_CONFIG_HOME: mkdtempSync(join(tmpdir(), 'slon-shots-config-')),
    XDG_DATA_HOME: mkdtempSync(join(tmpdir(), 'slon-shots-data-')),
    XDG_CACHE_HOME: mkdtempSync(join(tmpdir(), 'slon-shots-cache-')),
    SLON_MEMORY_SECRETS: '1',
  },
});
await sleep(1500);

const browser = await remote({
  hostname: '127.0.0.1',
  port: 4444,
  logLevel: 'error',
  capabilities: { 'tauri:options': { application: APP } },
});

const ACTIVE = '.bodies > .body:not([hidden])';
const shot = (n) => browser.saveScreenshot(`${SHOTS}/${n}.png`);
const click = async (sel) => (await browser.$(sel)).click();
const until = (fn, msg, timeout = 30000) =>
  browser.waitUntil(fn, { timeout, interval: 150, timeoutMsg: msg });
const row = (label) => `//div[@role="treeitem"][.//span[contains(@class,"label")][normalize-space()="${label}"]]`;
const contains = (t) => `//div[@role="treeitem"][contains(., "${t}")]`;

async function setSql(sql) {
  await browser.execute(() => document.querySelector('.bodies > .body:not([hidden]) .cm-content').focus());
  await browser.keys('Escape');
  await browser.keys(['Control', 'a']);
  await browser.execute((q) => document.execCommand('insertText', false, q), sql);
  await browser.keys('Escape');
}
const idle = () =>
  until(async () => (await browser.$(`${ACTIVE} .toolbar button.primary`)).isEnabled(), 'query did not finish');
const run = async (sql) => {
  await setSql(sql);
  await click(`${ACTIVE} .toolbar button.primary`);
  await idle();
  await sleep(400);
};

try {
  /* ---------- connect */
  await until(async () => (await browser.$('h1')).isExisting(), 'no connect form');
  await sleep(1000); // the form restores the last connection on mount; let it settle first
  const fill = async (i, v) => {
    const el = (await browser.$$('input'))[i];
    if (v) await el.setValue(v);
    else await el.clearValue();
  };
  // A plausible-looking form for the screenshot, then the real values to connect with.
  await fill(0, 'Production read replica');
  await fill(1, 'db.internal.example.com');
  await fill(2, '5432');
  await fill(3, 'shop');
  await fill(4, 'app_readonly');
  await fill(5, 'hunter2hunter2');
  await click('[data-testid=save-connection]');
  await sleep(400);
  await shot('connect');

  await fill(0, '');
  await fill(1, HOST);
  await fill(2, PORT);
  await fill(3, DB);
  await fill(4, USER);
  await fill(5, PASS);
  await click('[data-testid=save-connection]');
  await sleep(200);

  await click('button.primary');
  await until(async () => (await browser.$('.sidebar')).isExisting(), 'did not connect');

  /* ---------- tree: databases -> schemas -> tables */
  await until(async () => (await browser.$(row(DB))).isExisting(), 'no database row');
  await click(row(DB));
  await until(async () => (await browser.$(row('shop'))).isExisting(), 'no shop schema');
  await click(row('shop'));
  await until(async () => (await browser.$(contains('Tables ('))).isExisting(), 'no Tables group');
  await click(contains('Tables ('));
  await until(async () => (await browser.$(row('orders'))).isExisting(), 'no orders table');
  await sleep(300);

  /* ---------- 1. results grid */
  await run(`SELECT o.id,
       c.full_name,
       c.country,
       o.placed_at,
       o.status,
       o.total,
       o.shipped_at
FROM shop.orders o
JOIN shop.customers c ON c.id = o.customer_id
WHERE o.placed_at > now() - interval '7 days'
ORDER BY o.placed_at DESC;`);
  await shot('results');

  /* ---------- 2. table properties */
  // a relation expands from its chevron (clicking the row only selects it)
  await click(`${row('orders')}//span[contains(@class,"chev")]`);
  await until(async () => (await browser.$(contains('Columns ('))).isExisting(), 'table did not expand');
  await sleep(400);
  const open = async (label) => {
    const el = await browser.$(contains(label));
    if (await el.isExisting()) {
      await el.click();
      await sleep(250);
    }
  };
  await open('Indexes (');
  await open('Triggers (');
  // widen the sidebar so the types and index definitions are not cut off
  await browser.execute(() => document.querySelector('[data-testid=sidebar-splitter]').focus());
  for (let i = 0; i < 3; i++) await browser.keys(['Shift', 'ArrowRight']);
  await sleep(300);
  await shot('properties');
  await browser.keys('Enter');
  await sleep(200);

  /* ---------- 3. explain analyze */
  await setSql(`SELECT c.country, count(*) AS orders, sum(o.total) AS revenue
FROM shop.orders o
JOIN shop.customers c ON c.id = o.customer_id
WHERE o.status = 'shipped'
GROUP BY c.country
ORDER BY revenue DESC;`);
  await click(`${ACTIVE} [data-testid=explain-analyze]`);
  await idle();
  await sleep(600);
  await shot('explain');

  /* ---------- 4. server messages */
  await run(`DO $$
BEGIN
  RAISE NOTICE 'rebuilding order totals';
  RAISE NOTICE 'processed % orders', (SELECT count(*) FROM shop.orders);
  RAISE WARNING 'order 4182 has no shipping address';
  RAISE NOTICE 'done in 142 ms';
END $$;`);
  await sleep(400);
  await shot('messages');

  /* ---------- 5. export dialog */
  await run(`SELECT * FROM shop.recent_orders LIMIT 500;`);
  await click(`${ACTIVE} [data-testid=export]`);
  await until(async () => (await browser.$('[data-testid=export-dialog]')).isExisting(), 'no export dialog');
  await sleep(400);
  await shot('export');
  await browser.keys('Escape');
  await sleep(300);

  /* ---------- 6. query history */
  await click('[data-testid=history-open]');
  await until(async () => (await browser.$('[data-testid=history-panel]')).isExisting(), 'no history panel');
  await sleep(600);
  await shot('history');
  await browser.keys('Escape');

  console.log(`screenshots written to ${SHOTS}`);
} catch (e) {
  console.error('failed:', e.message);
  try {
    await shot('failure');
  } catch {}
  process.exitCode = 1;
} finally {
  try {
    await browser.deleteSession();
  } catch {}
  driver.kill();
}
