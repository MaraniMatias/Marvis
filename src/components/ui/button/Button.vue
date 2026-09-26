<script setup lang="ts">
defineOptions({ name: "UiButton" });

withDefaults(
  defineProps<{
    variant?: "primary" | "quiet";
    disabled?: boolean;
    type?: "button" | "submit";
  }>(),
  { variant: "primary", disabled: false, type: "button" },
);
</script>

<template>
  <button
    :type="type"
    :disabled="disabled"
    class="inline-flex h-8 items-center justify-center gap-2 rounded-[var(--marvis-radius)] px-3 text-sm font-medium transition-colors disabled:cursor-wait disabled:opacity-60"
    :class="
      variant === 'primary'
        ? 'bg-(--marvis-accent) text-(--marvis-bg-0) hover:brightness-110'
        : 'text-(--marvis-text-secondary) hover:bg-(--marvis-bg-2) hover:text-(--marvis-text)'
    "
  >
    <slot />
  </button>
</template>
