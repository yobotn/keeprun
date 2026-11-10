/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_APP_ENV: "dev" | "prod";
  readonly VITE_STATUS_POLL_MS: string;
  readonly VITE_LOG_MAX_LINES: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
