<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Icon from './Icon.svelte';
  import { toPgError } from './api';
  import { profiles } from './profiles.svelte';
  import { app, loadLastConnection } from './store.svelte';
  import type { PgError, Profile, SslMode } from './types';

  const last = loadLastConnection();

  let selectedId = $state<string | null>(null);
  let name = $state('');
  let host = $state(last.host ?? 'localhost');
  let port = $state(String(last.port ?? 5432));
  let database = $state(last.database ?? 'postgres');
  let user = $state(last.user ?? 'postgres');
  let password = $state('');
  let sslMode = $state<SslMode>(last.sslMode ?? 'prefer');
  let sslRootCert = $state(last.sslRootCert ?? '');
  let save = $state(false);
  let remember = $state(false);

  let busy = $state(false);
  let error = $state<PgError | null>(null);
  let confirmDelete = $state(false);
  let passwordInput: HTMLInputElement;

  const selected = $derived(profiles.items.find((p) => p.id === selectedId) ?? null);
  const verifying = $derived(sslMode === 'verify-ca' || sslMode === 'verify-full');
  const hasSavedPassword = $derived(!!selected?.hasPassword && remember);

  function fill(p: Profile) {
    selectedId = p.id;
    name = p.name;
    host = p.host;
    port = String(p.port);
    database = p.database;
    user = p.user;
    sslMode = p.sslMode;
    sslRootCert = p.sslRootCert ?? '';
    password = '';
    save = true;
    remember = p.hasPassword;
    error = null;
    confirmDelete = false;
  }

  function startNew() {
    selectedId = null;
    name = '';
    host = last.host ?? 'localhost';
    port = String(last.port ?? 5432);
    database = last.database ?? 'postgres';
    user = last.user ?? 'postgres';
    sslMode = last.sslMode ?? 'prefer';
    sslRootCert = last.sslRootCert ?? '';
    password = '';
    save = false;
    remember = false;
    error = null;
    confirmDelete = false;
  }

  async function pick(p: Profile) {
    fill(p);
    await tick();
    passwordInput?.focus();
  }

  onMount(async () => {
    await profiles.load();
    // Returning users land on their most recent connection.
    if (profiles.items.length > 0) fill(profiles.items[0]);
    await tick();
    passwordInput?.focus();
  });

  function parsePort(): number | null {
    const n = Number(port);
    if (!Number.isInteger(n) || n < 1 || n > 65535) {
      error = { kind: 'connection', message: 'Port must be a number between 1 and 65535.' };
      return null;
    }
    return n;
  }

  /** Saves the form as a saved connection (new or the selected one). */
  async function persist(portNumber: number): Promise<Profile> {
    const saved = await profiles.save({
      id: selectedId,
      name,
      host,
      port: portNumber,
      database,
      user,
      sslMode,
      sslRootCert: verifying ? sslRootCert : null,
      password: remember && password ? password : null,
      forgetPassword: !remember && !!selected?.hasPassword,
    });
    selectedId = saved.id;
    name = saved.name;
    remember = saved.hasPassword;
    return saved;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (busy) return;
    const portNumber = parsePort();
    if (portNumber === null) return;
    busy = true;
    error = null;
    try {
      let profileId = selectedId;
      if (save) profileId = (await persist(portNumber)).id;
      await app.connect(
        { host, port: portNumber, database, user, password, sslMode, sslRootCert: verifying ? sslRootCert : null },
        profileId,
      );
    } catch (err) {
      error = toPgError(err);
    } finally {
      busy = false;
    }
  }

  async function saveOnly() {
    if (busy) return;
    const portNumber = parsePort();
    if (portNumber === null) return;
    busy = true;
    error = null;
    try {
      await persist(portNumber);
      password = '';
    } catch (err) {
      error = toPgError(err);
    } finally {
      busy = false;
    }
  }

  async function remove() {
    if (!selectedId) return;
    if (!confirmDelete) {
      confirmDelete = true;
      return;
    }
    busy = true;
    try {
      await profiles.remove(selectedId);
      startNew();
    } catch (err) {
      error = toPgError(err);
      confirmDelete = false;
    } finally {
      busy = false;
    }
  }

  function when(p: Profile): string {
    return `${p.user}@${p.host}:${p.port}`;
  }
</script>

