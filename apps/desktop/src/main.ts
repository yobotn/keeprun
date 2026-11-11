import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { applyTheme, theme } from "./lib/theme.svelte";
import { applyDocument } from "./lib/i18n/index.svelte";

applyTheme(theme.value);
applyDocument();

const app = mount(App, { target: document.getElementById("app")! });

export default app;
