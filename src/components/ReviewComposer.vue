<script setup lang="ts">
import { computed, ref, watch } from "vue";

const props = withDefaults(
  defineProps<{
    side: "old" | "new";
    line: number;
    /** Last line of the range, when the draft spans more than one line. */
    lineEnd?: number | null;
    code?: string;
  }>(),
  { lineEnd: null, code: "" },
);
const emit = defineEmits<{
  submit: [content: string];
  cancel: [];
}>();

const content = ref("");
const input = ref<HTMLTextAreaElement | null>(null);
const isRange = computed(() => props.lineEnd !== null && props.lineEnd !== props.line);
const label = computed(() =>
  isRange.value ? `${props.side} lines ${props.line}-${props.lineEnd}` : `${props.side} line ${props.line}`,
);

watch(
  () => [props.side, props.line],
  async () => {
    content.value = "";
    await Promise.resolve();
    input.value?.focus();
  },
  { immediate: true },
);

function submit() {
  if (!content.value.trim()) return;
  emit("submit", content.value);
  content.value = "";
}
</script>

<template>
  <form
    class="m-1 rounded-sm border border-(--marvis-border) bg-(--marvis-bg-1) p-2 text-xs"
    aria-label="New review note"
    @submit.prevent="submit"
    @keydown.esc.stop.prevent="$emit('cancel')"
  >
    <p class="mb-1 font-mono text-[10px] text-(--marvis-text-faint)">
      {{ label }}
      <span v-if="code" class="ml-2 truncate text-(--marvis-text-secondary)">{{ code }}</span>
    </p>
    <textarea
      ref="input"
      v-model="content"
      rows="2"
      aria-label="Review note"
      placeholder="What should change here?"
      class="w-full resize-y rounded-sm border border-(--marvis-border) bg-(--marvis-bg-0) px-2 py-1 font-mono text-xs text-(--marvis-text) outline-none placeholder:text-(--marvis-text-faint)"
    />
    <p v-if="isRange" class="mt-1 text-[10px] text-(--marvis-text-faint)">
      Click another line to widen the range, or cancel to start over.
    </p>
    <div class="mt-1 flex items-center gap-2">
      <button
        type="submit"
        class="rounded-sm bg-(--marvis-border) px-2 py-0.5 text-[11px] text-(--marvis-text)"
        :disabled="!content.trim()"
      >
        Save note
      </button>
      <button
        type="button"
        class="rounded-sm px-2 py-0.5 text-[11px] text-(--marvis-text-faint) hover:text-(--marvis-text)"
        @click="$emit('cancel')"
      >
        Cancel
      </button>
    </div>
  </form>
</template>
