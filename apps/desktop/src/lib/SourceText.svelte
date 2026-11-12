<script lang="ts">
  // A line of app output with its file paths (stack traces, compiler errors) as links that
  // open the file at that line in the code editor.
  import { desktop } from "./api";
  import { t } from "./i18n/index.svelte";
  import { prefs } from "./prefs.svelte";
  import { splitSourceLinks, type SourceLink } from "./sourceLinks";

  let { text, cwd, onerror }: { text: string; cwd: string; onerror: (message: string) => void } = $props();

  const segments = $derived(splitSourceLinks(text));

  async function open(e: MouseEvent, l: SourceLink) {
    e.preventDefault();
    // A drag to select text ends in a click too. Let it copy instead.
    if (window.getSelection()?.toString()) return;
    try {
      await desktop.openInEditor(cwd, l.path, l.line, l.col, prefs.editor);
    } catch (err) {
      const code = String(err);
      onerror(
        code === "not-found"
          ? t("logs.fileNotFound", { path: l.path })
          : code === "no-editor"
            ? t("logs.noEditor")
            : t("logs.cannotOpenFile", { path: l.path, error: code }),
      );
    }
  }
</script>

{#each segments as s}{#if typeof s === "string"}{s}{:else}<a
      href="#{s.line}"
      class="underline decoration-dotted underline-offset-2 hover:text-accent hover:decoration-solid"
      title={t("logs.openFile", { line: s.line })}
      onclick={(e) => open(e, s)}>{s.text}</a
    >{/if}{/each}
