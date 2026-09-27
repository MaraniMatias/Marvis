# layout-spike

A minimal Vue 3 + Vite + Tailwind v4 app that lays out a desktop shell with
[shadcn-vue](https://www.shadcn-vue.com) `Resizable` (which is a thin wrapper over
[Reka UI](https://reka-ui.com)'s `SplitterGroup` / `SplitterPanel` / `SplitterResizeHandle`).

```
|ooo|titlebar|
|sidebar |main |sidebar|
```

`src/App.vue` builds a titlebar over a horizontal `ResizablePanelGroup` of three panels, and
splits the centre panel again vertically (editor / terminal) to exercise nested groups.

## Run

```sh
pnpm install
pnpm dev      # http://localhost:5173
pnpm build    # vue-tsc --noEmit && vite build
```

## Layout

- `src/components/ui/resizable/` — the shadcn-vue `Resizable` component, vendored verbatim from
  the registry. Re-run `pnpm dlx shadcn-vue@latest add resizable` to refresh it.
- `src/lib/utils.ts` — the shadcn-vue `cn` helper (`clsx` + `tailwind-merge`).
- `src/style.css` — Tailwind v4 `@theme inline` tokens. Only the semantic tokens these
  components use are declared; add more as components are added.
- `components.json` — registry config, so `shadcn-vue add <component>` resolves `@/components`,
  `@/lib/utils` and `src/style.css`.
