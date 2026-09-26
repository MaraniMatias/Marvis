# Marvis — plan de refactor de UI hacia el mockup

**Estado:** plan aprobado en conversación, pendiente de ejecución. **Alcance:** llevar el diseño visual de `docs/marvis-ui/` (mockup) a la aplicación Tauri real. **No** es un plan de alcance de producto: no sustituye a [MARVIS_DELIVERY_PLAN_v0.4.md](MARVIS_DELIVERY_PLAN_v0.4.md) ni a [PLAN_PENDIENTE.md](PLAN_PENDIENTE.md). **Plataforma:** macOS. **Origen:** análisis comparativo del mockup contra la app, y las decisiones tomadas sobre ese análisis.

> **Convención de este documento.** Los puntos marcados **[DECIDIDO]** están cerrados y no se revisan durante la ejecución. Los marcados **[DEFAULT]** son un supuesto quemado por falta de respuesta: se aplican salvo que se digan lo contrario antes de empezar la fase correspondiente. Los marcados **[ABIERTO]** necesitan una decisión y bloquean lo que indiquen.

---

## 1. Contexto

### 1.1 Qué hay en cada lado

|              | Mockup `docs/marvis-ui/`                                                           | App real `src/`                                                                                                                                                                                          |
| ------------ | ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Stack        | Vue 3.5 + Vite 8 + Tailwind 4 + shadcn-vue + reka-ui + `@lucide/vue`               | Vue 3.5 + Vite 7 + Tailwind 4 + reka-ui + xterm 6 + `@git-diff-view/vue` + Shiki                                                                                                                         |
| Componentes  | 4 (`App`, `MarvisTitlebar`, `WorkdirSidebar`, `DetailsSidebar`) + `ui/resizable/*` | 12 (`Sidebar`, `SessionPane`, `TerminalSession`, `DocumentPane`, `FileDiff`, `ChangesPane`, `InspectorPane`, `CommandPalette`, `WorktreeDialog`, `ReviewComposer`, `ReviewNoteList`, `ui/button/Button`) |
| Líneas de UI | ~1.100                                                                             | 3.771                                                                                                                                                                                                    |
| Datos        | `src/mocks/*.ts` estáticos                                                         | IPC real a Rust (`src-tauri/src`)                                                                                                                                                                        |
| Layout       | 3 paneles fijos, sin estado                                                        | `AppLayoutState` + `CheckoutUiState` persistidos por checkout                                                                                                                                            |
| Tests        | ninguno                                                                            | 9 archivos de componente, ~106 tests                                                                                                                                                                     |

### 1.2 Diagnóstico original: qué le faltaba al mockup

El mockup cubre **el cromado de la app, no la app**. Se ven tres superficies reales (titlebar, sidebar de workdirs, sidebar de files/changes) y un terminal falso de ocho `div`. Falta la vista documento/diff, el loop de review notes, el envío al agente, la palette, los diálogos, y todas las máquinas de estado. El detalle por sección está en §3.

---

## 2. Decisiones cerradas

### A — Titlebar y window chrome

- **[DECIDIDO] A.1** Traffic lights **nativos**. Se borran los tres `<span class="dot">` falsos del mockup (`background: #3a3d48`).
- **[DECIDIDO] A.2** El `search-box` del mockup es un `div` decorativo; pasa a ser un `<input>` **real y funcional**.
- **[DECIDIDO] A.3** El input de búsqueda **queda sin resultados por ahora**: focus, `Escape` y estado visual funcionan; la lógica de búsqueda se implementa después. La UI queda lista.
- **[DECIDIDO] A.4** Los crumbs (breadcrumbs) se quedan **como están en el mockup**.
- **[DECIDIDO] A.5** El botón de Settings (engranaje) **se queda**: hay settings previstos en el futuro cercano.
- **[DECIDIDO] A.6** El banner de error local (`role="alert"` flotante en `App.vue`) **no tiene sentido y se elimina**. Los errores van a **toasts**.
- **[DECIDIDO] A.7** El mockup **no tiene** `data-tauri-drag-region` y su `.marvis-titlebar` tiene `padding: 0px 16px` (≈22px de alto). La app usa `h-12` (48px) con `pl-[82px]`. Se adopta `h-12` y se agrega el spacer de arrastre.

Orden final del header (mismo orden que el mockup, con los dots nativos y el spacer):

```
[inset 70-78px para lights nativos] [search input] [data-tauri-drag-region spacer] [crumbs] [settings]
```

- **[DEFAULT] A.8** El `ChevronDownIcon` del último crumb abre un **dropdown con los items hermanos del workdir activo** (los terminals de ese workdir). El chevron está en el mockup y sin acción sería ruido; el dropdown es lo único que lo justifica. Si se prefiere, se saca el chevron y el crumb queda como texto plano.
- **[DEFAULT] A.9** Se agrega **double-click-to-zoom** sobre la zona de arrastre (comportamiento nativo de macOS, hoy ausente porque el header no es arrastrable en esa zona).

### B — Tokens, tipografía y ventana

- **[DECIDIDO] B.1** Tipografía de la terminal: **la que ya tiene la app**, fija y con ligaduras. No se toca el `fontFamily`. Se confirma en `src/components/TerminalSession.vue:34-47`:
  ```ts
  fontFamily: '"FiraCode Nerd Font Mono", monospace',
  fontSize: 16,
  lineHeight: 1.2,
  scrollback: 10000,
  ```
  xterm ya activa ligaduras por default. **Lo que sí cambia es el `theme` de xterm**, que está hardcodeado (`background: #0b0f15`, `foreground: #d9e2f0`, `cursor: #7dd3fc`, `selectionBackground: #334155`) y no usa tokens.
- **[DECIDIDO] B.2** Tokens: se usa **`marvis`** (los `--marvis-*` del mockup), no los `--surface-*` de la app.
- **[DECIDIDO] B.3** La app es **siempre monospace**. `body { font-family: var(--marvis-font) }`.
- **[DECIDIDO] B.4** **Se saca la transparencia de Tauri.** Fuera `transparent: true`, `windowEffects` y `macOSPrivateApi` de `src-tauri/tauri.conf.json`.
- **[DECIDIDO] B.5** **Dark only.** No hay conmutador de tema; la clase `.dark` del mockup (que nunca se aplica) se elimina.

