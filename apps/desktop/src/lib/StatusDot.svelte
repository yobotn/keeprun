<script lang="ts">
  import type { AppState } from "./api";

  /** `issue`: running, but printing errors. */
  let { state, issue = false, size = 8 }: { state: AppState; issue?: boolean; size?: number } = $props();

  const color: Record<AppState, string> = {
    running: "var(--accent)",
    starting: "var(--warn)",
    stopping: "var(--warn)",
    restarting: "var(--warn)",
    exited: "var(--fg-muted)",
    failed: "var(--danger)",
    stopped: "color-mix(in srgb, var(--fg-muted) 45%, transparent)",
  };
</script>

<span
  class="inline-block shrink-0 rounded-full"
  class:pulse={state === "starting" || state === "restarting" || state === "stopping"}
  style="width:{size}px;height:{size}px;background:{issue && state === 'running' ? 'var(--warn)' : color[state]}"
  aria-hidden="true"
></span>

<style>
  @media (prefers-reduced-motion: no-preference) {
    .pulse {
      animation: pulse 1.2s ease-in-out infinite;
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
</style>
