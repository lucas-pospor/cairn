<script lang="ts">
  import { onMount, tick } from "svelte";
  import { app, type SettingsSection } from "../app.svelte";
  import { settings, DEFAULT_SETTINGS, type Settings } from "../settings.svelte";
  import { commands, comboFromEvent, displayCombo, hotkeyProblem } from "../commands";
  import { setRecordingHotkey } from "../hotkeyRecorder";
  import { closeOnBack } from "../back";
  import { backend } from "../backend";
  import { modal } from "../modal";
  import { isMobile } from "../platform";
  import Icon from "./Icon.svelte";
  import type { PluginInfo } from "../plugins";
  import { errorMessage } from "../types";
  import { CORE_PLUGINS } from "../corePlugins";
  import { option, pluginOn as corePluginOn, setOption, setPluginOn, type CorePlugin } from "../corePlugins/core";
  import { THEMES, choiceSettings, listedTheme, themeFor, themeInUse } from "../themes";
  import { MediaQuery } from "svelte/reactivity";

  const PERMISSION_TEXT: Record<string, string> = {
    read: "read all notes",
    write: "create and change notes",
    editor: "read and replace the selected text",
  };
  let pluginList = $state<PluginInfo[]>([]);
  // The zoom keys (zoomHotkeysEnabled in tauri.conf.json); Android has none.
  const zoomHint = isMobile ? "" : ` ${displayCombo("Mod+=")} and ${displayCombo("Mod+-")} zoom the whole window; ${displayCombo("Mod+0")} resets it.`;

  async function refreshPlugins() {
    pluginList = await app.plugins.available();
  }

  /** On in this vault's settings and approved on this device (only then does it run). */
  const pluginOn = (p: PluginInfo) => s.plugins.includes(p.file) && p.approved;

  async function togglePlugin(p: PluginInfo, on: boolean) {
    if (on && p.permissions.length) {
      const ok = await app.confirm({
        title: `Enable ${p.name}?`,
        message: `This plugin asks to ${p.permissions.map((x) => PERMISSION_TEXT[x]).join(", ")}.`,
        okLabel: "Enable",
      });
      if (!ok) {
        await refreshPlugins();
        return;
      }
    }
    // Approve on this device the file and permissions shown here (a file changed since then does not start).
    if (on) {
      try {
        await app.plugins.approve(p);
      } catch (e) {
        app.toast(`Could not turn on ${p.name}: ${errorMessage(e)}`, "error");
        await refreshPlugins();
        return;
      }
    }
    pluginList = pluginList.map((x) => (x.file === p.file ? { ...x, approved: on } : x));
    const list = on ? [...s.plugins.filter((f) => f !== p.file), p.file] : s.plugins.filter((f) => f !== p.file);
    set("plugins", list);
    // Also when the host stopped it for taking too long: turning it on starts it again.
    if (on) app.plugins.turnedOn(p.file);
    const off = await app.plugins.sync(list);
    if (on && off.includes(p.file)) {
      app.toast(`${p.name} has changed. Turn it on again to review it.`, "error");
      await refreshPlugins();
    }
    // Turned off here: it needs approving again before it runs on this device.
    if (!on) await app.plugins.revoke(p.file).catch((e) => console.warn("plugin approval not removed", e));
  }

  async function reloadPlugins() {
    app.plugins.stopAll();
    await app.plugins.sync(s.plugins);
    await refreshPlugins();
    app.toast("Plugins reloaded.");
  }

  // Ids that name each row's control after its title and description.
  const uid = $props.id();

  type Section = SettingsSection;
  let section = $state<Section>(app.settingsSection);
  const s = $derived(settings.value);

  function set<K extends keyof Settings>(k: K, v: Settings[K]) {
    settings.update({ [k]: v } as Partial<Settings>);
  }

  const SCHEMES = [
    { scheme: "light", key: "lightTheme", label: "Light" },
    { scheme: "dark", key: "darkTheme", label: "Dark" },
  ] as const;
  const systemDark = new MediaQuery("(prefers-color-scheme: dark)");
  /** The theme on screen: its swatch is shown next to the Theme list, and its accent by the colour picker while no custom accent is set. */
  const shown = $derived(themeInUse(s, systemDark.current));

  // ----- snippets -----
  let editing = $state<{ name: string; css: string } | null>(null);

  async function editSnippet(name: string) {
    editing = { name, css: (await backend.readConfig(`snippets/${name}`)) ?? "" };
  }

  async function saveSnippet() {
    if (!editing) return;
    const name = editing.name.trim();
    if (!name || /[\\/]/.test(name) || name.startsWith(".")) {
      app.toast("Snippet names cannot contain slashes or start with a dot.", "error");
      return;
    }
    let file: string;
    try {
      file = await settings.saveSnippet(name, editing.css);
    } catch (e) {
      // The editor stays open, so the CSS is not lost.
      app.toast(`Could not save the snippet: ${errorMessage(e)}`, "error");
      return;
    }
    if (!s.snippets.includes(file)) set("snippets", [...s.snippets, file]);
    editing = null;
  }

  async function cancelSnippet() {
    editing = null;
    await tick();
    dialog.querySelector<HTMLElement>("[data-testid=snippet-new]")?.focus();
  }

  function toggleSnippet(name: string, on: boolean) {
    set("snippets", on ? [...s.snippets, name] : s.snippets.filter((n) => n !== name));
  }

  // ----- core plugins -----
  // What is typed in an option field, by "<plugin id>.<key>": the example under it follows the
  // typing, and the value is saved when the field changes (or when Settings closes).
  let drafts = $state<Record<string, string>>({});

  function saveOption(p: CorePlugin, key: string, value: string) {
    delete drafts[`${p.id}.${key}`];
    if (value.trim() !== option(p, key)) setOption(p, key, value.trim());
  }

  function saveDrafts() {
    for (const [k, value] of Object.entries(drafts)) {
      const i = k.indexOf(".");
      const p = CORE_PLUGINS.find((x) => x.id === k.slice(0, i));
      if (p) saveOption(p, k.slice(i + 1), value);
    }
  }

  // ----- hotkeys -----
  let hotkeyFilter = $state("");
  let recordingFor = $state<string | null>(null);
  // Every command, also those that need an open note: they can be bound at any time. Those of
  // core plugins that are off are left out, and come back when the plugin is turned on.
  const allCommands = $derived.by(() => {
    void s.corePlugins;
    return commands.bindable().sort((a, b) => a.name.localeCompare(b.name));
  });
  let shownCommands = $derived(allCommands.filter((c) => c.name.toLowerCase().includes(hotkeyFilter.toLowerCase())));
  let version = $state(0); // bumped after changes to overrides

  /** The command's current keys; reads `version` so the list updates in place. */
  function keysOf(id: string) {
    void version;
    return commands.keysFor(id);
  }

  /** After a button removed itself (remove, restore default), focus the row's add button. */
  async function refocusRow(id: string) {
    await tick();
    if (dialog.contains(document.activeElement)) return;
    dialog.querySelector<HTMLElement>(`[data-command="${CSS.escape(id)}"] [data-testid=hotkey-add]`)?.focus();
  }

  function setKeys(id: string, keys: string[]) {
    const hk = { ...s.hotkeys, [id]: keys };
    set("hotkeys", hk);
    commands.setOverrides(hk);
    version++;
  }

  function resetKeys(id: string) {
    const hk = { ...s.hotkeys };
    delete hk[id];
    set("hotkeys", hk);
    commands.setOverrides(hk);
    version++;
    void refocusRow(id);
  }

  function removeKey(id: string, combo: string) {
    setKeys(id, commands.keysFor(id).filter((k) => k !== combo));
    void refocusRow(id);
  }

  function startRecording(id: string) {
    recordingFor = id;
    setRecordingHotkey(true);
  }

  function stopRecording() {
    recordingFor = null;
    setRecordingHotkey(false);
  }

  // ----- sync -----
  let form = $state({ server: "", token: "", vaultId: "", device: "", passphrase: "", confirm: "" });
  let mismatch = $state(false);
  // A setup goes on when Settings is closed; the store keeps it.
  const connecting = $derived(!!app.syncSetup?.busy);
  const syncError = $derived(mismatch ? "The two passphrases do not match." : (app.syncSetup?.error ?? null));

  async function prepareSyncForm() {
    const last = app.syncSetup;
    if (last && !form.server) Object.assign(form, { server: last.server, vaultId: last.vaultId, device: last.device });
    if (!form.device) form.device = await backend.defaultDeviceName().catch(() => "");
    if (!form.vaultId) form.vaultId = (app.vault?.name ?? "notes").toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "") || "notes";
  }

  async function connectSync() {
    mismatch = form.passphrase !== form.confirm;
    if (mismatch) return;
    await app.setupSync({
      server: form.server,
      token: form.token,
      vaultId: form.vaultId,
      device: form.device,
      passphrase: form.passphrase,
    });
    if (app.sync?.configured) form.passphrase = form.confirm = "";
  }

  async function disconnectSync() {
    const ok = await app.confirm({
      title: "Turn off sync",
      message: "This device stops syncing. Your notes stay here and on the server.",
      okLabel: "Turn off",
    });
    if (ok) app.sync = await backend.syncDisconnect();
  }

  function ago(ms: number | null) {
    if (!ms) return "never";
    const s = Math.round((Date.now() - ms) / 1000);
    if (s < 60) return "just now";
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    return new Date(ms).toLocaleString();
  }

  $effect(() => {
    if (section === "sync") void prepareSyncForm();
    if (section === "plugins") void refreshPlugins();
  });

  // ----- focus -----
  let dialog: HTMLDivElement;
  const opener = app.takeOpener();

  /** Escape closes the innermost thing: the snippet editor, then Settings. */
  function escape() {
    if (editing && section === "appearance") void cancelSnippet();
    else app.settingsOpen = false;
  }

  onMount(() => {
    void settings.refreshSnippets();
    (dialog.querySelector<HTMLElement>("nav button.on") ?? dialog).focus();
    const onKey = (e: KeyboardEvent) => {
      if (recordingFor) {
        // Tab and Shift+Tab move on as usual, so a keyboard user is never stuck here.
        if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.altKey) return stopRecording();
        e.preventDefault();
        e.stopPropagation();
        if (e.key === "Escape") return stopRecording();
        const combo = comboFromEvent(e);
        if (!combo) return;
        const problem = hotkeyProblem(combo);
        if (problem) {
          app.toast(problem, "error");
          return stopRecording();
        }
        const id = recordingFor;
        const clash = commands.conflicts(combo, id);
        if (clash.length) app.toast(`${displayCombo(combo)} was also used by "${clash.map((c) => c.name).join(", ")}"; it now runs this command only.`);
        for (const c of clash) setKeys(c.id, commands.keysFor(c.id).filter((k) => k !== combo));
        setKeys(id, [...commands.keysFor(id).filter((k) => k !== combo), combo]);
        stopRecording();
      }
    };
    window.addEventListener("keydown", onKey, true);
    const offBack = closeOnBack(() => (app.settingsOpen = false));
    return () => {
      window.removeEventListener("keydown", onKey, true);
      setRecordingHotkey(false);
      offBack();
      saveDrafts();
    };
  });
