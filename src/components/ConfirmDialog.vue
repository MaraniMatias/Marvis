<script setup lang="ts">
/* The overlay and the surface are the ones `WorktreeDialog` draws, so a question asked here
   looks like the rest of the app rather than like the browser's own dialog. */
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    /** The body of the question: what the action does, and what it leaves alone. */
    message: string;
    confirmLabel: string;
    /** Fills the confirming button in the failure colour, for the answers that are not neutral. */
    destructive?: boolean;
    busy?: boolean;
  }>(),
  { destructive: false, busy: false },
);

const emit = defineEmits<{ confirm: []; close: [] }>();

/** The heading names the dialog, so it is what a screen reader announces on open. */
const titleId = computed(() => `confirm-${props.title.replace(/\W+/g, "-").toLowerCase()}`);
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-50 grid place-items-center bg-(--marvis-bg-0)/80 p-4"
    @click.self="emit('close')"
  >
    <section
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      :aria-labelledby="titleId"
      class="surface-popover w-full max-w-md rounded-[var(--marvis-radius)] p-5 shadow-2xl"
      @keydown.esc.stop.prevent="emit('close')"
    >
      <h2 :id="titleId" class="text-base font-semibold text-(--marvis-text)">{{ title }}</h2>
      <p class="mt-2 text-sm text-(--marvis-text-secondary)">{{ message }}</p>
      <footer class="mt-5 flex justify-end gap-2">
        <!-- Focus opens on the answer that does nothing, so a stray Enter cannot answer yes. -->
        <button
          type="button"
          autofocus
          class="marvis-control px-3 py-2 text-xs text-(--marvis-text-secondary) hover:text-(--marvis-text)"
          @click="emit('close')"
        >
          Cancel
        </button>
        <button
          type="button"
          :disabled="busy"
          :class="destructive ? 'marvis-button-danger' : 'marvis-control px-3 py-2 text-xs text-(--marvis-text)'"
          @click="emit('confirm')"
        >
          {{ busy ? "Working…" : confirmLabel }}
        </button>
      </footer>
    </section>
  </div>
</template>
