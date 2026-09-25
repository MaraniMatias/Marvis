<script setup lang="ts">
withDefaults(
  defineProps<{
    variant?: "default" | "outline";
  }>(),
  { variant: "default" },
);
</script>

<template>
  <button
    type="button"
    :class="[
      'inline-flex min-h-9 items-center justify-center rounded-md px-3 text-xs font-semibold whitespace-nowrap transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400 disabled:pointer-events-none disabled:opacity-40',
      variant === 'default'
        ? 'border border-slate-600 bg-slate-800 text-slate-100 hover:border-sky-300 hover:bg-slate-700'
        : 'border border-slate-700 bg-transparent text-slate-200 hover:border-sky-300 hover:bg-slate-800',
    ]"
  >
    <slot />
  </button>
</template>
