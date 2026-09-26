<script setup lang="ts">
import { ref } from "vue";
import { anchorOutcome, reviewLineRange } from "../domain/review";
import type { AnchorOutcome, ReviewNote } from "../domain/review";

const props = defineProps<{
  notes: ReviewNote[];
  /** What the diff can prove about each anchor; absent means "not judged". */
  outcomes?: Map<string, AnchorOutcome>;
}>();
const emit = defineEmits<{
  updateNote: [id: string, content: string];
  deleteNote: [id: string];
  clearOutdated: [id: string];
  resolveNote: [id: string];
}>();

const editingId = ref<string | null>(null);
const draft = ref("");

function startEditing(note: ReviewNote) {
  editingId.value = note.id;
  draft.value = note.content;
}

function save(id: string) {
  if (!draft.value.trim()) return;
  emit("updateNote", id, draft.value);
  editingId.value = null;
}

/** The verdict for a note the agent has already seen, or null when it is not judged. */
function verdictFor(note: ReviewNote) {
  const outcome = props.outcomes?.get(note.id);
  return outcome ? anchorOutcome(outcome) : null;
}

/**
 * Resolving is offered only when the diff can back it. Without a verdict the note predates
 * this render, and offering the button would let the user mark something resolved that
 * nothing has actually checked.
 */
function canResolve(note: ReviewNote): boolean {
  return verdictFor(note)?.resolvable === true;
}
</script>

<template>
  <ul class="m-0 list-none space-y-1 p-1">
    <li
      v-for="note in props.notes"
      :key="note.id"
      class="rounded-sm border px-2 py-1 text-xs"
      :class="note.outdated ? 'border-amber-500/50 bg-amber-500/10' : 'border-amber-500/25 bg-amber-500/5'"
    >
      <p class="flex items-center gap-2 font-mono text-[10px] text-zinc-500">
        <span>{{ note.side }}:{{ reviewLineRange(note) }}</span>
        <span v-if="note.status === 'sent'" class="text-sky-400">sent</span>
        <span v-if="note.status === 'resolved'" class="text-emerald-400">resolved</span>
        <span v-if="note.outdated" class="text-amber-400">outdated</span>
        <span class="ml-auto flex gap-2">
          <button
            v-if="note.status === 'sent' && !note.outdated && canResolve(note)"
            type="button"
            class="text-emerald-300 hover:text-emerald-100"
            :aria-label="`Mark note on line ${note.lineStart} as resolved`"
            @click="$emit('resolveNote', note.id)"
          >
            Resolved
          </button>
          <button
            v-if="note.outdated"
            type="button"
            class="text-amber-300 hover:text-amber-100"
            :aria-label="`Accept note on line ${note.lineStart} even though the line changed`"
            @click="$emit('clearOutdated', note.id)"
          >
            Accept
          </button>
          <button
            type="button"
            class="text-zinc-400 hover:text-zinc-100"
            :aria-label="`Edit note on line ${note.lineStart}`"
            @click="startEditing(note)"
          >
            Edit
          </button>
          <button
            type="button"
            class="text-zinc-400 hover:text-red-300"
            :aria-label="`Delete note on line ${note.lineStart}`"
            @click="$emit('deleteNote', note.id)"
          >
            Delete
          </button>
        </span>
      </p>
      <form v-if="editingId === note.id" aria-label="Edit review note" @submit.prevent="save(note.id)">
        <textarea
          v-model="draft"
          rows="2"
          aria-label="Review note"
          class="mt-1 w-full resize-y rounded-sm border border-white/10 bg-black/30 px-2 py-1 font-sans text-xs text-zinc-100 outline-none"
          @keydown.esc.stop.prevent="editingId = null"
        />
        <div class="mt-1 flex gap-2">
          <button type="submit" class="rounded-sm bg-sky-500/20 px-2 py-0.5 text-[11px] text-sky-200">Save</button>
          <button type="button" class="rounded-sm px-2 py-0.5 text-[11px] text-zinc-500" @click="editingId = null">
            Cancel
          </button>
        </div>
      </form>
      <p v-else class="mt-0.5 whitespace-pre-wrap text-zinc-200">{{ note.content }}</p>
      <p v-if="note.outdated" class="mt-0.5 text-[10px] text-amber-400/90">
        The line this note points at changed. It is kept out of the review sent to the agent.
      </p>
      <p
        v-else-if="note.status === 'sent' && verdictFor(note)"
        class="mt-0.5 text-[10px]"
        :class="verdictFor(note)!.resolvable ? 'text-zinc-500' : 'text-zinc-600'"
      >
        {{ verdictFor(note)!.message }}
      </p>
    </li>
  </ul>
</template>
