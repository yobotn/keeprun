<script lang="ts">
  import { X } from "@lucide/svelte";
  import { languages, locale, setLocale, systemLang, t } from "./i18n/index.svelte";
  import { applyTheme, theme, type Theme } from "./theme.svelte";
  import { prefs, setEditor, setTray, type Editor } from "./prefs.svelte";

  let { onclose }: { onclose: () => void } = $props();
  let dialog: HTMLDialogElement;

  $effect(() => {
    dialog.showModal();
  });

  const themes: [Theme, "prefs.themeSystem" | "prefs.themeLight" | "prefs.themeDark"][] = [
    ["system", "prefs.themeSystem"],
    ["light", "prefs.themeLight"],
    ["dark", "prefs.themeDark"],
  ];
</script>

<dialog bind:this={dialog} {onclose} class="dialog w-[520px] max-w-[calc(100vw-32px)]">
  <div class="flex flex-col gap-5 p-5">
    <div class="flex items-center justify-between">
      <h2 class="text-base font-semibold">{t("prefs.title")}</h2>
      <button type="button" class="btn-icon" onclick={() => dialog.close()} title={t("common.close")}><X size={16} /></button>
    </div>

    <div>
      <label class="label" for="p-lang">{t("prefs.language")}</label>
      <select id="p-lang" class="input" value={locale.choice} onchange={(e) => setLocale(e.currentTarget.value)}>
        <option value="system">{t("prefs.languageSystem", { language: systemLang.name })}</option>
        {#each languages as l (l.code)}
          <option value={l.code} lang={l.code} dir={l.rtl ? "rtl" : "ltr"}>{l.name}</option>
        {/each}
      </select>
    </div>

    <fieldset>
      <legend class="label">{t("prefs.theme")}</legend>
      <div class="inline-flex rounded-md border border-border p-0.5" role="radiogroup">
        {#each themes as [value, key] (value)}
          <button
            type="button"
            role="radio"
            aria-checked={theme.value === value}
            class="h-7 rounded px-3 text-[13px] {theme.value === value ? 'bg-bg-active font-medium' : 'text-fg-muted hover:text-fg'}"
            onclick={() => applyTheme(value)}>{t(key)}</button
          >
        {/each}
      </div>
    </fieldset>

    <div>
      <label class="label" for="p-editor">{t("prefs.editor")}</label>
      <select id="p-editor" class="input" value={prefs.editor} onchange={(e) => setEditor(e.currentTarget.value as Editor)}>
        <option value="auto">{t("prefs.editorAuto")}</option>
        <option value="vscode">Visual Studio Code</option>
        <option value="cursor">Cursor</option>
        <option value="windsurf">Windsurf</option>
        <option value="default">{t("prefs.editorDefault")}</option>
      </select>
      <p class="hint mt-1">{t("prefs.editorHint")}</p>
    </div>

    <label class="flex items-start gap-3">
      <span class="switch mt-0.5">
        <input type="checkbox" role="switch" checked={prefs.tray} onchange={(e) => setTray(e.currentTarget.checked)} />
        <span aria-hidden="true"></span>
      </span>
      <span>
        <span class="text-[13px] font-medium">{t("prefs.tray")}</span>
        <span class="hint block">{t(prefs.tray ? "prefs.trayHint" : "prefs.trayOffHint")}</span>
      </span>
    </label>
  </div>
</dialog>
