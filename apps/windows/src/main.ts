import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const param = new URLSearchParams(location.search).get("view");
const view = param === "flyout" || param === "login" ? param : "dashboard";
const target = document.getElementById("app");
if (!target) throw new Error("missing #app");
mount(App, { target, props: { view } });
