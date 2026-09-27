/**
 * What a right-click opens, and where it is allowed to open anything.
 *
 * The window is a webview, and a webview answers a right-click with a menu of its own: Reload,
 * Back, Forward, Look Up, and Inspect Element in a build with a debugger attached. Reload is the
 * one that costs something. The app keeps unsaved drafts in the code editor and writes into the
 * user's repositories, so a reload drops every open draft on the floor — and it does it from a
 * right-click that landed on a sidebar row and did not look like it had hit anything at all.
 *
 * There is no way to take that menu away from the host. wry carries `with_default_context_menus`,
 * but it is applied in its WebView2 path and nowhere else, and Tauri does not expose it, so on
 * macOS the event is the only lever there is.
 *
 * The native menu is worth keeping where it is a text menu: it is the one place Cut, Copy and
 * Paste arrive already wired to the right keys and already greyed out when there is nothing to act
 * on. So it is denied by default and granted on the surfaces where a right-click has work to do,
 * which is the same rule `style.css` applies to `user-select` and for the same reason — a
 * right-click that lands on a row, a crumb or a pane header has nothing to offer, and offering
 * nothing is the honest answer.
 */

/**
 * Where the webview's own menu is the right answer.
 *
 * Every entry is somewhere text is selected from. The read views are on the list because
 * `style.css` makes them selectable, and a selection a user cannot copy with a pointer is a
 * selection the pointer forgot about.
 *
 * The terminal is named rather than reached, and cannot be left out: xterm's stylesheet puts
 * `user-select: none` on `.xterm` itself, so no grant inherits into one. It is here because a
 * right-click over terminal output has exactly one thing a user wants from it, and the menu is
 * where that is normally reached from.
 */
const GRANTED = [
  ".code-editor-host .cm-content",
  ".terminal-host .xterm",
  ".source-read",
  ".markdown-preview",
  ".diff-files",
  ".diff-viewport",
  "input",
  "textarea",
].join(", ");

/** Whether a right-click that landed on this target keeps the webview's menu. */
export function keepsNativeMenu(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(GRANTED) !== null;
}

/**
 * Denies the webview menu everywhere a right-click is not asking for text.
 *
 * Capture, so that a library handling the event on its way up cannot reach the webview's menu by
 * stopping it before it gets here.
 */
export function installContextMenu(root: Document = document): void {
  root.addEventListener(
    "contextmenu",
    (event) => {
      if (!keepsNativeMenu(event.target)) event.preventDefault();
    },
    true,
  );
}
