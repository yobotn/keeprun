import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { applyTheme, theme } from "./lib/theme.svelte";

applyTheme(theme.value);

const app = mount(App, { target: document.getElementById("app")! });

export default app;
