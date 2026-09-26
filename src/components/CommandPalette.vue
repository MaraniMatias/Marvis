<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { PaletteCommand, PaletteCommandId } from "../domain/command-palette";

const props = withDefaults(defineProps<{ commands: PaletteCommand[]; openRequestToken?: number }>(), {
  openRequestToken: 0,
});
const emit = defineEmits<{ select: [command: PaletteCommandId] }>();

const open = ref(false);
const query = ref("");
const selectedIndex = ref(0);
const searchInput = ref<HTMLInputElement | null>(null);
const filteredCommands = computed(() => {
  const normalized = query.value.trim().toLocaleLowerCase();
  return props.commands.filter((command) => !normalized || command.label.toLocaleLowerCase().includes(normalized));
});

watch(filteredCommands, () => {
  selectedIndex.value = 0;
});

watch(open, async (isOpen) => {
  if (!isOpen) return;
  query.value = "";
  await nextTick();
  searchInput.value?.focus();
});

watch(
  () => props.openRequestToken,
  (token, previous) => {
    if (token !== previous) open.value = true;
  },
);

function close() {
  open.value = false;
}

function select(command: PaletteCommand) {
  if (!command.enabled) return;
  emit("select", command.id);
  close();
}

function handleKeydown(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    open.value = !open.value;
    return;
  }
  if (!open.value) return;
  if (event.key === "Escape") {
    event.preventDefault();
    close();
  } else if (event.key === "ArrowDown" && filteredCommands.value.length) {
    event.preventDefault();
    selectedIndex.value = (selectedIndex.value + 1) % filteredCommands.value.length;
  } else if (event.key === "ArrowUp" && filteredCommands.value.length) {
    event.preventDefault();
    selectedIndex.value = (selectedIndex.value - 1 + filteredCommands.value.length) % filteredCommands.value.length;
  } else if (event.key === "Enter") {
    event.preventDefault();
    const command = filteredCommands.value[selectedIndex.value];
    if (command) select(command);
  }
}

onMounted(() => window.addEventListener("keydown", handleKeydown));
onUnmounted(() => window.removeEventListener("keydown", handleKeydown));
</script>

<template>
  <div
    v-if="open"
    class="absolute inset-x-0 bottom-0 top-12 z-50 flex items-start justify-center bg-black/55 px-4 pt-[12vh]"
    @click.self="close"
  >
    <section
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      class="surface-popover w-full max-w-lg overflow-hidden rounded-xl border border-white/10 shadow-2xl"
    >
      <input
        ref="searchInput"
        v-model="query"
        aria-label="Filter commands"
        placeholder="Type a command…"
        class="h-12 w-full border-b border-white/8 bg-transparent px-4 text-sm text-zinc-100 outline-none placeholder:text-zinc-500"
      />
      <div class="max-h-[55vh] overflow-y-auto p-2" role="listbox" aria-label="Commands">
        <p v-if="filteredCommands.length === 0" role="status" class="px-3 py-5 text-sm text-zinc-500">
          No matching commands.
        </p>
        <button
          v-for="(command, index) in filteredCommands"
          :key="command.id"
          type="button"
          role="option"
          :aria-selected="index === selectedIndex"
          :aria-disabled="!command.enabled"
          :disabled="!command.enabled"
          :title="command.disabledReason"
          class="flex w-full items-center justify-between rounded-md px-3 py-2.5 text-left text-sm"
          :class="[
            index === selectedIndex ? 'bg-white/8 text-zinc-100' : 'text-zinc-300 hover:bg-white/5',
            !command.enabled && 'cursor-not-allowed opacity-45',
          ]"
          @mouseenter="selectedIndex = index"
          @click="select(command)"
        >
          <span>{{ command.label }}</span>
          <span v-if="command.disabledReason" class="ml-4 text-[11px] text-amber-300">
            {{ command.disabledReason }}
          </span>
        </button>
      </div>
      <footer class="border-t border-white/8 px-4 py-2 text-[10px] text-zinc-600">
        ↑↓ navigate · Enter select · Esc close
      </footer>
    </section>
  </div>
</template>