**Consecuencia aceptada:** se pierde la vibrancia nativa de macOS (el material `sidebar` de `windowEffects`). Today `src/style.css` tiene seis `backdrop-filter: blur(...) saturate(...)` y tokens con alfa (`rgb(25 28 34 / 90%)`, `rgb(12 15 20 / 82%)`, `rgb(13 17 23 / 96%)`, `rgb(15 19 26 / 98%)`). Todos se van.

- **[DEFAULT] B.6** `--marvis-font` es `ui-monospace, "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace`, pero **el mockup no importa JetBrains Mono** (su `style.css` solo trae Geist desde Google Fonts). Si no está instalada en el sistema, la UI cae a SF Mono mientras la terminal queda en Fira Code: dos monoes distintas en la misma pantalla. **Default:** usar el fallback del sistema tal cual, sin bundlear la fuente. **[ABIERTO]** decidir si se bundlea un woff2 de JetBrains Mono en `src/assets/fonts/`.

### C — Sidebar de workdirs

- **[DECIDIDO] C.1** **Olvidarse de `isMissing`.** Se elimina el estado de checkout faltante: badge `Missing`, botón `Locate` (folder picker), botón de cerrar con `window.confirm("Close “X” in Marvis? No files will be deleted.")`, y el copy asociado.
- **[DECIDIDO] C.2** Se elimina el **badge "Concurrent"** y el `activityByCheckout` que lo alimentaba (agentes concurrentes + "Recent file writes").
- **[DECIDIDO] C.3** **Fuera el collapse de repos.** `collapsedRepoIds` y el chevron ▸/▾ se van. El `.group-header` del mockup es texto plano, consistente con esto.
- **[DECIDIDO] C.4** **Fuera la barra de herramientas del sidebar** (input "Filter repositories…" y botón `+` de abrir directorio en el header).
- **[DECIDIDO] C.5** En su lugar, al **final del sidebar** (después del último grupo) va una fila **"Open directory"**, con la misma forma visual que la fila `New terminal` del mockup: `PlusIcon` + texto en `--marvis-text-dim`, 12px, clase `.new-item`.
- **[DECIDIDO] C.6** Los botones de worktree **existen** en el mockup (hover) y hay que **conectarlos**: `TrashIcon` → `WorktreeDialog` en modo `remove`, `PlusIcon` → `WorktreeDialog` en modo `create`.
- **[DECIDIDO] C.7** El `XIcon` de cierre de sesión **ya está** en hover en los items. Hoy hace `items.splice()` sobre los props (anti-patrón que revienta en Vue real); pasa a emitir el evento contra el estado real.
- **[DECIDIDO] C.8** Las carpetas planas (no-git) se ven **como en el mockup**: sin diff stats, sin `TrashIcon`/`PlusIcon`, sin tab Changes.
- **[DECIDIDO] C.9** El label de tipo de repo ("Git" / "Plain") se va: no está en el mockup.
- **[DEFAULT] C.10** **Sin punto verde/rojo por sesión.** El icono `SquareTerminal` va en `--marvis-accent` cuando la sesión/item está activo y en `--marvis-text-faint` cuando exited. Es la información mínima sin inventar un elemento que el mockup no tiene. (La app hoy pone un punto verde/rojo en `Sidebar.vue:221-228`.)

### D — Selección y vista activa

- **[DECIDIDO] D.1** En el panel main **solo puede haber una cosa activa (visible) a la vez**. Por eso la sidebar tiene la estructura workdir + item: la fila del workdir y la fila del item son los dos niveles de la misma selección.
- **[DEFAULT] D.2** Como el mockup **no tiene handlers de selección**, se define: click en la fila de un **workdir** → activa ese workdir y muestra su item activo (o el último); click en un **item** → muestra ese item.
- **[DEFAULT] D.3** El crumb final muestra el **item activo** del workdir activo (el mockup lo llama `target`, con el valor de ejemplo `"terminal 1"`).

### E — Details sidebar (Files / Changes)

- **[DECIDIDO] E.1** **Sin búsqueda de archivos por ahora.** No hay input de search en este panel. (La app hoy tiene fuzzy search con cap de 200 matches, aviso de truncado a 50.000 archivos y estados `idle/loading/ready/error` en `InspectorPane.vue`; eso se desactiva, no se borra todavía.)
- **[DECIDIDO] E.2** Lazy loading y virtualización **se mantienen** (es comportamiento, no diseño): `listCheckoutFiles` por directorio, estados `loading/empty/error/truncated`, ventana de 80 filas con 12 de overscan.
- **[DECIDIDO] E.3** **Lo que está abierto se hace visible.** Hoy las filas del árbol no tienen estado activo: click en un archivo o en un change **abre en el panel main**, y mientras carga **se oculta el contenido previo** (estado de carga en `.pane-state`).
- **[DECIDIDO] E.4** Errores: **generales a toast**; si el error es de un repo/workdir concreto, **el texto va en rojo en la fila** de ese workdir, en `.workdir-meta` (donde están los `+N/-N`), porque `.workdir-title` no hace wrap.
- **[DECIDIDO] E.5** Las decoraciones git del árbol son **como en el mockup**: status `M/A/D/U` en la fila con los colores del mockup, más `+N/-N`. **No** va la burbuja `•` en directorios ni el caso `??` en verde que tiene la app.
- **[DECIDIDO] E.6** El tab **Changes se agrupa por directorio padre**, como en el mockup (`changeGroups` con `group.dir` como header). Esto **reemplaza** la lista plana de paths completos con chip de status que tiene la app.

### F — Panel main

- **[DECIDIDO] F.1** **Una sola vista a la vez**, y son tres: **terminal**, **git diff**, **file preview**. La app hoy tiene más cosas y la UI/UX es confusa; el mockup es más simple y se adopta.
- **[DECIDIDO] F.2** En el tab **Changes** se agrega una fila con icono **`square-arrow-out-up-right`** que **carga el git diff de todos los archivos** en el panel main, con capacidad de comentar.
- **[DECIDIDO] F.3** Cada **archivo seleccionado en Changes** carga **el git diff de ese archivo** en main, con la funcionalidad de **comentar** y **enviar a opencode**.
- **[DECIDIDO] F.4** Si el archivo seleccionado es un `.md`, el panel main muestra el **render del Markdown**.
- **[DECIDIDO] F.5** La vista de diff **no tiene ni un píxel en el mockup**. Se **recolorea el diff actual** (`@git-diff-view/vue`) con los tokens marvis y **se mantiene la UI de notas actual**: composer inline por línea (`ReviewComposer`) + lista de notas al pie (`ReviewNoteList`). Solo se rediseña el header del diff.

