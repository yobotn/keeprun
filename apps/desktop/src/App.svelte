<script lang="ts">
  import { openPath, openUrl } from "@tauri-apps/plugin-opener";
  import { ArrowUpCircle, Monitor, Moon, Play, Plus, RotateCw, Search, Server, Square, Sun, TriangleAlert } from "@lucide/svelte";
  import {
    api,
    appUrl,
    config,
    connection,
    isActive,
    stateLabel,
    type AppView,
    type Connection,
  } from "./lib/api";
  import { cycleTheme, applyTheme, theme } from "./lib/theme.svelte";
  import StatusDot from "./lib/StatusDot.svelte";
  import Overview from "./lib/Overview.svelte";
  import LogPanel from "./lib/LogPanel.svelte";
  import EnvEditor from "./lib/EnvEditor.svelte";
  import SettingsTab from "./lib/SettingsTab.svelte";
  import EventsList from "./lib/EventsList.svelte";
  import AddAppDialog from "./lib/AddAppDialog.svelte";
  import AgentDialog from "./lib/AgentDialog.svelte";
  import TasksTab from "./lib/TasksTab.svelte";
  import UpdateDialog from "./lib/UpdateDialog.svelte";
  import { checkForUpdates, scheduleUpdateChecks, updates } from "./lib/updates.svelte";
  import CommandPalette, { type Command } from "./lib/CommandPalette.svelte";

  type Tab = "overview" | "logs" | "tasks" | "environment" | "settings" | "events";
  const tabs: [Tab, string][] = [
    ["overview", "Overview"],
    ["logs", "Logs"],
    ["tasks", "Tasks"],
    ["environment", "Environment"],
    ["settings", "Settings"],
    ["events", "Events"],
  ];

  let conn = $state<Connection | null>(null);
  let apps = $state<AppView[]>([]);
  let selectedId = $state<string | null>(null);
  let tab = $state<Tab>("overview");
  let agentError = $state("");
  let actionError = $state("");
  let busy = $state(false);
  let adding = $state(false);
  let palette = $state(false);
  let agentOpen = $state(false);
  let updateOpen = $state(false);

  $effect(() => scheduleUpdateChecks());
  let loaded = $state(false);
  let now = $state(Date.now());

  const selected = $derived(apps.find((a) => a.id === selectedId) ?? null);

  // Group by first tag. Untagged apps come first, without a header.
  const groups = $derived.by(() => {
    const map = new Map<string, AppView[]>();
    for (const a of [...apps].sort((x, y) => x.name.localeCompare(y.name))) {
      const g = a.tags[0] ?? "";
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(a);
    }
    return [...map.entries()].sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b)));
  });
  const ordered = $derived(groups.flatMap(([, list]) => list));

  async function refresh() {
    try {
      conn = await connection();
      apps = await api.list();
      agentError = "";
      if (!selectedId || !apps.some((a) => a.id === selectedId)) selectedId = ordered[0]?.id ?? null;
    } catch (e) {
      agentError = (e as Error).message;
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    refresh();
    const poll = setInterval(refresh, config.pollMs);
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  });

  async function run(fn: () => Promise<unknown>) {
    busy = true;
    actionError = "";
    try {
      await fn();
    } catch (e) {
      actionError = (e as Error).message;
    } finally {
      busy = false;
      await refresh();
    }
  }

  const start = (id: string) => run(() => api.start(id));
  const stop = (id: string) => run(() => api.stop(id));
  const restart = (id: string) => run(() => api.restart(id));

  function select(id: string, t?: Tab) {
    if (id !== selectedId) actionError = "";
    selectedId = id;
    if (t) tab = t;
  }

  function onKey(e: KeyboardEvent) {
    if (adding || palette || agentOpen || updateOpen) return;
    const k = e.key.toLowerCase();
    if (e.ctrlKey && k === "k") {
      e.preventDefault();
      palette = true;
      return;
    }
    if (e.ctrlKey && k === "n") {
      e.preventDefault();
      adding = true;
      return;
    }
    if (e.ctrlKey && e.key >= "1" && e.key <= String(tabs.length)) {
      e.preventDefault();
      tab = tabs[Number(e.key) - 1][0];
      return;
    }
    if (e.altKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      const i = ordered.findIndex((a) => a.id === selectedId);
      const next = ordered[Math.max(0, Math.min(ordered.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) select(next.id);
      return;
    }
    if (!selected || busy) return;
    if (e.ctrlKey && e.key === "Enter") {
      e.preventDefault();
      isActive(selected.state) ? stop(selected.id) : start(selected.id);
    } else if (e.ctrlKey && e.shiftKey && k === "r") {
      e.preventDefault();
      if (isActive(selected.state)) restart(selected.id);
    }
  }

  const commands = $derived.by<Command[]>(() => {
    const out: Command[] = [];
    if (selected) {
      const s = selected;
      if (isActive(s.state)) {
        out.push({ id: "stop", group: s.name, label: `Stop ${s.name}`, hint: "Ctrl+Enter", run: () => stop(s.id) });
        out.push({ id: "restart", group: s.name, label: `Restart ${s.name}`, hint: "Ctrl+Shift+R", run: () => restart(s.id) });
      } else {
        out.push({ id: "start", group: s.name, label: `Start ${s.name}`, hint: "Ctrl+Enter", run: () => start(s.id) });
      }
      tabs.forEach(([t, label], i) =>
        out.push({ id: `tab-${t}`, group: s.name, label: `Open ${label}`, hint: `Ctrl+${i + 1}`, run: () => (tab = t) }),
      );
      if (!s.task?.running) {
        for (const t of s.tasks) {
          out.push({
            id: `task-${t.name}`,
            group: s.name,
            label: `Run task ${t.name}`,
            run: () => run(() => api.runTask(s.id, { name: t.name })).then(() => (tab = "tasks")),
          });
        }
      }
      const url = appUrl(s);
      if (url) out.push({ id: "url", group: s.name, label: `Open ${url}`, run: () => openUrl(url) });
      out.push({ id: "folder", group: s.name, label: "Open folder", run: () => openPath(s.cwd) });
    }
    for (const a of ordered) {
      out.push({ id: `go-${a.id}`, group: "Apps", label: `${a.name}  (${stateLabel[a.state]})`, run: () => select(a.id) });
      out.push({ id: `logs-${a.id}`, group: "Apps", label: `${a.name} logs`, run: () => select(a.id, "logs") });
    }
    out.push({ id: "add", group: "General", label: "Add app", hint: "Ctrl+N", run: () => (adding = true) });
    out.push({ id: "agent", group: "General", label: "Agent settings (port)", run: () => (agentOpen = true) });
    if (updates.available) {
      const v = updates.available.version;
      out.push({ id: "update", group: "General", label: `Update to ${v}`, run: () => (updateOpen = true) });
    } else if (updates.policy?.enabled !== false) {
      out.push({ id: "check-update", group: "General", label: "Check for updates", run: () => (agentOpen = true, checkForUpdates(true)) });
    }
    out.push({ id: "theme-system", group: "General", label: "Theme: follow system", run: () => applyTheme("system") });
    out.push({ id: "theme-light", group: "General", label: "Theme: light", run: () => applyTheme("light") });
    out.push({ id: "theme-dark", group: "General", label: "Theme: dark", run: () => applyTheme("dark") });
    return out;
  });

  const restartCountdown = $derived(
    selected?.state === "restarting" && selected.next_restart_at
      ? Math.max(0, Math.ceil((selected.next_restart_at - now) / 1000))
      : null,
  );
</script>

<svelte:window onkeydown={onKey} />

<div class="flex h-full flex-col">
  <header class="flex h-12 shrink-0 items-center gap-3 border-b border-border px-4">
    <span class="font-semibold tracking-tight">Keeprun</span>
    {#if conn?.env === "dev"}
      <span
        class="rounded bg-warn/15 px-1.5 py-0.5 text-[11px] font-semibold uppercase tracking-wide text-warn"
        title="Agent {conn.baseUrl}, data in {conn.dataDir}">dev</span
      >
    {/if}
    <button
      class="mx-auto flex h-8 w-80 max-w-[40vw] items-center gap-2 rounded-md border border-border bg-bg-subtle px-3 text-[13px] text-fg-muted hover:bg-bg-hover"
      onclick={() => (palette = true)}
    >
      <Search size={14} />
      <span class="flex-1 text-left">Search</span>
      <span class="kbd">Ctrl K</span>
    </button>
    {#if updates.available}
      <button class="btn !border-accent !text-accent" onclick={() => (updateOpen = true)} title="Version {updates.available.version} is available">
        <ArrowUpCircle size={15} /> Update
      </button>
    {/if}
    <button class="btn-icon" onclick={() => (agentOpen = true)} title="Agent: {conn?.baseUrl ?? 'not connected'}. Change port.">
      <Server size={16} />
    </button>
    <button
      class="btn-icon"
      onclick={cycleTheme}
      title="Theme: {theme.value === 'system' ? 'follow system' : theme.value}. Click to change."
    >
      {#if theme.value === "light"}<Sun size={16} />{:else if theme.value === "dark"}<Moon size={16} />{:else}<Monitor size={16} />{/if}
    </button>
    <button class="btn btn-primary" onclick={() => (adding = true)} title="Add app (Ctrl+N)">
      <Plus size={15} /> Add app
    </button>
  </header>

  {#if agentError}
    <div class="flex items-start gap-2 border-b border-border px-4 py-2 text-[13px] alert !rounded-none" role="alert">
      <TriangleAlert size={16} class="mt-0.5 shrink-0" />
      <span class="selectable">{agentError}</span>
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <nav class="flex w-60 shrink-0 flex-col overflow-auto border-r border-border bg-bg-subtle py-2" aria-label="Apps">
      {#each groups as [group, list]}
        <p class="px-4 pb-1 pt-3 text-[11px] font-semibold uppercase tracking-wider text-fg-muted">{group || "Apps"}</p>
        {#each list as app (app.id)}
          <button
            class="mx-2 flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] hover:bg-bg-hover {app.id ===
            selectedId
              ? '!bg-bg-active font-medium'
              : ''}"
            onclick={() => select(app.id)}
            title="{app.name}: {stateLabel[app.state]}"
            aria-current={app.id === selectedId}
          >
            <StatusDot state={app.state} />
            <span class="flex-1 truncate">{app.name}</span>
            {#if app.state === "running" && app.stats?.ports[0]}
              <span class="text-[11px] tabular-nums text-fg-muted">:{app.stats.ports[0]}</span>
            {/if}
          </button>
        {/each}
      {/each}
      {#if loaded && apps.length === 0 && !agentError}
        <p class="px-4 pt-3 text-xs text-fg-muted">No apps yet.</p>
      {/if}
      <div class="mt-auto px-4 pt-4 text-[11px] text-fg-muted">
        <span class="kbd">Alt</span> + <span class="kbd">Up</span>/<span class="kbd">Down</span> to switch
      </div>
    </nav>

    <main class="flex min-w-0 flex-1 flex-col">
      {#if selected}
        <section class="border-b border-border px-6 pt-4">
          <div class="flex items-center gap-3">
            <h1 class="truncate text-lg font-semibold">{selected.name}</h1>
            <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
              <StatusDot state={selected.state} />
              {stateLabel[selected.state]}{selected.state === "exited" && selected.exit_code !== null
                ? ` (code ${selected.exit_code})`
                : ""}{restartCountdown !== null ? ` in ${restartCountdown}s` : ""}
            </span>
            <div class="flex-1"></div>
            {#if isActive(selected.state)}
              <button class="btn" disabled={busy} onclick={() => stop(selected!.id)} title="Stop (Ctrl+Enter)">
                <Square size={13} /> Stop
              </button>
              <button
                class="btn"
                disabled={busy || selected.state === "restarting"}
                onclick={() => restart(selected!.id)}
                title="Restart (Ctrl+Shift+R)"
              >
                <RotateCw size={13} /> Restart
              </button>
            {:else}
              <button class="btn btn-primary" disabled={busy} onclick={() => start(selected!.id)} title="Start (Ctrl+Enter)">
                <Play size={13} /> Start
              </button>
            {/if}
          </div>
          {#if actionError}
            <p class="selectable alert mt-3" role="alert">{actionError}</p>
          {/if}
          <div class="mt-3 flex gap-1" role="tablist">
            {#each tabs as [t, label], i}
              <button
                role="tab"
                aria-selected={tab === t}
                class="-mb-px border-b-2 px-3 py-2 text-[13px] {tab === t
                  ? 'border-accent font-medium text-fg'
                  : 'border-transparent text-fg-muted hover:text-fg'}"
                onclick={() => (tab = t)}
                title="Ctrl+{i + 1}">{label}</button
              >
            {/each}
          </div>
        </section>

        {#key selected.id}
          {#if tab === "overview"}
            <Overview app={selected} {now} onshowevents={() => (tab = "events")} />
          {:else if tab === "logs"}
            <LogPanel appId={selected.id} />
          {:else if tab === "tasks"}
            <TasksTab app={selected} {now} onchanged={refresh} />
          {:else if tab === "environment"}
            <EnvEditor app={selected} onsaved={refresh} />
          {:else if tab === "settings"}
            <SettingsTab app={selected} onsaved={refresh} onremoved={() => ((selectedId = null), (tab = "overview"), refresh())} />
          {:else}
            <div class="flex-1 overflow-auto px-6 py-4"><EventsList appId={selected.id} limit={300} /></div>
          {/if}
        {/key}
      {:else if loaded && !agentError}
        <div class="m-auto flex flex-col items-center gap-3 text-center">
          <p class="text-base font-semibold">No apps yet</p>
          <p class="max-w-xs text-[13px] text-fg-muted">Pick a folder. Keeprun detects how to run it and keeps it running.</p>
          <button class="btn btn-primary mt-1" onclick={() => (adding = true)}><Plus size={15} /> Add your first app</button>
          <p class="mt-2 text-xs text-fg-muted"><span class="kbd">Ctrl N</span> add app, <span class="kbd">Ctrl K</span> search</p>
        </div>
      {/if}
    </main>
  </div>
</div>

{#if adding}
  <AddAppDialog
    onclose={() => (adding = false)}
    onadded={(app) => {
      adding = false;
      select(app.id, "logs");
      refresh();
    }}
  />
{/if}

{#if agentOpen}
  <AgentDialog {conn} onclose={() => (agentOpen = false)} onchanged={refresh} onupdate={() => ((agentOpen = false), (updateOpen = true))} />
{/if}

{#if updateOpen}
  <UpdateDialog onclose={() => (updateOpen = false)} />
{/if}

{#if palette}
  <CommandPalette {commands} onclose={() => (palette = false)} />
{/if}
