import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const view = new URLSearchParams(location.search).get("view") === "flyout" ? "flyout" : "dashboard";
const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
mount(App, { target, props: { view } });