### G — Overlays

- **[DECIDIDO] G.1** **`CommandPalette` no es útil: se borra.** Con ella caen `src/components/CommandPalette.vue` y `src/domain/command-palette.ts`.
- **[DECIDIDO] G.2** **`WorktreeDialog` se queda**, para crear, abierto desde el `PlusIcon` del workdir (y en modo `remove` desde el `TrashIcon`).
- **[DECIDIDO] G.3** **Toasts: sí**, siguiendo el estilo. El mockup **no tiene ninguno**, así que se diseña desde los tokens.
- **[DECIDIDO] G.4** Markdown preview en main (mismo punto que F.4).

### H — Layout

- **[DECIDIDO] H.1** Sidebar: **default 240, min 240, max 500**.
- **[DECIDIDO] H.2** Inspector: **default 280, min 200, max 480**.
- **[DECIDIDO] H.3** **Se aplica el drawer overlay** en ventana angosta.
- **[DECIDIDO] H.4** **Nada** de: collapse/expand de paneles, focus mode con `focusSnapshot`, `reset-layout`, atajos `⌘0` / `⌘⌥0` / `⌘Q`.
- **[DECIDIDO] H.5** El drawer **se auto-abre al seleccionar** un archivo/change y **se cierra al abrir main**.
- **[DEFAULT] H.6** Se **sigue guardando el ancho** de los panels al arrastrar. Es lo único que sobrevive de `AppLayoutState`.

**Contraste con la app actual** (`src/domain/ui-state.ts:32-35,74-75,167-170`):

|           | Mockup pedido                | App hoy                                                                                               |
| --------- | ---------------------------- | ----------------------------------------------------------------------------------------------------- |
| Sidebar   | default 240, 240–500         | default 260, 220–380                                                                                  |
| Inspector | default 280, 200–480         | default 320, 260–560                                                                                  |
| Main      | —                            | `min-size 420`, ventana `minWidth 900`                                                                |
| Handle    | `with-handle` (grip visible) | hairline 5px, dbl-click resetea                                                                       |
| Angosto   | drawer auto                  | `needsInspectorDrawer`: `viewportWidth < sidebar + inspector + 430` → panel colapsado + pane flotando |

El clamp de `resizeLayoutPanel` en Rust-free (frontend) queda en 240–500 / 200–480, y `needsInspectorDrawer` se recalibra al nuevo default.

### I — Implementación

- **[DECIDIDO] I.1** **Todos los iconos de `@lucide/vue`.** Se agrega la dependencia. Los SVG inline a mano que tiene hoy `Sidebar.vue` se reemplazan.
- **[DECIDIDO] I.2** Componentes: **CSS local + shadcn-vue**. Se trae el `components.json` y el `ui/resizable/*` del mockup.
- **[DECIDIDO] I.3** `user-select`: **como en el mockup** — `none` en el shell, `text` **solo dentro del panel main** (terminal, file preview, git diff).
- **[DEFAULT] I.4** **Se conserva el copy en inglés de la app.** El mockup no tiene ni un string de empty / loading / error; reescribirlos es trabajo sin información. Solo se inventa el copy nuevo que no exista (toasts, header del diff).

---

## 3. Hallazgo bloqueante: Neovim queda huérfano

**Hoy `nvim` solo se alcanza desde la palette.** `runPaletteCommand('open-neovim')` (`src/App.vue:805-811`) es el **único** caller de `requestNvim()`, y `DocumentPane` solo tiene un botón `↗ Zed` (`src/components/DocumentPane.vue:386-394`). Borrar `CommandPalette.vue` + `domain/command-palette.ts` deja sin entrada a:

- `requestNvim()` → `SessionPane.launchNeovim()` → `createTerminalSession("nvim", target)`
- `getEditorAvailability().neovim` (`src/lib/ipc.ts:20`), que además es lo que habilita el botón
- El path Rust completo: `neovim` aparece en `src-tauri/src/services/{editor,terminal}.rs`, `commands/{editor,terminal}.rs`, `domain/workspace.rs`, `persistence/mod.rs` (12 ocurrencias). El tipo de sesión nvim es un concepto de primera clase en Rust; sin entrada de UI queda huérfano.

- **[DEFAULT] G.5** Se agrega un botón **`↗ Neovim`** en el toolbar de la vista diff/document, al lado de `↗ Zed`, con el mismo wiring (`requestNvim(checkoutId, file, line, column)`).

### 3.1 Destino de los 13 comandos de la palette

| Comando               | Destino tras el refactor                                |
| --------------------- | ------------------------------------------------------- |
| `open-directory`      | Fila "Open directory" al pie del sidebar (C.5)          |
| `new-terminal`        | Fila `New terminal` por workdir (ya en el mockup)       |
| `new-worktree`        | `PlusIcon` del workdir → `WorktreeDialog` create (C.6)  |
| `open-file`           | Click en una fila del árbol de Files (E.3)              |
| `open-changes`        | Fila `square-arrow-out-up-right` del tab Changes (F.2)  |
| `open-preview`        | Botón `View`/`Code` del toolbar de document (ya existe) |
| `open-zed`            | Botón `↗ Zed` (ya existe)                               |
| `open-neovim`         | **Botón nuevo `↗ Neovim`** (G.5)                        |
| `toggle-sidebar`      | **Sin replacement** (H.4)                               |
| `toggle-inspector`    | **Sin replacement** (H.4)                               |
| `toggle-focus`        | **Sin replacement** (H.4)                               |
| `toggle-transparency` | **Sin replacement** (B.4 mata la transparencia)         |
| `reset-layout`        | **Sin replacement** (H.4)                               |

---

## 4. Defaults quemados (tabla resumen)

