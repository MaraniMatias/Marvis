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
    class="inline-flex h-9 items-center justify-center gap-2 rounded-md px-3 text-sm font-medium transition-colors disabled:cursor-wait disabled:opacity-60"
    :class="
      variant === 'primary'
        ? 'bg-indigo-400 text-[#111318] hover:bg-indigo-300'
        : 'text-zinc-300 hover:bg-white/8 hover:text-white'
    "
  >
    <slot />
  </button>
</template>
