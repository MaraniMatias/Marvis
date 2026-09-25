<script setup lang="ts">
defineProps<{
  modelValue?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();
</script>

<template>
  <input
    :value="modelValue"
    class="h-9 min-w-0 rounded-md border border-slate-700 bg-slate-950/60 px-3 text-xs text-slate-100 placeholder:text-slate-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-400"
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
