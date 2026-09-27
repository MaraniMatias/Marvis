<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
import { X as XIcon } from "@lucide/vue";
import { useToasts } from "../presentation/toasts";

const { toasts, dismiss, dismissNewest, hold, release } = useToasts();

/** A field owns its own Escape, so a toast never takes one away from what is being typed. */
function isEditableTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

function onKeydown(event: KeyboardEvent) {
  if (event.key !== "Escape" || isEditableTarget(event.target)) return;
  dismissNewest();
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <!-- The stack is chrome, not content: it never takes the focus, and the empty space around
       the toasts stays click-through so a toast cannot swallow a click on the panel. -->
  <div v-if="toasts.length" class="toast-stack">
    <div
      v-for="toast in toasts"
      :key="toast.id"
      class="toast"
      :class="`toast-${toast.level}`"
      :role="toast.level === 'error' ? 'alert' : 'status'"
      @mouseenter="hold(toast.id)"
      @mouseleave="release(toast.id)"
    >
      <span class="toast-accent" aria-hidden="true" />
      <p class="toast-message">{{ toast.message }}</p>
      <button type="button" class="toast-dismiss" :aria-label="`Dismiss: ${toast.message}`" @click="dismiss(toast.id)">
        <XIcon class="icon-xs" aria-hidden="true" />
      </button>
    </div>
  </div>
</template>

<style scoped>
/* Bottom center over the main panel. Newest last in the DOM and first at the bottom, so
   every toast pushes the ones above it up instead of covering them. Above the drawer and the
   splitter handles, below the worktree dialog's scrim: a modal ask comes first. */
.toast-stack {
  position: fixed;
  z-index: 40;
  bottom: 1rem;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  flex-direction: column-reverse;
  align-items: center;
  gap: 6px;
  width: min(420px, calc(100vw - 2rem));
  pointer-events: none;
}

.toast {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  width: 100%;
  padding: 8px 8px 8px 10px;
  border: 1px solid var(--marvis-border);
  border-radius: var(--marvis-radius);
  background: var(--marvis-bg-2);
  color: var(--marvis-text);
  font-size: 13px;
  line-height: 1.4;
  pointer-events: auto;
}

.toast-accent {
  width: 6px;
  height: 6px;
  margin-top: 6px;
  flex-shrink: 0;
  border-radius: 50%;
  background: var(--marvis-text-secondary);
}

.toast-error .toast-accent {
  background: var(--marvis-red);
}

.toast-success .toast-accent {
  background: var(--marvis-green);
}

.toast-message {
  flex: 1;
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}

.toast-dismiss {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  padding: 0;
  border: none;
  border-radius: var(--marvis-radius);
  background: transparent;
  color: var(--marvis-text-secondary);
  cursor: pointer;
}

.toast-dismiss:hover {
  background: var(--marvis-border);
  color: var(--marvis-text);
}
</style>