</script>

<div
  class="backdrop"
  role="presentation"
  onmousedown={(e) => {
    if (e.target === e.currentTarget) app.settingsOpen = false;
  }}
>
  <div class="settings" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" bind:this={dialog} use:modal={{ close: escape, opener }} data-testid="settings">
    <nav>
      <h2>Settings</h2>
      {#each [["appearance", "Appearance"], ["editor", "Editor"], ["files", "Files and links"], ["sync", "Sync"], ["core-plugins", "Core plugins"], ["plugins", "Plugins"], ["hotkeys", "Hotkeys"]] as [id, label]}
        <button
          class:on={section === id}
          aria-current={section === id ? "page" : undefined}
          onclick={() => (section = id as Section)}
          data-testid="settings-{id}">{label}</button
        >
      {/each}
      <span class="grow"></span>
      <p class="muted small">Stored in <code>.cairn/settings.json</code> inside this notebook.</p>
    </nav>
    <section>
      <button class="icon-btn close" title="Close" onclick={() => (app.settingsOpen = false)}><Icon name="x" /></button>
      {#if section === "appearance"}
        <h3>Appearance</h3>
        <div class="row">
          <div><b id="{uid}-theme">Theme</b><p id="{uid}-theme-d">Pick a theme to always use it, or System to follow the system's light or dark mode with the light and dark theme picked below.</p></div>
          <div class="theme-pick">
            <div class="inline">
              <span class="swatch" aria-hidden="true" title={shown.name} style:--sw-bg={shown.swatch.bg} style:--sw-side={shown.swatch.side} style:--sw-text={shown.swatch.text} style:--sw-accent={shown.swatch.accent} data-testid="theme-swatch"><i></i><i></i></span>
              <select aria-labelledby="{uid}-theme" aria-describedby="{uid}-theme-d" value={listedTheme(s)} onchange={(e) => settings.update(choiceSettings(e.currentTarget.value))} data-testid="theme-select">
                <option value="system">System</option>
                {#each SCHEMES as g (g.scheme)}
                  <optgroup label={g.label}>
                    {#each THEMES.filter((t) => t.scheme === g.scheme) as t (t.id)}
                      <option value={t.id}>{t.name}</option>
                    {/each}
                  </optgroup>
                {/each}
              </select>
            </div>
            {#if s.theme === "system"}
              <div class="inline system-pair">
                {#each SCHEMES as g (g.scheme)}
                  <label class="inline">
                    {g.label}
                    <select aria-label="{g.label} theme" value={themeFor(g.scheme, s[g.key])} onchange={(e) => set(g.key, e.currentTarget.value)} data-testid="theme-{g.scheme}-select">
                      {#each THEMES.filter((t) => t.scheme === g.scheme) as t (t.id)}
                        <option value={t.id}>{t.name}</option>
                      {/each}
                    </select>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        </div>
        <div class="row">
          <div><b id="{uid}-accent">Accent color</b><p id="{uid}-accent-d">Used for links, selection and highlights. Made darker or lighter if needed so text stays readable.</p></div>
          <div class="inline">
            <input type="color" aria-labelledby="{uid}-accent" aria-describedby="{uid}-accent-d" value={s.accent || shown.swatch.accent} oninput={(e) => set("accent", e.currentTarget.value)} />
            {#if s.accent}<button class="btn" onclick={() => set("accent", "")}>Default</button>{/if}
          </div>
        </div>
        <div class="row">
          <div><b id="{uid}-font">Text font</b><p id="{uid}-font-d">Font for note text.</p></div>
          <select aria-labelledby="{uid}-font" aria-describedby="{uid}-font-d" value={s.fontFamily} onchange={(e) => set("fontFamily", e.currentTarget.value as Settings["fontFamily"])}>
            <option value="sans">Sans serif</option>
            <option value="serif">Serif</option>
            <option value="mono">Monospace</option>
          </select>
        </div>
        <div class="row">
          <div>
            <b id="{uid}-size">Font size</b>
            <p>{s.fontSize}px. <span id="{uid}-size-d">For note text.{zoomHint}</span></p>
          </div>
          <input type="range" aria-labelledby="{uid}-size" aria-describedby="{uid}-size-d" aria-valuetext="{s.fontSize}px" min="12" max="24" value={s.fontSize} oninput={(e) => set("fontSize", Number(e.currentTarget.value))} />
        </div>
        <div class="row">
          <div><b id="{uid}-width">Line width</b><p>{s.lineWidth}px maximum</p></div>
          <input type="range" aria-labelledby="{uid}-width" aria-valuetext="{s.lineWidth}px" min="500" max="1400" step="20" value={s.lineWidth} oninput={(e) => set("lineWidth", Number(e.currentTarget.value))} />
        </div>
        <h4>CSS snippets</h4>
        <p class="muted">Snippets are CSS files in <code>.cairn/snippets/</code>. Enabled snippets are applied on top of the theme; they can override any of the <code>--bg</code>, <code>--text</code>, <code>--accent</code> variables.</p>
        {#each settings.available as name (name)}
          <div class="row snippet">
            <label class="inline"><input type="checkbox" checked={s.snippets.includes(name)} onchange={(e) => toggleSnippet(name, e.currentTarget.checked)} /> {name}</label>
            <button class="btn" aria-label="Edit {name}" onclick={() => editSnippet(name)}>Edit</button>
          </div>
        {/each}
        {#if editing}
          <div class="snippet-editor">
            <input class="text-input" bind:value={editing.name} placeholder="name.css" aria-label="Snippet file name" data-testid="snippet-name" />
            <textarea class="text-input" bind:value={editing.css} aria-label="Snippet CSS" rows="10" spellcheck="false" data-testid="snippet-css"></textarea>
            <div class="inline">
              <button class="btn primary" onclick={saveSnippet} data-testid="snippet-save">Save and enable</button>
              <button class="btn" onclick={cancelSnippet}>Cancel</button>
            </div>
          </div>
        {:else}
          <button class="btn" onclick={() => (editing = { name: "custom.css", css: ":root {\n  /* --accent: #b35c44; */\n}\n" })} data-testid="snippet-new">New snippet</button>
        {/if}
      {:else if section === "editor"}
        <h3>Editor</h3>
        <div class="row">
          <div>
            <b id="{uid}-mode">Default view for new tabs</b>
            <p id="{uid}-mode-d">Live Preview renders Markdown as you type; source shows the raw text.</p>
          </div>
          <select aria-labelledby="{uid}-mode" aria-describedby="{uid}-mode-d" value={s.defaultMode} onchange={(e) => set("defaultMode", e.currentTarget.value as Settings["defaultMode"])}>
            <option value="live">Live Preview</option>
            <option value="source">Source</option>
            <option value="preview">Reading view</option>
          </select>
        </div>
        <div class="row">
          <div><b id="{uid}-spell">Spell check</b><p id="{uid}-spell-d">Use the system spell checker in the editor.</p></div>
          <input type="checkbox" aria-labelledby="{uid}-spell" aria-describedby="{uid}-spell-d" checked={s.spellcheck} onchange={(e) => set("spellcheck", e.currentTarget.checked)} />
        </div>
      {:else if section === "files"}
        <h3>Files and links</h3>
        <div class="row">
          <div>
            <b id="{uid}-attach">Attachment folder</b>
            <p id="{uid}-attach-d">Where pasted and dropped files are saved, relative to the notebook folder.</p>
          </div>
          <input class="text-input narrow" aria-labelledby="{uid}-attach" aria-describedby="{uid}-attach-d" value={s.attachmentFolder} onchange={(e) => set("attachmentFolder", e.currentTarget.value.trim())} />
        </div>
        <div class="row">
          <div>
            <b id="{uid}-unresolved">Show unresolved links in graph</b>
            <p id="{uid}-unresolved-d">Adds nodes for notes that are linked but do not exist yet.</p>
          </div>
          <input type="checkbox" aria-labelledby="{uid}-unresolved" aria-describedby="{uid}-unresolved-d" checked={s.graphShowUnresolved} onchange={(e) => set("graphShowUnresolved", e.currentTarget.checked)} />
        </div>
      {:else if section === "sync"}
        <h3>Sync</h3>
        {#if app.sync?.configured}
          <div class="row">
            <div>
              <b>Connected</b>
              <p>Notebook <code>{app.sync.vaultId}</code> on {app.sync.server}, as "{app.sync.device}".</p>
            </div>
            <span class="sync-state {app.sync.state}" data-testid="sync-state">{app.sync.state}</span>
          </div>
          <div class="row">
            <div>
              <b>Last sync</b>
              <p>
                {ago(app.sync.lastSync)}{#if app.sync.lastSync}: {app.sync.lastPulled} received, {app.sync.lastPushed} sent{/if}
              </p>
              {#if app.sync.lastError}<p class="err">{app.sync.lastError}</p>{/if}
            </div>
            <button class="btn primary" onclick={async () => (app.sync = await backend.syncNow())} disabled={app.sync.state === "syncing"} data-testid="sync-now">Sync now</button>
          </div>
          {#if app.sync.skipped.length}
            <h4>Files not synced</h4>
            <p class="muted">The last sync left these files out.</p>
            {#each app.sync.skipped as f}
              <div class="row snippet skipped" data-testid="sync-skipped">
                <div><code>{f.path || "(a file from the server)"}</code><p>{f.reason}</p></div>
              </div>
            {/each}
          {/if}
          {#if app.sync.conflicts.length}
            <h4>Conflict copies</h4>
            <p class="muted">When the same part of a note changed on two devices, the other version was saved next to yours. Compare them and delete the one you do not need.</p>
            {#each app.sync.conflicts as c}
              <div class="row snippet">
                <button class="linkish" onclick={() => ((app.settingsOpen = false), app.openNote(c, { newTab: true }))}>{c}</button>
              </div>
            {/each}
          {/if}
          <div class="row">
            <div><b>Turn off sync on this device</b><p>Notes are kept. You can connect again later with the same passphrase.</p></div>
            <button class="btn" onclick={disconnectSync}>Turn off</button>
          </div>
        {:else}
          <p class="muted">
            Sync keeps this notebook in step with your other devices through your own Cairn server. Notes are encrypted on this
            device with your passphrase before they are sent; the server only stores encrypted data.
          </p>
          <form class="sync-form" onsubmit={(e) => { e.preventDefault(); void connectSync(); }}>
            <!-- No automatic capitals or corrections: a vault name typed as "Notes" instead of "notes" is another vault.
                 inputmode, not type="url": the browser would refuse a URL without http(s):// before Cairn can explain it. -->
            <label>Server URL<input class="text-input" inputmode="url" placeholder="https://notes.example.com" bind:value={form.server} required autocapitalize="off" autocorrect="off" spellcheck="false" data-testid="sync-server" /></label>
            <label>Access token<input class="text-input" type="password" placeholder="CAIRN_TOKENS value on the server" bind:value={form.token} required data-testid="sync-token" /></label>
            <label>Notebook name on the server<input class="text-input" bind:value={form.vaultId} pattern="[A-Za-z0-9_-]{'{'}1,64{'}'}" required autocapitalize="off" autocorrect="off" spellcheck="false" data-testid="sync-vault" /></label>
            <label>This device's name<input class="text-input" bind:value={form.device} required autocapitalize="off" autocorrect="off" spellcheck="false" data-testid="sync-device" /></label>
            <label>Encryption passphrase<input class="text-input" type="password" bind:value={form.passphrase} minlength="8" required data-testid="sync-pass" /></label>
            <label>Passphrase again<input class="text-input" type="password" bind:value={form.confirm} minlength="8" required data-testid="sync-pass2" /></label>
            <p class="warn">
              Use the same notebook name and passphrase on every device. If you lose the passphrase, the data on the server cannot
              be decrypted by anyone, including you.
            </p>
            {#if syncError}<p class="err" data-testid="sync-error">{syncError}</p>{/if}
            <div class="inline">
              <button class="btn primary" type="submit" disabled={connecting} data-testid="sync-connect">{connecting ? "Connecting…" : "Connect and sync"}</button>
              {#if connecting}<button class="btn" type="button" onclick={() => app.cancelSyncSetup()} data-testid="sync-cancel">Cancel</button>{/if}
            </div>
          </form>
        {/if}
      {:else if section === "core-plugins"}
        <h3>Core plugins</h3>
        <p class="muted">Optional features that come with Cairn. They need no approval, and their settings are saved in this notebook.</p>
        {#each CORE_PLUGINS as p (p.id)}
          {@const on = corePluginOn(p)}
          <div class="row" data-testid="core-plugin-row" data-id={p.id}>
            <div><b id="{uid}-cp-{p.id}">{p.name}</b><p id="{uid}-cp-{p.id}-d">{p.description}</p></div>
            <input
              type="checkbox"
              aria-labelledby="{uid}-cp-{p.id}"
              aria-describedby="{uid}-cp-{p.id}-d"
              checked={on}
              onchange={(e) => setPluginOn(p, e.currentTarget.checked)}
              data-testid="core-plugin-toggle"
            />
          </div>
          {#if on}
            {#each p.options as o (o.key)}
              {@const k = `${p.id}.${o.key}`}
              {@const value = drafts[k] ?? option(p, o.key)}
              {@const check = o.check?.(value.trim(), app.coreHost, (key) => (drafts[`${p.id}.${key}`] ?? option(p, key)).trim())}
              {@const id = `${uid}-cp-${p.id}-${o.key}`}
              <div class="row option" data-testid="core-plugin-option" data-id={p.id} data-key={o.key}>
                <div>
                  <b id={id}>{o.label}</b>
                  <p id="{id}-d">{o.description}</p>
                  {#if check?.problem}
                    <p class="err" id="{id}-c" data-testid="core-plugin-problem">{check.problem}</p>
                  {:else if check?.example}
                    <p id="{id}-c" data-testid="core-plugin-example">{check.example}</p>
                  {/if}
                </div>
                <input
                  class="text-input narrow"
                  aria-labelledby={id}
                  aria-describedby="{id}-d{check?.problem || check?.example ? ` ${id}-c` : ''}"
                  {value}
                  placeholder={o.placeholder}
                  autocapitalize="off"
                  autocorrect="off"
                  spellcheck="false"
                  oninput={(e) => (drafts[k] = e.currentTarget.value)}
                  onchange={(e) => saveOption(p, o.key, e.currentTarget.value)}
                  data-testid="core-plugin-input"
                />
              </div>
            {/each}
          {/if}
        {/each}
      {:else if section === "plugins"}
        <h3>Plugins</h3>
        <p class="muted">
          Plugins are JavaScript files in <code>.cairn/plugins/</code> in this notebook. Each one runs in its own sandbox
          without access to the network, the file system or the rest of the app; it can only add commands, show messages, and
          use the permissions it asks for. Only enable plugins you trust.
        </p>
        {#each pluginList as p (p.file)}
          <div class="row" data-testid="plugin-row" data-file={p.file}>
            <div>
              <!-- The file name too: @name is free text, and two plugins can have the same one. -->
              <b>{p.name}</b> <span class="file" data-testid="plugin-file">{p.file}</span>
              {#if p.description}<p>{p.description}</p>{/if}
              <p>{p.permissions.length ? `Can ${p.permissions.map((x) => PERMISSION_TEXT[x]).join(", ")}.` : "Needs no permissions."}</p>
              {#if s.plugins.includes(p.file) && !p.approved}<p data-testid="plugin-not-approved">Off on this device until you turn it on.</p>{/if}
            </div>
            <input
              type="checkbox"
              aria-label={p.name}
              checked={pluginOn(p)}
              onchange={(e) => togglePlugin(p, e.currentTarget.checked)}
              data-testid="plugin-toggle"
            />
          </div>
        {:else}
          <p class="muted">No plugins found. Put <code>.js</code> files into <code>.cairn/plugins/</code> and press Reload.</p>
        {/each}
        <button class="btn" onclick={reloadPlugins} data-testid="plugins-reload">Reload plugins</button>
      {:else}
        <h3>Hotkeys</h3>
        <input class="text-input" placeholder="Filter commands" aria-label="Filter commands" bind:value={hotkeyFilter} />
        <div class="hotkeys">
          {#each shownCommands as c (c.id)}
            <div class="hk-row" data-testid="hotkey-row" data-command={c.id}>
              <span class="hk-name">{c.name}</span>
              <span class="hk-keys">
                {#each keysOf(c.id) as k}
                  <span class="combo">{displayCombo(k)}<button title="Remove" aria-label="Remove {displayCombo(k)} from {c.name}" onclick={() => removeKey(c.id, k)}>×</button></span>
                {/each}
                <!-- One button for both states, so focus stays on it while recording. -->
                <button
                  class={recordingFor === c.id ? "combo recording" : "icon-btn"}
                  title="Add hotkey"
                  aria-label={recordingFor === c.id ? undefined : `Add hotkey for ${c.name}`}
                  onclick={() => startRecording(c.id)}
                  data-testid="hotkey-add">{recordingFor === c.id ? "Press keys… (Esc cancels)" : "+"}</button
                >
                {#if s.hotkeys[c.id]}
                  <button class="icon-btn" title="Restore default" aria-label="Restore default for {c.name}" onclick={() => resetKeys(c.id)}><Icon name="refresh" size={14} /></button>
                {/if}
              </span>
            </div>
          {/each}
        </div>
        <button class="btn" onclick={() => {
          set("hotkeys", { ...DEFAULT_SETTINGS.hotkeys });
          commands.setOverrides({});
          version++;
        }}>Restore all defaults</button>
      {/if}
    </section>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.3);
    z-index: 95;
    display: grid;
    place-items: center;
    padding: 16px;
  }
  .settings {
    width: min(920px, 100%);
    height: min(640px, 100%);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow);
    display: grid;
    grid-template-columns: 200px 1fr;
    overflow: hidden;
  }
  nav {
    background: var(--bg-side);
    border-right: 1px solid var(--border);
    padding: 16px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  nav h2 {
    font-size: 13px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin: 0 8px 10px;
  }
  nav button {
    text-align: left;
    padding: 7px 10px;
    border-radius: 6px;
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.on {
    background: var(--bg-active);
    font-weight: 600;
    /* In the high-contrast themes, a ring as well as the tint. */
    box-shadow: inset 0 0 0 var(--ring) var(--accent);
  }
  /* Focused as well: the ring that shows the selection, and the focus ring. */
  nav button.on:focus-visible {
    box-shadow:
      inset 0 0 0 var(--ring) var(--accent),
      0 0 0 var(--ring) var(--accent);
  }
  .grow {
    flex: 1;
  }
  .small {
    font-size: 11.5px;
    padding: 0 8px;
    line-height: 1.4;
  }
  section {
    overflow: auto;
    padding: 20px 28px 32px;
    position: relative;
    user-select: text;
  }
  .close {
    position: absolute;
    top: 12px;
    right: 12px;
  }
  h3 {
    margin: 0 0 12px;
    font-size: 18px;
  }
  h4 {
    margin: 24px 0 6px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 20px;
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
  }
  .row p {
    margin: 2px 0 0;
    color: var(--text-muted);
    font-size: 12.5px;
  }
  .row .file {
    color: var(--text-muted);
    font-size: 12.5px;
  }
  .row.snippet {
    padding: 6px 0;
  }
  .row.option {
    padding-left: 20px;
  }
  .skipped code {
    overflow-wrap: anywhere;
  }
  .inline {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  select,
  .narrow {
    background: var(--bg-input);
    border: 1px solid var(--border-strong);
    border-radius: 6px;
    padding: 5px 8px;
    color: var(--text);
    font: inherit;
  }
  .narrow {
    width: 200px;
  }
  /* As .text-input:focus, which the rule above outranks. */
  .narrow:focus {
    border-color: var(--accent);
  }
  input[type="checkbox"] {
    accent-color: var(--accent);
    width: 16px;
    height: 16px;
  }
  /* The Theme list, and under it System's light and dark theme. */
  .theme-pick {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
  }
  .system-pair {
    flex-wrap: wrap;
    justify-content: flex-end;
    column-gap: 14px;
  }
  /* The theme in miniature: sidebar, page, a line of text and one of accent. */
  .swatch {
    flex-shrink: 0;
    display: grid;
    grid-template-columns: 11px 1fr;
    width: 44px;
    height: 28px;
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    overflow: hidden;
  }
  .swatch i:first-child {
    background: var(--sw-side);
  }
  .swatch i:last-child {
    background:
      linear-gradient(var(--sw-text), var(--sw-text)) 6px 8px / 20px 3px no-repeat,
      linear-gradient(var(--sw-accent), var(--sw-accent)) 6px 15px / 12px 3px no-repeat,
      var(--sw-bg);
  }
  input[type="range"] {
    accent-color: var(--accent);
    width: 200px;
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.9em;
  }
  .snippet-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 10px;
  }
  textarea {
    font-family: var(--font-mono) !important;
    font-size: 13px;
    resize: vertical;
  }
  .hotkeys {
    margin: 10px 0 16px;
  }
  .hk-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .hk-name {
    flex: 1;
  }
  .hk-keys {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .combo {
    font-family: var(--font-mono);
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 2px 4px 2px 7px;
    display: inline-flex;
    gap: 4px;
    align-items: center;
    background: var(--bg-side);
  }
  span.combo {
    padding: 0 0 0 7px;
  }
  /* 24px square: the smallest pointer target WCAG 2.5.8 allows. */
  .combo button {
    color: var(--text-faint);
    min-width: 24px;
    min-height: 24px;
  }
  .combo button:hover {
    color: var(--danger);
  }
  .sync-form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 480px;
    margin-top: 12px;
  }
  .sync-form label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-weight: 600;
    font-size: 13px;
  }
  .sync-form label input {
    font-weight: 400;
  }
  .warn {
    font-size: 12.5px;
    color: var(--text-muted);
    border-left: 3px solid var(--unresolved);
    padding-left: 10px;
    margin: 4px 0;
  }
  .err {
    color: var(--danger) !important;
    font-size: 13px;
    user-select: text;
  }
  .sync-state {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--accent-soft);
  }
  .sync-state.error {
    background: color-mix(in srgb, var(--danger) 20%, transparent);
    color: var(--danger);
  }
  .linkish {
    color: var(--link);
    text-align: left;
  }
  @media (max-width: 760px) {
    .backdrop {
      padding: 0;
    }
    .settings {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
      height: 100%;
      border-radius: 0;
    }
    nav {
      flex-direction: row;
      overflow-x: auto;
      border-right: none;
      border-bottom: 1px solid var(--border);
      padding: calc(8px + env(safe-area-inset-top)) 8px 8px;
    }
    nav h2,
    nav .small,
    nav .grow {
      display: none;
    }
    nav button {
      white-space: nowrap;
    }
    section {
      padding: 16px;
    }
    .row {
      flex-wrap: wrap;
    }
    /* On its own line under the label, the Theme list starts at the left. */
    .theme-pick {
      align-items: flex-start;
    }
    .system-pair {
      justify-content: flex-start;
    }
  }
  .recording {
    border-color: var(--accent);
    color: var(--accent);
    padding-right: 7px;
  }
</style>
