<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { openPath, openUrl } from "@tauri-apps/plugin-opener";
  import {
    ArrowUpCircle,
    Ban,
    ChevronDown,
    CirclePower,
    Monitor,
    Moon,
    Play,
    Plus,
    RotateCw,
    Search,
    Server,
    Settings2,
    Square,
    Sun,
    TriangleAlert,
  } from "@lucide/svelte";
  import { api, appUrl, config, connection, desktop, isActive, samePath, stateText, type AppView, type Connection } from "./lib/api";
  import { cycleTheme, applyTheme, theme } from "./lib/theme.svelte";
  import { languages, locale, setLocale, t, type Key } from "./lib/i18n/index.svelte";
  import { prefs, rememberSelection, setTray, syncTrayText, toggleSection } from "./lib/prefs.svelte";
  import StatusDot from "./lib/StatusDot.svelte";
  import Overview from "./lib/Overview.svelte";
  import LogPanel from "./lib/LogPanel.svelte";
  import EnvEditor from "./lib/EnvEditor.svelte";
  import SettingsTab from "./lib/SettingsTab.svelte";
  import EventsList from "./lib/EventsList.svelte";
  import AddAppDialog from "./lib/AddAppDialog.svelte";
  import AgentDialog from "./lib/AgentDialog.svelte";
  import PreferencesDialog from "./lib/PreferencesDialog.svelte";
  import TasksTab from "./lib/TasksTab.svelte";
  import UpdateDialog from "./lib/UpdateDialog.svelte";
  import { checkForUpdates, scheduleUpdateChecks, updates } from "./lib/updates.svelte";
  import CommandPalette, { type Command } from "./lib/CommandPalette.svelte";

  type Tab = "overview" | "logs" | "tasks" | "environment" | "settings" | "events";
  const tabs: Tab[] = ["overview", "logs", "tasks", "environment", "settings", "events"];
  const tabLabel = (tab: Tab) => t(`tab.${tab}`);

  let conn = $state<Connection | null>(null);
  let apps = $state<AppView[]>([]);
  /** Apps printing errors while a coding agent still works in their folder. Their errors wait. */
  let waiting = $state<string[]>([]);
  const showsIssue = (a: AppView) => !!a.issue && !waiting.includes(a.id);
  let selectedId = $state<string | null>(null);
  let tab = $state<Tab>(tabs.includes(prefs.lastTab as Tab) ? (prefs.lastTab as Tab) : "overview");
  let agentError = $state("");
  let actionError = $state("");
  let busy = $state(false);
  let adding = $state(false);
  /** Folder to add, from the Explorer menu. */
  let addFolder = $state<string | undefined>();
  let palette = $state(false);
  let agentOpen = $state(false);
  let prefsOpen = $state(false);
  let updateOpen = $state(false);

  $effect(() => scheduleUpdateChecks());
  let loaded = $state(false);
  let now = $state(Date.now());

  const selected = $derived(apps.find((a) => a.id === selectedId) ?? null);

  // Sidebar: running, off and disabled apps in separate sections. Inside a section, apps are
  // grouped by first tag (untagged first), then sorted by name.
  type SectionId = "running" | "off" | "disabled";
  const sectionOf = (a: AppView): SectionId => (a.disabled ? "disabled" : isActive(a.state) ? "running" : "off");

  function byTag(list: AppView[]): [string, AppView[]][] {
    const map = new Map<string, AppView[]>();
    for (const a of [...list].sort((x, y) => x.name.localeCompare(y.name, locale.lang.code))) {
      const g = a.tags[0] ?? "";
      if (!map.has(g)) map.set(g, []);
      map.get(g)!.push(a);
    }
    return [...map.entries()].sort(([a], [b]) => (a === "" ? -1 : b === "" ? 1 : a.localeCompare(b, locale.lang.code)));
  }

  const sections = $derived(
    (["running", "off", "disabled"] as SectionId[])
      .map((id) => {
        const list = apps.filter((a) => sectionOf(a) === id);
        return { id, count: list.length, groups: byTag(list) };
      })
      // An empty Disabled section is noise. Running and Off always show, so apps have a place to move to.
      .filter((s) => s.id !== "disabled" || s.count > 0),
  );
  const ordered = $derived(
    sections.flatMap((s) => (prefs.collapsed.includes(s.id) ? [] : s.groups.flatMap(([, list]) => list))),
  );
  const sectionLabel: Record<SectionId, Key> = { running: "sidebar.running", off: "sidebar.off", disabled: "sidebar.disabled" };
  const sectionEmpty: Partial<Record<SectionId, Key>> = { running: "sidebar.noneRunning", off: "sidebar.noneOff" };

  async function refresh() {
    try {
      conn = await connection();
      apps = await api.list();
      agentError = "";
      if (!selectedId || !apps.some((a) => a.id === selectedId)) {
        // Open the app the user looked at last, as long as it still exists.
        const last = prefs.lastApp && apps.some((a) => a.id === prefs.lastApp) ? prefs.lastApp : null;
        selectedId = last ?? ordered[0]?.id ?? apps[0]?.id ?? null;
        // The error belonged to the app that is gone, not to the one shown now.
        actionError = "";
      }
    } catch (e) {
      agentError = (e as Error).message;
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    const pollWaiting = () =>
      desktop
        .agentWaiting()
        .then((ids) => (waiting = ids))
        .catch(() => {});
    refresh();
    pollWaiting();
    const poll = setInterval(() => {
      refresh();
      pollWaiting();
    }, config.pollMs);
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => {
      clearInterval(poll);
      clearInterval(tick);
    };
  });

  // Explorer "Open with Keeprun": run the app in that folder, or add it if it is new.
  async function openFolder(path: string) {
    await refresh();
    const app = apps.find((a) => samePath(a.cwd, path));
    if (!app) {
      palette = agentOpen = updateOpen = prefsOpen = false;
      addFolder = path;
      adding = true;
      return;
    }
    select(app.id, "logs");
    // A disabled app shows its notice instead. A running one is already where it should be.
    if (!app.disabled && !isActive(app.state)) await start(app.id);
  }

  $effect(() => {
    let stop: (() => void) | undefined;
    let closed = false;
    listen<string>("open-folder", (e) => openFolder(e.payload)).then((un) => (closed ? un() : (stop = un)));
    desktop
      .takeLaunchFolder()
      .then((path) => {
        if (path) openFolder(path);
      })
      .catch(() => {});
    return () => {
      closed = true;
      stop?.();
    };
  });

  // Remember the selection once apps loaded, so a failed first load does not forget it.
  $effect(() => {
    if (loaded && selectedId) rememberSelection(selectedId, tab);
  });

  // The shell draws the notification area icon. Give it the setting and translated text.
  $effect(() => setTray(prefs.tray));
  $effect(() => {
    locale.lang;
    syncTrayText(apps.length);
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
  const disable = (a: AppView) =>
    run(async () => {
      if (isActive(a.state)) await api.stop(a.id);
      await api.update(a.id, { disabled: true });
    });
  const enable = (id: string) => run(() => api.update(id, { disabled: false }));

  function select(id: string, to?: Tab) {
    if (id !== selectedId) actionError = "";
    selectedId = id;
    if (to) tab = to;
  }

  function onKey(e: KeyboardEvent) {
    if (adding || palette || agentOpen || updateOpen || prefsOpen) return;
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
      tab = tabs[Number(e.key) - 1];
      return;
    }
    if (e.altKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      const i = ordered.findIndex((a) => a.id === selectedId);
      const next = ordered[Math.max(0, Math.min(ordered.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) select(next.id);
      return;
    }
    if (!selected || busy || selected.disabled) return;
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
      const name = s.name;
      if (s.disabled) {
        out.push({ id: "enable", group: name, label: t("cmd.enable", { name }), run: () => enable(s.id) });
      } else if (isActive(s.state)) {
        out.push({ id: "stop", group: name, label: t("cmd.stop", { name }), hint: "Ctrl+Enter", run: () => stop(s.id) });
        out.push({ id: "restart", group: name, label: t("cmd.restart", { name }), hint: "Ctrl+Shift+R", run: () => restart(s.id) });
      } else {
        out.push({ id: "start", group: name, label: t("cmd.start", { name }), hint: "Ctrl+Enter", run: () => start(s.id) });
      }
      if (!s.disabled) out.push({ id: "disable", group: name, label: t("cmd.disable", { name }), run: () => disable(s) });
      tabs.forEach((tb, i) =>
        out.push({ id: `tab-${tb}`, group: name, label: t("cmd.openTab", { tab: tabLabel(tb) }), hint: `Ctrl+${i + 1}`, run: () => (tab = tb) }),
      );
      if (!s.task?.running) {
        for (const task of s.tasks) {
          out.push({
            id: `task-${task.name}`,
            group: name,
            label: t("cmd.runTask", { name: task.name }),
            run: () => run(() => api.runTask(s.id, { name: task.name })).then(() => (tab = "tasks")),
          });
        }
      }
      const url = appUrl(s);
      if (url) out.push({ id: "url", group: name, label: t("cmd.openUrl", { url }), run: () => openUrl(url) });
      out.push({ id: "folder", group: name, label: t("cmd.openFolder"), run: () => openPath(s.cwd) });
    }
    const appsGroup = t("palette.groupApps");
    for (const a of ordered) {
      const state = a.disabled ? t("state.disabled") : stateText(a.state);
      out.push({ id: `go-${a.id}`, group: appsGroup, label: `${a.name}  (${state})`, run: () => select(a.id) });
      out.push({ id: `logs-${a.id}`, group: appsGroup, label: t("cmd.appLogs", { name: a.name }), run: () => select(a.id, "logs") });
    }
    const general = t("palette.groupGeneral");
    out.push({ id: "add", group: general, label: t("cmd.add"), hint: "Ctrl+N", run: () => (adding = true) });
    out.push({ id: "prefs", group: general, label: t("cmd.preferences"), run: () => (prefsOpen = true) });
    out.push({ id: "agent", group: general, label: t("cmd.agent"), run: () => (agentOpen = true) });
    if (updates.available) {
      const version = updates.available.version;
      out.push({ id: "update", group: general, label: t("cmd.update", { version }), run: () => (updateOpen = true) });
    } else if (updates.policy?.enabled !== false) {
      out.push({ id: "check-update", group: general, label: t("cmd.checkUpdate"), run: () => ((agentOpen = true), checkForUpdates(true)) });
    }
    out.push({ id: "theme-system", group: general, label: t("cmd.themeSystem"), run: () => applyTheme("system") });
    out.push({ id: "theme-light", group: general, label: t("cmd.themeLight"), run: () => applyTheme("light") });
    out.push({ id: "theme-dark", group: general, label: t("cmd.themeDark"), run: () => applyTheme("dark") });
    for (const l of languages) {
      out.push({ id: `lang-${l.code}`, group: general, label: t("cmd.language", { language: l.name }), run: () => setLocale(l.code) });
    }
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
        title={t("header.devBadgeHint", { url: conn.baseUrl, dir: conn.dataDir })}>dev</span
      >
    {/if}
    <button
      class="mx-auto flex h-8 w-80 max-w-[40vw] items-center gap-2 rounded-md border border-border bg-bg-subtle px-3 text-[13px] text-fg-muted hover:bg-bg-hover"
      onclick={() => (palette = true)}
    >
      <Search size={14} />
      <span class="flex-1 text-start">{t("header.search")}</span>
      <span class="kbd" dir="ltr">Ctrl K</span>
    </button>
    {#if updates.available}
      <button
        class="btn !border-accent !text-accent"
        onclick={() => (updateOpen = true)}
        title={t("header.updateAvailable", { version: updates.available.version })}
      >
        <ArrowUpCircle size={15} />
        {t("header.update")}
      </button>
    {/if}
    <button
      class="btn-icon"
      onclick={() => (agentOpen = true)}
      title={t("header.agent", { url: conn?.baseUrl ?? t("header.agentNotConnected") })}
    >
      <Server size={16} />
    </button>
    <button
      class="btn-icon"
      onclick={cycleTheme}
      title={t("theme.title", { theme: t(`theme.${theme.value}`) })}
    >
      {#if theme.value === "light"}<Sun size={16} />{:else if theme.value === "dark"}<Moon size={16} />{:else}<Monitor size={16} />{/if}
    </button>
    <button class="btn-icon" onclick={() => (prefsOpen = true)} title={t("header.preferences")}>
      <Settings2 size={16} />
    </button>
    <button class="btn btn-primary" onclick={() => (adding = true)} title={t("header.addAppHint")}>
      <Plus size={15} />
      {t("header.addApp")}
    </button>
  </header>

  {#if agentError}
    <div class="flex items-start gap-2 border-b border-border px-4 py-2 text-[13px] alert !rounded-none" role="alert">
      <TriangleAlert size={16} class="mt-0.5 shrink-0" />
      <span class="selectable">{agentError}</span>
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <nav class="flex w-64 shrink-0 flex-col overflow-auto border-e border-border bg-bg-subtle py-2" aria-label={t("sidebar.label")}>
      {#if loaded && apps.length === 0 && !agentError}
        <p class="px-4 pt-3 text-xs text-fg-muted">{t("sidebar.noApps")}</p>
      {:else if apps.length}
        {#each sections as section (section.id)}
          {@const collapsed = prefs.collapsed.includes(section.id)}
          {@const label = t(sectionLabel[section.id])}
          <button
            class="mx-2 mt-2 flex items-center gap-1.5 rounded-md px-2 py-1 text-[11px] font-semibold uppercase tracking-wider text-fg-muted hover:bg-bg-hover hover:text-fg"
            onclick={() => toggleSection(section.id)}
            aria-expanded={!collapsed}
            title={t(collapsed ? "sidebar.expand" : "sidebar.collapse", { section: label })}
          >
            <ChevronDown size={13} class="shrink-0 transition-transform {collapsed ? 'chevron-collapsed' : ''}" />
            <span class="flex-1 text-start">{label}</span>
            <span class="tabular-nums font-normal">{section.count}</span>
          </button>
          {#if !collapsed}
            {#if section.count === 0 && sectionEmpty[section.id]}
              <p class="px-4 pb-1 pt-0.5 text-xs text-fg-muted">{t(sectionEmpty[section.id]!)}</p>
            {/if}
            {#each section.groups as [group, list] (group)}
              {#if group}
                <p class="truncate px-5 pb-0.5 pt-1.5 text-[11px] text-fg-muted">{group}</p>
              {/if}
              {#each list as app (app.id)}
                {@const state = app.disabled ? t("state.disabled") : stateText(app.state)}
                <button
                  class="mx-2 flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-start text-[13px] hover:bg-bg-hover {app.id ===
                  selectedId
                    ? '!bg-bg-active font-medium'
                    : ''} {app.disabled ? 'text-fg-muted' : ''}"
                  onclick={() => select(app.id)}
                  title="{app.name}: {state}"
                  aria-current={app.id === selectedId}
                >
                  {#if app.disabled}
                    <Ban size={10} class="shrink-0 text-fg-muted" aria-hidden="true" />
                  {:else}
                    <StatusDot state={app.state} issue={showsIssue(app)} />
                  {/if}
                  <span class="flex-1 truncate">{app.name}</span>
                  {#if app.state === "running" && app.stats?.ports[0]}
                    <span class="text-[11px] tabular-nums text-fg-muted" dir="ltr">:{app.stats.ports[0]}</span>
                  {/if}
                </button>
              {/each}
            {/each}
          {/if}
        {/each}
      {/if}
      <div class="mt-auto px-4 pt-4 text-[11px] text-fg-muted">
        <span dir="ltr"><span class="kbd">Alt</span> + <span class="kbd">Up</span>/<span class="kbd">Down</span></span>
        {t("sidebar.switchHint")}
      </div>
    </nav>

    <main class="flex min-w-0 flex-1 flex-col">
      {#if selected}
        <section class="border-b border-border px-6 pt-4">
          <div class="flex items-center gap-3">
            <h1 class="truncate text-lg font-semibold">{selected.name}</h1>
            <span class="inline-flex items-center gap-1.5 text-[13px] text-fg-muted">
              {#if selected.disabled}
                <Ban size={12} />
                {t("state.disabled")}
              {:else}
                <StatusDot state={selected.state} issue={showsIssue(selected)} />
                {stateText(selected.state)}
                {#if selected.state === "running" && selected.health && !selected.health.ok}<span class="text-warn">· {t("app.notResponding")}</span>{:else if showsIssue(selected) && selected.state === "running"}<span class="text-warn">· {t("app.printingErrors")}</span>{/if}
                {#if selected.state === "exited" && selected.exit_code !== null}{t("app.exitCode", { code: selected.exit_code })}{/if}
                {#if restartCountdown !== null}{t("app.restartIn", { seconds: restartCountdown })}{/if}
              {/if}
            </span>
            <div class="flex-1"></div>
            {#if selected.disabled}
              <button class="btn btn-primary" disabled={busy} onclick={() => enable(selected!.id)} title={t("app.enableHint")}>
                <CirclePower size={13} />
                {t("app.enable")}
              </button>
            {:else}
              {#if isActive(selected.state)}
                <button class="btn" disabled={busy} onclick={() => stop(selected!.id)} title={t("app.stopHint")}>
                  <Square size={13} />
                  {t("app.stop")}
                </button>
                <button
                  class="btn"
                  disabled={busy || selected.state === "restarting"}
                  onclick={() => restart(selected!.id)}
                  title={t("app.restartHint")}
                >
                  <RotateCw size={13} />
                  {t("app.restart")}
                </button>
              {:else}
                <button class="btn btn-primary" disabled={busy} onclick={() => start(selected!.id)} title={t("app.startHint")}>
                  <Play size={13} class="flip-rtl" />
                  {t("app.start")}
                </button>
              {/if}
              <button class="btn" disabled={busy} onclick={() => disable(selected!)} title={t("app.disableHint")}>
                <Ban size={13} />
                {t("app.disable")}
              </button>
            {/if}
          </div>
          {#if actionError}
            <p class="selectable alert mt-3" role="alert">{actionError}</p>
          {/if}
          {#if selected.disabled}
            <p class="alert alert-warn mt-3" role="status">{t("app.disabledNotice")}</p>
          {/if}
          <div class="mt-3 flex gap-1" role="tablist">
            {#each tabs as tb, i}
              <button
                role="tab"
                aria-selected={tab === tb}
                class="-mb-px border-b-2 px-3 py-2 text-[13px] {tab === tb
                  ? 'border-accent font-medium text-fg'
                  : 'border-transparent text-fg-muted hover:text-fg'}"
                onclick={() => (tab = tb)}
                title="Ctrl+{i + 1}">{tabLabel(tb)}</button
              >
            {/each}
          </div>
        </section>

        {#key selected.id}
          {#if tab === "overview"}
            <Overview app={selected} {now} agentWorking={waiting.includes(selected.id)} onshowevents={() => (tab = "events")} />
          {:else if tab === "logs"}
            <LogPanel appId={selected.id} cwd={selected.cwd} />
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
          <p class="text-base font-semibold">{t("app.emptyTitle")}</p>
          <p class="max-w-xs text-[13px] text-fg-muted">{t("app.emptyBody")}</p>
          <button class="btn btn-primary mt-1" onclick={() => (adding = true)}><Plus size={15} /> {t("app.emptyAction")}</button>
          <p class="mt-2 text-xs text-fg-muted">
            {t("app.emptyKeys", { addKey: "Ctrl N", searchKey: "Ctrl K" })}
          </p>
        </div>
      {/if}
    </main>
  </div>
</div>

{#if adding}
  <AddAppDialog
    folder={addFolder}
    onclose={() => ((adding = false), (addFolder = undefined))}
    onadded={(app) => {
      adding = false;
      addFolder = undefined;
      select(app.id, "logs");
      refresh();
    }}
  />
{/if}

{#if agentOpen}
  <AgentDialog {conn} onclose={() => (agentOpen = false)} onchanged={refresh} onupdate={() => ((agentOpen = false), (updateOpen = true))} />
{/if}

{#if prefsOpen}
  <PreferencesDialog onclose={() => (prefsOpen = false)} />
{/if}

{#if updateOpen}
  <UpdateDialog onclose={() => (updateOpen = false)} />
{/if}

{#if palette}
  <CommandPalette {commands} onclose={() => (palette = false)} />
{/if}
