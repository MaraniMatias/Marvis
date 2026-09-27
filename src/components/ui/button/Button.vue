<script setup lang="ts">
/**
 * A quiet action, in the mockup's own vocabulary.
 *
 * There is no filled accent button in the mockup: `--marvis-accent` only ever paints a
 * selection bar, an active icon, the prompt or a focus ring. An accent-filled button with
 * dark text reads as a different product, so the accent stays out of the surface and the
 * action sits on the background until it is hovered.
 */
defineOptions({ name: "UiButton" });

withDefaults(
  defineProps<{
    disabled?: boolean;
    type?: "button" | "submit";
  }>(),
  { disabled: false, type: "button" },
);
</script>

<template>
  <button
    :type="type"
    :disabled="disabled"
    class="inline-flex h-7 items-center justify-center gap-2 rounded-[var(--marvis-radius)] px-2.5 text-xs text-(--marvis-text-dim) transition-colors hover:bg-(--marvis-bg-2) hover:text-(--marvis-text) disabled:cursor-wait disabled:opacity-60"
  >
    <slot />
  </button>
</template>