<div class="wrap">
  <div class="card">
    <aside aria-label="Saved connections">
      <div class="aside-head">
        <span>Saved connections</span>
        <button class="ghost" type="button" title="New connection" data-testid="new-connection" onclick={startNew}>
          <Icon name="plus" size={14} />
        </button>
      </div>
      <ul>
        {#each profiles.items as p (p.id)}
          <li>
            <button
              type="button"
              class="profile"
              class:active={p.id === selectedId}
              data-testid="profile"
              onclick={() => pick(p)}
              ondblclick={() => (document.getElementById('connect-btn') as HTMLButtonElement | null)?.click()}
            >
              <span class="pname">{p.name}</span>
              <span class="psub">
                {when(p)}
                {#if p.hasPassword}<Icon name="key" size={11} />{/if}
              </span>
            </button>
          </li>
        {/each}
      </ul>
      {#if profiles.error}
        <div class="aside-note error-note" role="alert">{profiles.error.message}</div>
      {:else if profiles.loaded && profiles.items.length === 0}
        <div class="aside-note">Nothing saved yet. Tick “Save this connection” to keep one.</div>
      {/if}
    </aside>

    <form onsubmit={submit}>
      <header>
        <Icon name="database" size={22} />
        <div>
          <h1>{selected ? selected.name : 'Connect to PostgreSQL'}</h1>
          <p>{selected ? 'Saved connection' : 'Enter the server details.'}</p>
        </div>
      </header>

      <div class="grid">
        <label class="full">
          <span>Name <span class="opt">(optional)</span></span>
          <input bind:value={name} placeholder="e.g. Production read replica" autocomplete="off" data-testid="name" />
        </label>
        <label class="host">Host<input bind:value={host} required spellcheck="false" autocomplete="off" /></label>
        <label class="port">Port<input bind:value={port} required inputmode="numeric" autocomplete="off" /></label>
        <label class="full">Database<input bind:value={database} spellcheck="false" autocomplete="off" /></label>
        <label class="full">User<input bind:value={user} required spellcheck="false" autocomplete="username" /></label>
        <label class="full">
          Password
          <input
            type="password"
            bind:value={password}
            bind:this={passwordInput}
            placeholder={hasSavedPassword ? 'Saved – leave blank to use it' : ''}
            autocomplete="current-password"
            data-testid="password"
          />
        </label>
        <label class="full">
          SSL mode
          <select bind:value={sslMode} data-testid="ssl-mode">
            <option value="prefer">Prefer – use TLS when the server offers it</option>
            <option value="require">Require – encrypt, certificate not checked</option>
            <option value="verify-ca">Verify CA – encrypt, certificate must be trusted</option>
            <option value="verify-full">Verify full – trusted certificate for this host name</option>
            <option value="disable">Disable – never use TLS</option>
          </select>
        </label>
        {#if verifying}
          <label class="full">
            <span>CA certificate file <span class="opt">(PEM, optional)</span></span>
            <input
              bind:value={sslRootCert}
              placeholder="Empty = use the Windows certificate store"
              spellcheck="false"
              autocomplete="off"
              data-testid="root-cert"
            />
            <span class="note">
              Needed for private CAs, e.g. AWS RDS (<code>global-bundle.pem</code>) or your company CA.
            </span>
          </label>
        {/if}
      </div>

      <div class="checks">
        <label class="check">
          <input type="checkbox" bind:checked={save} data-testid="save-connection" />
          Save this connection
        </label>
        <label class="check" class:off={!save}>
          <input type="checkbox" bind:checked={remember} disabled={!save} data-testid="remember-password" />
          Remember the password
          <span class="opt">in the Windows Credential Manager</span>
        </label>
      </div>

      {#if error}
        <div class="error" role="alert">
          <Icon name="alert" size={16} />
          <div>
            <strong>{error.message}</strong>
            {#if error.detail}<div>{error.detail}</div>{/if}
            {#if error.hint}<div class="hint">Hint: {error.hint}</div>{/if}
            {#if error.sqlstate}<div class="code">SQLSTATE {error.sqlstate}</div>{/if}
          </div>
        </div>
      {/if}

      <footer>
        {#if selected}
          <button class="danger" type="button" disabled={busy} onclick={remove} data-testid="delete-connection">
            <Icon name="trash" size={14} />
            {confirmDelete ? 'Click again to delete' : 'Delete'}
          </button>
        {/if}
        <span class="spacer"></span>
        {#if save}
          <button type="button" disabled={busy} onclick={saveOnly} data-testid="save-only">Save</button>
        {/if}
        <button class="primary" id="connect-btn" type="submit" disabled={busy}>
          {#if busy}<span class="spinner"></span> Connecting…{:else}Connect{/if}
        </button>
      </footer>
    </form>
  </div>
</div>

<style>
  .wrap {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .card {
    display: grid;
    grid-template-columns: 230px 1fr;
    width: 720px;
    max-width: 100%;
    max-height: 100%;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--panel);
    box-shadow: 0 8px 30px rgb(0 0 0 / 0.08);
  }
  form {
    padding: 24px;
    overflow: auto;
  }
  aside {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: color-mix(in srgb, var(--panel) 92%, var(--text) 8%);
  }
  .aside-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px 16px;
    color: var(--muted);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  ul {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0 8px 8px;
    overflow: auto;
    list-style: none;
  }
  .profile {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    width: 100%;
    height: auto;
    padding: 7px 8px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    text-align: left;
  }
  .profile.active {
    background: var(--selected);
  }
  .pname {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .psub {
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-size: 12px;
  }
  .aside-note {
    padding: 8px 16px 16px;
    color: var(--muted);
    font-size: 12px;
  }
  .error-note {
    color: var(--danger);
  }
  .opt {
    color: var(--muted);
    font-weight: normal;
    opacity: 0.8;
  }
  .checks {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 14px;
  }
  .check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-size: 13px;
  }
  .check.off {
    opacity: 0.5;
  }
  .check input {
    width: auto;
  }
  .note {
    color: var(--muted);
    font-size: 11.5px;
  }
  code {
    font-family: var(--mono);
  }
  .spacer {
    flex: 1;
  }
  button.danger {
    color: var(--danger);
  }
  header {
    display: flex;
    gap: 12px;
    align-items: center;
    margin-bottom: 20px;
    color: var(--accent);
  }
  h1 {
    margin: 0;
    font-size: 17px;
    color: var(--text);
  }
  p {
    margin: 2px 0 0;
    color: var(--muted);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 110px;
    gap: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--muted);
    font-size: 12px;
  }
  label input,
  label select {
    color: var(--text);
    font-size: 13px;
  }
  .full {
    grid-column: 1 / -1;
  }
  .error {
    display: flex;
    gap: 10px;
    margin-top: 16px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--danger-bg);
    color: var(--danger);
    word-break: break-word;
  }
  .hint,
  .code {
    margin-top: 2px;
    font-size: 12px;
    opacity: 0.85;
  }
  footer {
    display: flex;
    gap: 8px;
    align-items: center;
    margin-top: 20px;
  }
  footer button {
    min-width: 110px;
    justify-content: center;
    height: 32px;
  }
</style>