| #       | Default                                                                                                 | Si hay que cambiarlo, bloquea |
| ------- | ------------------------------------------------------------------------------------------------------- | ----------------------------- |
| A.8     | ChevronDown del crumb → dropdown de items hermanos                                                      | Fase 1                        |
| A.9     | Double-click-to-zoom en la zona de arrastre                                                             | Fase 1                        |
| B.6     | Sin bundle de JetBrains Mono; fallback del sistema                                                      | Fase 0                        |
| C.10    | Sin punto de estado; color del icono según active/exited                                                | Fase 2                        |
| D.2     | Click workdir → muestra su item activo; click item → muestra ese item                                   | Fase 2                        |
| D.3     | Ultimo crumb = item activo                                                                              | Fase 1                        |
| E.3-bis | El badge del tab Changes cuenta **archivos cambiados**, no notas (como el mockup: `changedRows.length`) | Fase 3                        |
| H.6     | Se sigue guardando el ancho de los panels al arrastrar                                                  | Fase 1                        |
| I.4     | Se conserva el copy en inglés de la app                                                                 | Fase 5                        |
| G.5     | Botón `↗ Neovim` junto a `↗ Zed`                                                                        | Fase 6                        |
| G.6     | Copy del botón de diff completo: `All changes`                                                          | Fase 6                        |
| E.7     | Se cae el tracking "Viewed" (badge verde en Changes)                                                    | Fase 3 + 7                    |

---

## 5. Plan por fases

Cada fase termina en algo que corre y se puede ver. Los tests se reescriben **dentro** de la fase que toca el componente, no al final.

### Fase 0 — Base: tokens y ventana

**Objetivo:** la app se ve con la paleta marvis, opaca, toda en monospace. El layout interior sigue siendo el viejo.

| Archivo                     | Acción                                                                                                                                                                                                                                                                  |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `package.json`              | `+ @lucide/vue`, `+ shadcn-vue`, `+ class-variance-authority`, `+ clsx`, `+ tailwind-merge`, `+ tw-animate-css` (los que falten de la lista del mockup)                                                                                                                 |
| `components.json`           | Nuevo, copiado del mockup                                                                                                                                                                                                                                               |
| `src/marvis.css`            | Nuevo: los `--marvis-*`, `.icon-sm/xs/xxs`, `.group-header`, `.diff-add/del`, las reglas de `:focus-visible` y el repintado de `[data-slot="resizable-handle"]`                                                                                                         |
| `src/style.css`             | `body { font-family: var(--marvis-font) }`; **fuera** los 6 tokens `--surface-*`/`--border-*`/`--selection`/`--accent-soft` con alfa, los 6 `backdrop-filter`, las 9 reglas `.reduce-transparency`; los selectores de superficie pasan a pintar con `--marvis-bg-0/1/2` |
| `src-tauri/tauri.conf.json` | **Fuera** `app.transparent`, `app.windowEffects`, `app.macOSPrivateApi`                                                                                                                                                                                                 |
| `src/main.ts`               | Importar `marvis.css` antes de `style.css`                                                                                                                                                                                                                              |

**Tokens a adoptar textualmente** (de `docs/marvis-ui/src/marvis.css`):

```css
--marvis-bg-0: #17191f; /* panel central, fondo del shell   */
--marvis-bg-1: #1c1f27; /* titlebar y ambas sidebars        */
--marvis-bg-2: #22252e; /* hover de fila, search box        */
--marvis-border: #2c2f3a; /* hairlines, splitter, active child*/
--marvis-text: #d6d9e0;
--marvis-text-secondary: #979a9f;
--marvis-text-dim: #8b8fa3;
--marvis-text-faint: #5c6072;
--marvis-accent: #7c9eff; /* selección, item activo, cursor   */
--marvis-green: #7fd88f; /* +N, status A                     */
--marvis-red: #e08585; /* -N, status D, action peligroso   */
--marvis-radius: 6px;
--marvis-font: ui-monospace, "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace;
```

**Mapa de superficie** (reemplaza los `--surface-*`):

| Superficie               | Token                    | Reemplaza                              |
| ------------------------ | ------------------------ | -------------------------------------- |
| Shell / panel main       | `--marvis-bg-0`          | `--surface-center`                     |
| Titlebar, sidebars       | `--marvis-bg-1`          | `--surface-panel`                      |
| Hover de fila, input bg  | `--marvis-bg-2`          | `rgb(0 0 0 / 12%)`                     |
| Document, diff           | `--marvis-bg-0`          | `--surface-document`, `--surface-diff` |
| Terminal                 | `--marvis-bg-0`          | `.terminal-surface { #0b0f15 }`        |
| Hairline                 | `--marvis-border`        | `--border-subtle`, `--border-hairline` |
| Selección de texto       | `rgb(124 158 255 / 30%)` | `--selection`                          |
| Superficie de tab activa | `--marvis-border`        | `--accent-soft`                        |

**Verificación:** `pnpm typecheck`, `pnpm lint`, `pnpm build`, abrir la app y confirmar que no quedan `backdrop-filter` ni `--surface-*`.

---

### Fase 1 — Shell: titlebar y splitter

**Objetivo:** corredor completo con crumbs, search input, lights nativos y anchos 240/280. La sidebar y el inspector siguen siendo los viejos, re-coloreados.

| Archivo                                                                  | Acción                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `src/App.vue`                                                            | Header nuevo: `h-12`, inset izquierdo para los lights nativos, `<input>` de búsqueda, spacer `data-tauri-drag-region`, crumbs, engranaje. Fuera el banner `v-if="error"`. Splitter con los nuevos clamps.                                                                                                                |
| `src/domain/ui-state.ts`                                                 | `AppLayoutState` → `{ sidebarWidth, inspectorWidth }`. Defaults 240 / 280, clamps 240–500 / 200–480. Fuera `sidebarVisible`, `inspectorVisible`, `collapsedRepoIds`, `focusSnapshot`, `reduceTransparency`, `toggleFocusLayout`, `toggleLayoutVisibility`, `needsInspectorDrawer` (se recalibra con los nuevos default). |
| `src/lib/ipc.ts`                                                         | `loadAppLayout` / `saveAppLayout` con el shape reducido                                                                                                                                                                                                                                                                  |
| `src-tauri/src/commands/ui_state.rs`, `src-tauri/src/persistence/mod.rs` | Aceptar el shape reducido; `normalize` de vuelta a defaults si viene el shape viejo                                                                                                                                                                                                                                      |
| `src/components/ui/button/Button.vue`                                    | Re-tokenizar (cva + `--marvis-*`)                                                                                                                                                                                                                                                                                        |

