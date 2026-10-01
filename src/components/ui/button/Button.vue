<script setup lang="ts">
/** Reusable action button with semantic visual variants and compact sizing. */
defineOptions({ name: "UiButton" });

withDefaults(
  defineProps<{
    disabled?: boolean;
    type?: "button" | "submit";
    variant?: "primary" | "secondary" | "quiet" | "danger";
    size?: "sm" | "md";
  }>(),
  { disabled: false, type: "button", variant: "secondary", size: "md" },
);
</script>

<template>
  <button
    :type="type"
    :disabled="disabled"
    class="marvis-button"
    :class="[`marvis-button-${variant}`, `marvis-button-${size}`]"
  >
    <slot />
  </button>
</template>
