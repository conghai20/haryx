import { mount } from "svelte";
import "@fontsource-variable/geist";
import "@fontsource-variable/geist-mono";
import "@fontsource-variable/space-grotesk";
import "./app.css";
import App from "./App.svelte";
import { followSystemTheme } from "./lib/theme";

followSystemTheme(document.documentElement, window.matchMedia("(prefers-color-scheme: dark)"));
mount(App, { target: document.getElementById("app")! });