**Borrar de `src/App.vue`:** `toggleFocusMode`, `toggleTransparency`, `toggleLayoutVisibility`, `resetLayout`, `resetPanelWidth`, `onWindowKeydown` (los atajos `⌘0`/`⌘⌥0`/`⌘Q`), `onPanelCollapse`/`onPanelExpand`, `withSplitterSynchronization`, `synchronizingSplitters`, `userSplitterIntent`, `userSplitterKeyDown`, `paletteRequestToken`, `paletteCommands`, `runPaletteCommand`, `requestInspector`.

**Crumbs:** se derivan del estado, no son props. Estructura del mockup:

```
{repo} / {icon GitFork} {branch} / {item activo} {ChevronDown}
```

`repo` y `branch` son spans (el mockup no los hace clickeables, y H.4 elimina el toggle del sidebar). El `ChevronDown` aplica el default A.8.

**Search input:** `<input>` real con `aria-label="Search files and commands"` (el `aria-label` sobrevive aunque A.3 deje la búsqueda sin implementar), placeholder `Search...` como el mockup, focus visible con `--marvis-accent`, `Escape` que lo desenfoca. **Sin** el `kbd ⌘K` del mockup (la palette que lo atendía no existe).

**Verificación:** `src/App.test.ts` (36 tests) reescrito; la ventana se puede arrastrar por el spacer; los lights nativos no chocan con el input.

---

### Fase 2 — Sidebar de workdirs

**Objetivo:** la sidebar del mockup, con worktree create/remove y "Open directory" al pie.

| Archivo                             | Acción                                                                   |
| ----------------------------------- | ------------------------------------------------------------------------ |
| `src/components/Sidebar.vue`        | Reescritura siguiendo `docs/marvis-ui/src/components/WorkdirSidebar.vue` |
| `src/components/WorktreeDialog.vue` | Re-tokenizar; conservar modos `create` / `remove`                        |
| `src/domain/worktree.ts`            | Sin cambios de contrato                                                  |

**Se porta casi literal del mockup:**

- `.workdir-item` / `.workdir-row` / `.workdir-items` / `.workdir-child` con el padding escalonado (5px 8px 5px 10px, child 24px)
- Acciones **overlay** en hover: `position: absolute; right: 0; opacity: 0` + `padding-right: 46px` en hover para que el título elipsize en vez de meterse debajo de los iconos
- `.workdir-item.active` → `--marvis-bg-2` + `box-shadow: inset 2px 0 0 var(--marvis-accent)`
- `.workdir-child.active` → `--marvis-border` + icono en acento
- `.workdir-item.has-active` → `--marvis-border` + `inset 2px 0 0 var(--marvis-text-secondary)` (workdir padre de un item activo)
- `.workdir-meta` (los `+N/-N`) se oculta en hover para no chocar con las acciones
- `.workdir-branch` con `meta-dot` + `GitBranchIcon` 10px
- `.new-item` (12px, `--marvis-text-dim`) para `New terminal` y la nueva fila `Open directory`
- `.workdir-action-danger:hover` → `--marvis-red`

**Iconos** (de `@lucide/vue`, vía `IconKind`): `FolderGit2` (repo raíz), `GitFork` (worktree), `Folder` (carpeta plain), `SquareTerminal` (item), `Plus`, `Trash`, `X`, `GitBranch`.

**Handlers que hay que conectar** (hoy el mockup no tiene ninguno):

| Elemento                        | Acción                                                      |
| ------------------------------- | ----------------------------------------------------------- |
| Fila workdir                    | `selectCheckout(checkoutId)` + mostrar su item activo (D.2) |
| Fila item                       | `selectSession(sessionId)` + `mainViews → terminal`         |
| `TrashIcon` (solo worktree)     | `openWorktreeDialog('remove', checkoutId)`                  |
| `PlusIcon` (solo repo raíz git) | `openWorktreeDialog('create', checkoutId)`                  |
| Fila `New terminal`             | `requestShell(checkoutId)`                                  |
| `XIcon` de item                 | `closeTerminalSession(sessionId)` con confirmación          |
| Fila `Open directory` (pie)     | `chooseFolder()`                                            |

**Se elimina:** input de filtro, `isMissing` (badge + `Locate` + close + `window.confirm`), `activityByCheckout` y el badge `Concurrent`, `collapsedRepoIds` y el toggle del repo, el label `Git`/`Plain`, el punto verde/rojo de sesión (default C.10).

**Estructura de datos:** el `Workdir` del mockup (`title, branch, additions, deletions, active, worktree, gitdir, kind, items`) se mapea al `Checkout` real (`id, path, branch, isPrimary, isMissing, sessions[]`) + `Repo.kind` + `gitSnapshot` para los `+N/-N`. `isMissing` se ignora, no se elimina del tipo hasta que el backend lo revise.

**Verificación:** `src/components/Sidebar.test.ts` (3 tests) reescrito; hover sobre worktree y sobre item; el `padding-right` no rompe el ellipsis con títulos largos.

---

### Fase 3 — Details: Files y Changes

**Objetivo:** árbol con selección visible y Changes agrupado por directorio, con la fila de diff completo.

