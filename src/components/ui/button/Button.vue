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
    class="inline-flex h-8 items-center justify-center gap-2 rounded-sm px-3 text-sm font-medium transition-colors disabled:cursor-wait disabled:opacity-60"
    :class="
      variant === 'primary'
        ? 'bg-sky-300 text-[#0b1118] hover:bg-sky-200'
        : 'text-zinc-300 hover:bg-white/8 hover:text-white'
    "
  >
    <slot />
  </button>
</template>
