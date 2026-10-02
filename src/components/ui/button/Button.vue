<script setup lang="ts">
/** Reusable action button with semantic visual variants and compact sizing. */
defineOptions({ name: "UiButton" });

withDefaults(
  defineProps<{
    disabled?: boolean;
    type?: "button" | "submit";
    variant?: "filled" | "tinted" | "outlined" | "subtle" | "ghost" | "danger";
    size?: "sm" | "md" | "lg";
  }>(),
  { disabled: false, type: "button", variant: "subtle", size: "md" },
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