| Archivo                              | Acción                                                                                                                                            |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/components/InspectorPane.vue`   | Reescritura siguiendo `docs/marvis-ui/src/components/DetailsSidebar.vue`; conserva el lazy-load, la virtualización, los estados y el `savedState` |
| `src/components/ChangesPane.vue`     | **Se disuelve**: su lista pasa al tab Changes del inspector, su lógica de envío al diff (fase 6)                                                  |
| `src/components/ChangesPane.test.ts` | Se borra (8 tests)                                                                                                                                |

**Files:**

- `TREE_ROW_HEIGHT` pasa de **28 → 22** (`.file-row` del mockup es `padding: 3px 6px` + font 12px). Ajustar `TREE_WINDOW_SIZE` (80) y `TREE_OVERSCAN` (12) proporcionalmente. **La constante y el CSS tienen que quedar derivando del mismo valor.**
- Fila: `ChevronRight` (10px, rota 90° al expandir) o spacer, `Folder`/`File` (12px), nombre con ellipsis, status `M/A/D/U` con los colores del mockup, `+N`/`-N`.
- Profundidad: `paddingLeft: 6 + depth * 14`.
- **Selección visible** (E.3): `aria-current` + superficie `--marvis-bg-2`, como el `.details-tab[aria-selected]` del mockup.
- Estados por directorio: `loading` / `empty` / `error` / `truncated` en `.pane-state`, con `role="status"` / `role="alert"` según corresponda.
- Se **desactiva** la búsqueda: `searchQuery` queda sin input. Los tests de rutas de búsqueda implícitos en `InspectorPane.test.ts` se reescriben.
- Tabs con roving tabindex: `ArrowLeft`/`ArrowRight`/`Home`/`End`, `aria-selected`, `aria-controls`, `tabindex="-1"` en el inactivo, y **`tabindex="0"` en el `role="tabpanel"`** (el mockup se lo salta).

**Changes:**

- `changeGroups`: `Map<parentDir, rows>` con `group.dir` como `.group-header` y las filas un nivel adentro (`padding-left: 20px`), exactamente como el mockup.
- Status `M` → `--marvis-text`, `A` → `--marvis-green`, `D` → `--marvis-red`, `U` → `--marvis-text-faint`.
- Badge en el tab: **cantidad de archivos cambiados** (`changedRows.length`, default E.3-bis).
- **Fila `square-arrow-out-up-right`** arriba de la lista → `mainViews → diff` con `path: null` (diff de todos los archivos). Visual: `PlusIcon` + texto `All changes` en `.new-item` (default G.6).
- Se elimina: input de filtro de changed files, summary block (branch, `vs defaultBranch`, `N changed`, `N commits ahead`, `X/N viewed`), flag `Viewed`, y los `viewedPaths` de `src/presentation/active-git-snapshot.ts:12,43,73,76-77,110,142,150` (default E.7).

**Verificación:** `src/components/InspectorPane.test.ts` (6 tests) reescrito; la virtualización no desalinea con el nuevo row height; seleccionar un archivo emite el evento que la fase 4 consume.

---

### Fase 4 — Main unificado

**Objetivo:** una sola vista activa — terminal, document o diff.

| Archivo                              | Acción                                                                                                                                                               |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/domain/main-document.ts`        | `MainDocumentMode` → `MainView = { kind: "terminal"; sessionId } \| { kind: "document"; path; mode } \| { kind: "diff"; path \| null }`; `resolveMainView` reescrito |
| `src/App.vue`                        | `mainViews: Record<checkoutId, "terminal" \| "document">` → `Record<checkoutId, MainView>`; el último crumb pasa a derivarse de ahí                                  |
| `src/components/SessionPane.vue`     | Re-tokenizar. Sin rediseño.                                                                                                                                          |
| `src/components/TerminalSession.vue` | Solo el `theme` de xterm, a tokens                                                                                                                                   |
| `src/components/DocumentPane.vue`    | Se parte: preview de archivo + render de `.md`                                                                                                                       |
| `src/components/FileDiff.vue`        | Re-tokenizar + modo "todos los archivos" vía `use-large-diff.ts`                                                                                                     |

**Vista terminal:** se conserva tal cual. `TerminalSession` mantiene `fontFamily`/`fontSize`/`lineHeight`/`scrollback`; solo el `theme` pasa a tokens:

```ts
theme: {
  background: "#17191f",        // --marvis-bg-0
  foreground: "#d6d9e0",        // --marvis-text
  cursor: "#7c9eff",            // --marvis-accent
  selectionBackground: "#22252e",// --marvis-bg-2
}
```

**Vista document:** toolbar con el path + `Wrap` + `View`/`Code` + `↗ Zed` + `↗ Neovim` (default G.5). El `.md` se renderiza con el markdown preview existente (`src/lib/markdown-preview.ts`, sanitizado con DOMPurify, `max-width: 78ch` re-tokenizado). Preview de código con Shiki como hoy, con el sticky line-number gutter re-tokenizado.

**Vista diff:** `FileDiff` con dos modos — un archivo (`path`) o todos (`path: null`). `use-large-diff.ts` ya existe y cubre el caso grande. Estados: `loading`, `error` (→ toast, fase 5), `isBinary`, `symlinkTarget`, `tooLarge`, `showNoTextHunks`.

**Loading (E.3):** al seleccionar, el main **oculta el contenido previo** y muestra el estado de carga en `.pane-state` (`Loading file…` / `Loading diff…` / `Loading changes…`).

**`v-show` vs `v-if`:** se conserva el patrón actual (secciones `absolute inset-0` con `v-show`) para que xterm no se desmonte al cambiar de vista. El prop `visible` de `SessionPane` sigue siendo lo que evita el problema de medición.

**Verificación:** `src/components/SessionPane.test.ts` (14), `src/components/DocumentPane.test.ts` (23), `src/components/TerminalSession.test.ts` (10) reescritos; cambiar entre las tres vistas no rompe el foco del terminal ni el tamaño de xterm.

---

### Fase 5 — Toasts y errores

**Objetivo:** fuera el banner; errores consistente.

| Archivo                                     | Acción                                                                  |
| ------------------------------------------- | ----------------------------------------------------------------------- |
| `src/components/ToastStack.vue`             | Nuevo                                                                   |
| `src/presentation/toasts.ts`                | Nuevo: store mínimo (push / dismiss / auto-expire)                      |
| `src/App.vue`                               | Montar `ToastStack`; `reportWarning` y `showWindowError` pasan a toasts |
| `src/components/InspectorPane.vue`          | `rootError` de.path → toast                                             |
| `src/components/ChangesPane.vue` (disuelto) | —                                                                       |
| `src/components/WorktreeDialog.vue`         | `@warning` → toast                                                      |
| `src/components/Sidebar.vue`                | Error de repo/workdir → texto rojo en `.workdir-meta` (E.4)             |

