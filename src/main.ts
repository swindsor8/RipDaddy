import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Failed to mount RipDaddy: missing #app container.");
}

function renderFatalError(err: unknown): void {
  const message =
    err instanceof Error ? `${err.name}: ${err.message}\n${err.stack ?? ""}` : String(err);
  target!.innerHTML = `<pre style="padding:24px;font-family:monospace;font-size:12px;color:#b91c1c;background:#fef2f2;white-space:pre-wrap;margin:16px;border:1px solid #fecaca;border-radius:8px;">RipDaddy failed to render:\n\n${message.replace(/</g, "&lt;")}</pre>`;
}

try {
  mount(App, { target });
} catch (err) {
  renderFatalError(err);
}

