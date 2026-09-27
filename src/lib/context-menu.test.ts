// @vitest-environment happy-dom
import { beforeEach, describe, expect, it } from "vitest";
import { installContextMenu, keepsNativeMenu } from "./context-menu";

/** The chrome a right-click lands on when it misses, built the way the app builds it. */
function shell(inner: string): HTMLElement {
  document.body.innerHTML = `<div class="app-shell">${inner}</div>`;
  return document.querySelector(".app-shell") as HTMLElement;
}

describe("Context menu", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("denies the webview menu on the chrome, which is where Reload is reachable", () => {
    const row = shell(`<div class="workdir-item"><span class="workdir-name">main</span></div>`);

    expect(keepsNativeMenu(row.querySelector(".workdir-name"))).toBe(false);
  });

  it("keeps the webview menu in the surfaces text is selected from", () => {
    shell(`
      <div class="code-editor-host"><div class="cm-content"><span class="cm-line">x</span></div></div>
      <div class="terminal-host"><div class="xterm"><div class="xterm-rows">y</div></div></div>
      <div class="source-read">z</div>
      <div class="markdown-preview">w</div>
      <div class="diff-files">v</div>
      <input />
      <textarea></textarea>
    `);

    for (const selector of [
      ".cm-line",
      ".xterm-rows",
      ".source-read",
      ".markdown-preview",
      ".diff-files",
      "input",
      "textarea",
    ]) {
      expect(keepsNativeMenu(document.querySelector(selector)), selector).toBe(true);
    }
  });

  it("reads a right-click on a child of a granted surface, not only on the surface itself", () => {
    shell(`<div class="markdown-preview"><p><em>deep</em></p></div>`);

    expect(keepsNativeMenu(document.querySelector("em"))).toBe(true);
  });

  it("denies the webview menu outside every grant, including where there is no target", () => {
    expect(keepsNativeMenu(null)).toBe(false);
    expect(keepsNativeMenu(document.createTextNode("x"))).toBe(false);
  });

  it("leaves the menu to a granted surface and takes it from the chrome", () => {
    const row = shell(`<input data-testid="field" /><div class="workdir-item"></div>`);
    installContextMenu(document);
    const field = row.querySelector("input") as HTMLInputElement;
    const workdir = row.querySelector(".workdir-item") as HTMLElement;

    const inField = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    field.dispatchEvent(inField);
    expect(inField.defaultPrevented).toBe(false);

    const onChrome = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    workdir.dispatchEvent(onChrome);
    expect(onChrome.defaultPrevented).toBe(true);
  });

  it("takes the menu even from a surface that stops the event on its way up", () => {
    const row = shell(`<div class="source-read"></div>`);
    // A library handling `contextmenu` itself must not be able to hand the webview menu back.
    row.addEventListener("contextmenu", (event) => event.stopPropagation());
    installContextMenu(document);

    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    row.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
  });
});