**Diseño del toast** (no existe en el mockup, se hace desde los tokens):

```
┌──────────────────────────────────────────┐
│ ●  <mensaje>                          ✕  │
└──────────────────────────────────────────┘
  background: var(--marvis-bg-2)
  border:     1px solid var(--marvis-border)
  radius:     var(--marvis-radius)   (6px)
  texto:      var(--marvis-text)     13px, --marvis-font
  acento:     --marvis-red  (error)  ·  --marvis-green (éxito)
  ancho:      min(420px, calc(100vw - 2rem))
  posición:   abajo centro, sobre el panel main
  lifetimes:  error 6s (sticky si hay hover)  ·  info 3s
```

Cada toast es `role="status"`; los de error, `role="alert"`. Botón `✕` de dismiss. Se apilan hacia arriba con 6px de gap. `Esc` descarta el más reciente.

**Se elimina:** el `div v-if="error" role="alert"` de `src/App.vue:1080-1086`, y los `role="alert"` inline de los paneles que pasan a toast (los `role="status"` de carga/vacío **se quedan** en `.pane-state`, no son errores).

**Verificación:** test nuevo `src/components/ToastStack.test.ts`; `pnpm test:frontend` verde.

---

### Fase 6 — Notas y envío a opencode

**Objetivo:** el diff completo — commenters y envío.

| Archivo                                                         | Acción                                           |
| --------------------------------------------------------------- | ------------------------------------------------ |
| `src/components/FileDiff.vue`                                   | Header nuevo + notes re-tokenizados              |
| `src/components/ReviewComposer.vue`                             | Re-tokenizar                                     |
| `src/components/ReviewNoteList.vue`                             | Re-tokenizar                                     |
| `src/components/MainPane.vue` (nuevo, o el que resuelva fase 4) | Botón `Send to opencode` + choice de busy        |
| `src/App.vue`                                                   | `requestNvim` desde el botón nuevo (default G.5) |

**Header del diff** (lo único que se diseña de cero):

```
┌────────────────────────────────────────────────────────────┐
│ src/components/App.vue                          [Wrap]     │
│ bug/13104984920-timeline-element-boundaries                 │
│                                          [↗ Zed] [↗ Neovim]│
│                                          [Send to opencode]│
├────────────────────────────────────────────────────────────┤
│  diff body (@git-diff-view/vue re-coloreado)               │
│                                                            │
├────────────────────────────────────────────────────────────┤
│  notes list / composer inline            [Send now|Queue] │
└────────────────────────────────────────────────────────────┘
```

- Título: el path, o `All changes` + la rama cuando `path === null`.
- Body: `@git-diff-view/vue` con `--marvis-bg-0` de fondo, `+` en `--marvis-green`, `-` en `--marvis-red`, hunk headers con `--marvis-border`.
- Notas: **se mantiene la interfaz actual** (F.5). `ReviewComposer` inline por línea/rango, con "Click another line to widen the range" y `Esc`; `ReviewNoteList` al pie con los chips `sent` / `resolved` / `outdated` y el gating `canResolve` (el botón `Resolved` solo aparece si el diff puede respaldarlo).
- Botón `Send to opencode`: con **más de un agente** aparece un `<select>` de destino al lado (como hoy). Cuando el agente está ocupado, el choice `Send now / Queue / Not now` aparece debajo del botón, con el texto actual: _"“X” is mid-task. Sending now lands inside its current turn; queueing waits for it to finish. This OpenCode version cannot cancel a turn."_
- Se conserva la lógica de `src/presentation/review-notes.ts`: `dispatchRound`, `flushQueuedRounds`, `markSent`, `reconcileRounds`, `ackFinishedTurn`, y el auto-create de sesión de `resolveAgentTarget()`.
- Contadores: notas + borradores al lado del botón; "N rounds not finished" debajo; checkbox de outdated notes.

**Verificación:** test nuevo de `FileDiff` con el header; el envío con y sin agente busy; el gating de `Resolved`.

---

### Fase 7 — Limpieza

| Archivo                                   | Acción                                                                                                                |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `src/components/CommandPalette.vue`       | **Borrar** (126 líneas)                                                                                               |
| `src/components/CommandPalette.test.ts`   | **Borrar** (2 tests)                                                                                                  |
| `src/domain/command-palette.ts`           | **Borrar** (13 `PaletteCommandId`)                                                                                    |
| `src/domain/command-palette.test.ts`      | **Borrar**                                                                                                            |
| `src/App.vue`                             | `paletteCommands`, `runPaletteCommand`, `paletteRequestToken`, `requestInspector`, `editorAvailability.neovim` wiring |
| `src/presentation/active-git-snapshot.ts` | `viewedPaths` y `markViewed` (default E.7)                                                                            |
| `src-tauri/src/services/workspace.rs`     | 4 ocurrencias de `viewed`                                                                                             |
| `src-tauri/src/services/git.rs`           | 10 ocurrencias de `viewed`                                                                                            |
| `src-tauri/src/commands/git.rs`           | 5 ocurrencias de `viewed`                                                                                             |
| `src-tauri/src/main.rs`                   | 2 ocurrencias de `viewed`                                                                                             |
| `src-tauri/src/persistence/mod.rs`        | 13 ocurrencias de `viewed`, 2 de `neovim`                                                                             |

**Aviso sobre `viewed`:** son **34 ocurrencias en Rust** (persistencia incluida). Default E.7 lo saca del frontend, pero la capa de persistencia queda huérfana. Hay que decidir en esta fase si se limpia también o se deja el dato persistido sin leer (dejarlo es más barato y no rompe nada, pero es deuda).

**Verificación:** `pnpm fmt:check`, `pnpm lint`, `pnpm lint:rust`, `pnpm typecheck`, `pnpm test:frontend`, `pnpm test:rust`, `pnpm test:security`, `pnpm build:app`.

---

## 6. Plan de tests

