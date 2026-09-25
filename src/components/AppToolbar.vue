<script setup lang="ts">
import type { AppLayoutState } from "../domain/ui-state";

defineProps<{ layout: AppLayoutState; narrow: boolean }>();
defineEmits<{
  toggleSidebar: [];
  toggleInspector: [];
  toggleStatusBar: [];
  toggleFocus: [];
  toggleTransparency: [];
  resetLayout: [];
  openCommands: [];
}>();
</script>

<template>
  <header class="app-toolbar flex h-12 shrink-0 items-center gap-2 border-b border-white/8 px-3 pl-[76px]">
    <button
      type="button"
      class="toolbar-button"
      :aria-pressed="layout.sidebarVisible"
      :title="layout.sidebarVisible ? 'Hide navigation' : 'Show navigation'"
      :aria-label="layout.sidebarVisible ? 'Hide navigation' : 'Show navigation'"
      @click="$emit('toggleSidebar')"
    >
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <rect x="2.5" y="3" width="15" height="14" rx="2" />
        <path d="M7 3v14" />
      </svg>
    </button>
    <button
      type="button"
      class="toolbar-button"
      :aria-pressed="layout.inspectorVisible"
      :title="layout.inspectorVisible ? 'Hide inspector' : 'Show inspector'"
      :aria-label="layout.inspectorVisible ? 'Hide inspector' : 'Show inspector'"
      @click="$emit('toggleInspector')"
    >
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <rect x="2.5" y="3" width="15" height="14" rx="2" />
        <path d="M13 3v14" />
      </svg>
    </button>
    <button
      type="button"
      class="toolbar-button"
      :aria-pressed="layout.statusBarVisible"
      :title="layout.statusBarVisible ? 'Hide status bar' : 'Show status bar'"
      :aria-label="layout.statusBarVisible ? 'Hide status bar' : 'Show status bar'"
      @click="$emit('toggleStatusBar')"
    >
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <rect x="2.5" y="3" width="15" height="14" rx="2" />
        <path d="M3 13.5h14" />
      </svg>
    </button>
    <div class="toolbar-drag-region min-w-8 flex-1" data-tauri-drag-region aria-hidden="true" />
    <button
      type="button"
      class="toolbar-button focus-button"
      :aria-pressed="layout.focusSnapshot !== null"
      @click="$emit('toggleFocus')"
    >
      {{ layout.focusSnapshot ? "Exit Focus" : "Focus" }}
    </button>
    <button
      type="button"
      class="toolbar-button command-button"
      title="Open command palette (⌘K)"
      @click="$emit('openCommands')"
    >
      <span>⌘K</span>
      <span class="sr-only">Open command palette</span>
    </button>
    <details class="toolbar-settings">
      <summary class="toolbar-button" aria-label="Toolbar settings" title="Toolbar settings">···</summary>
      <div class="toolbar-menu">
        <button type="button" :aria-pressed="layout.reduceTransparency" @click="$emit('toggleTransparency')">
          {{ layout.reduceTransparency ? "✓" : "" }} Reduce transparency
        </button>
        <button type="button" :aria-pressed="layout.statusBarVisible" @click="$emit('toggleStatusBar')">
          {{ layout.statusBarVisible ? "✓" : "" }} Show status bar
        </button>
        <button type="button" @click="$emit('resetLayout')">Reset layout</button>
      </div>
    </details>
  </header>
</template>

<style scoped>
.toolbar-button {
  display: inline-flex;
  min-width: 30px;
  height: 30px;
  align-items: center;
  justify-content: center;
  gap: 7px;
  border-radius: 6px;
  padding: 0 8px;
  color: #a1a1aa;
  font-size: 12px;
  transition:
    background-color 120ms ease,
    color 120ms ease;
}

.toolbar-button:hover,
.toolbar-button[aria-pressed="true"] {
  background: rgb(255 255 255 / 8%);
  color: #f4f4f5;
}

.toolbar-button:focus-visible,
.toolbar-settings summary:focus-visible {
  outline: 2px solid #60a5fa;
  outline-offset: 2px;
}

.toolbar-button svg {
  width: 16px;
  height: 16px;
  fill: none;
  stroke: currentColor;
  stroke-width: 1.35;
}

.toolbar-settings {
  position: relative;
}

.toolbar-settings summary {
  list-style: none;
  cursor: pointer;
  font-size: 18px;
  letter-spacing: 1px;
}

.toolbar-settings summary::-webkit-details-marker {
  display: none;
}

.toolbar-menu {
  position: absolute;
  z-index: 40;
  top: 36px;
  right: 0;
  width: 210px;
  border: 1px solid rgb(255 255 255 / 12%);
  border-radius: 8px;
  background: rgb(31 34 41 / 92%);
  padding: 5px;
  box-shadow: 0 12px 35px rgb(0 0 0 / 35%);
  backdrop-filter: blur(22px);
}

.toolbar-menu button {
  display: block;
  width: 100%;
  border-radius: 5px;
  padding: 8px;
  color: #e4e4e7;
  text-align: left;
  font-size: 12px;
}

.toolbar-menu button:hover {
  background: rgb(255 255 255 / 8%);
}

@media (prefers-reduced-motion: reduce) {
  .toolbar-button {
    transition: none;
  }
}
</style>
