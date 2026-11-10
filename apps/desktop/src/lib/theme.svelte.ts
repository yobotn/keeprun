// Theme preference: light, dark or follow the system. Remembered per user.
export type Theme = "system" | "light" | "dark";

const KEY = "keeprun.theme";

function read(): Theme {
  try {
    const t = localStorage.getItem(KEY);
    if (t === "light" || t === "dark" || t === "system") return t;
  } catch {
    // Storage can be unavailable. Fall back to the system theme.
  }
  return "system";
}

export const theme = $state({ value: read() });

export function applyTheme(t: Theme) {
  theme.value = t;
  if (t === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", t);
  try {
    localStorage.setItem(KEY, t);
  } catch {
    // Not persisted. The choice still applies for this session.
  }
}

export function cycleTheme() {
  applyTheme(theme.value === "system" ? "light" : theme.value === "light" ? "dark" : "system");
}