| Archivo                                  | Tests | Fase    | Acción                                          |
| ---------------------------------------- | ----- | ------- | ----------------------------------------------- |
| `src/App.test.ts`                        | 36    | 1, 4, 7 | Reescribir (shell, crumbs, splitter, main view) |
| `src/components/DocumentPane.test.ts`    | 23    | 4       | Reescribir (preview + markdown)                 |
| `src/components/SessionPane.test.ts`     | 14    | 4       | Re-tokenizar, misma lógica                      |
| `src/components/TerminalSession.test.ts` | 10    | 4       | Solo el `theme` de xterm                        |
| `src/components/ChangesPane.test.ts`     | 8     | 3       | **Borrar**                                      |
| `src/components/InspectorPane.test.ts`   | 6     | 3       | Reescribir                                      |
| `src/components/WorktreeDialog.test.ts`  | 4     | 2       | Re-tokenizar                                    |
| `src/components/Sidebar.test.ts`         | 3     | 2       | Reescribir                                      |
| `src/components/CommandPalette.test.ts`  | 2     | 7       | **Borrar**                                      |
| `src/components/ToastStack.test.ts`      | —     | 5       | **Nuevo**                                       |
| `src/domain/command-palette.test.ts`     | —     | 7       | **Borrar**                                      |
| `src/domain/ui-state.test.ts`            | —     | 1       | Reescribir (shape reducido)                     |
| `src/domain/main-document.test.ts`       | —     | 4       | Reescribir (tres `MainView`)                    |
| `src/lib/ipc.test.ts`                    | —     | 1       | Ajustar (`loadAppLayout`)                       |

Además, los tests de `presentation/` que arman `gitSnapshot` con `viewedPaths: []` (`active-git-snapshot.test.ts:79`, `DocumentPane.test.ts:85`, `InspectorPane.test.ts:37`, `ChangesPane.test.ts:76`) hay que sacarlos.

Estrategia: **re-escribir por fase, no al final**. La repo valora cobertura (64 pruebas frontend + 80 Rust en la última auditoría de `PLAN_PENDIENTE.md`); una fase que rompe tests queda sin cerrar.

---

## 7. Riesgos

| Riesgo                                                           | Mitigación                                                                                                |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| **Neovim y `viewed` quedan huérfanos** (§3, fase 7)              | G.5 y E.7 los hacen conscious; si se rechaza el default, la capacidad se pierde y hay que dejarlo escrito |
| **Perder la vibrancia nativa** (B.4)                             | Es decisión tomada; queda registrado acá como cambio visual consciente                                    |
| **La virtualización se desalinea** con el nuevo row height (E.2) | `TREE_ROW_HEIGHT` y el CSS se derivan del mismo valor; test de alineación en fase 3                       |
| **xterm se desmonta al cambiar de vista** y queda con tamaño 0   | Se conserva `v-show` + `absolute inset-0` + prop `visible` de la fase 4                                   |
| **El header sin drag region deja la ventana inmovible** (A.7)    | El spacer `data-tauri-drag-region` va en la fase 1, no después                                            |
| **Perder funciones sin entrada de UI**                           | La tabla §3.1 las inventariza; las 5 sin replacement son decisión, las 8 restantes tienen destino         |
| **Rework de 106 tests**                                          | Por fase, no acumulado                                                                                    |
| **Doble sistema de tokens** si sobreviven `--surface-*`          | La fase 0 los borra; `pnpm lint` no lo caza, hay que buscar `--surface` a mano                            |
| **Fuentes distintas en UI y terminal** (B.6)                     | Default documentado; se acepta o se bundlea                                                               |

---

## 8. Checklist de cierre

- [ ] Fase 0: tokens marvis en la app, sin `--surface-*` ni `backdrop-filter`, `tauri.conf.json` sin transparencia, body monospace
- [ ] Fase 1: titlebar con lights nativos + drag region + crumbs + input, anchos 240/280, sin atajos ni toggles
- [ ] Fase 2: sidebar del mockup, worktree create/remove funcionando, "Open directory" al pie
- [ ] Fase 3: árbol con selección visible, Changes agrupado por directorio, fila de diff completo
- [ ] Fase 4: una sola vista activa (terminal / document / diff), `.md` renderizado
- [ ] Fase 5: toasts, sin banner de error
- [ ] Fase 6: diff con comentarios, `Send to opencode`, `↗ Neovim`
- [ ] Fase 7: palette borrada, `viewed` Cleaned o documentado como deuda
- [ ] `pnpm fmt:check` · `pnpm lint` · `pnpm lint:rust` · `pnpm typecheck` · `pnpm test:frontend` · `pnpm test:rust` · `pnpm test:security` · `pnpm build:app`
- [ ] Recorrido manual en la app nativa: crear worktree, abrir terminal, `nvim` con ligaduras, diff de un archivo, diff completo, comentar, enviar al agente, archivo `.md`, error de repo → texto rojo en la fila, redimensionar a angosto → drawer

---

## 9. Registro de decisiones pendientes

| #       | Pregunta                                             | Bloquea    | Default en uso                               |
| ------- | ---------------------------------------------------- | ---------- | -------------------------------------------- |
| B.6     | ¿Se bundlea JetBrains Mono?                          | Fase 0     | No, fallback del sistema                     |
| A.8     | ¿El chevron del crumb abre dropdown, o se saca?      | Fase 1     | Abre dropdown de items hermanos              |
| A.9     | ¿Double-click-to-zoom en la zona de arrastre?        | Fase 1     | Sí                                           |
| C.10    | ¿Punto de estado por sesión, o solo color del icono? | Fase 2     | Solo color del icono                         |
| D.2     | ¿Click en workdir muestra el item activo?            | Fase 2     | Sí, o el último                              |
| E.3-bis | ¿El badge de Changes cuenta archivos o notas?        | Fase 3     | Archivos                                     |
| E.7     | ¿Se cae el tracking "Viewed"?                        | Fase 3 + 7 | Se cae; 34 ocurrencias Rust quedan huérfanas |
| H.6     | ¿Se sigue guardando el ancho?                        | Fase 1     | Sí                                           |
| I.4     | ¿Se reescribe el copy?                               | Fase 5     | No, se conserva                              |
| G.5     | ¿Botón `↗ Neovim`?                                   | Fase 6     | Sí, junto a `↗ Zed`                          |
| G.6     | Copy del diff completo                               | Fase 6     | `All changes`                                |
| —       | ¿Se limpia `viewed` en Rust o se deja el dato?       | Fase 7     | Dejar el dato persistido sin leer            |
