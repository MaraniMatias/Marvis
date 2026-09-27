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
    <!-- The palette has no warning hue, so a note reads the diff's own: red is a line that
         changed or is gone, green is a note the agent has settled, and everything else that is
         merely a fact is text. The card itself only carries the border, because the chip and the
         line below it already say which state the note is in. -->
    <li
      v-for="note in props.notes"
      :key="note.id"
      class="rounded-sm border px-2 py-1 text-xs"
      :class="
        note.outdated ? 'border-(--marvis-border) bg-(--marvis-bg-1)' : 'border-(--marvis-border) bg-(--marvis-bg-0)'
      "
    >
      <p class="flex items-center gap-2 font-mono text-[10px] text-(--marvis-text-faint)">
        <span>{{ note.side }}:{{ reviewLineRange(note) }}</span>
        <span v-if="note.status === 'sent'" class="text-(--marvis-text-secondary)">sent</span>
        <span v-if="note.status === 'resolved'" class="text-(--marvis-green)">resolved</span>
        <span v-if="note.outdated" class="text-(--marvis-red)">outdated</span>
        <span class="ml-auto flex gap-2">
          <button
            v-if="note.status === 'sent' && !note.outdated && canResolve(note)"
            type="button"
            class="text-(--marvis-green) hover:text-(--marvis-text)"
            :aria-label="`Mark note on line ${note.lineStart} as resolved`"
            @click="$emit('resolveNote', note.id)"
          >
            Resolved
          </button>
          <button
            v-if="note.outdated"
            type="button"
            class="text-(--marvis-text-secondary) hover:text-(--marvis-text)"
            :aria-label="`Accept note on line ${note.lineStart} even though the line changed`"
            @click="$emit('clearOutdated', note.id)"
          >
            Accept
          </button>
          <button
            type="button"
            class="text-(--marvis-text-secondary) hover:text-(--marvis-text)"
            :aria-label="`Edit note on line ${note.lineStart}`"
            @click="startEditing(note)"
          >
            Edit
          </button>
          <button
            type="button"
            class="text-(--marvis-text-secondary) hover:text-(--marvis-red)"
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
          class="mt-1 w-full resize-y rounded-sm border border-(--marvis-border) bg-(--marvis-bg-0) px-2 py-1 font-mono text-xs text-(--marvis-text) outline-none"
          @keydown.esc.stop.prevent="editingId = null"
        />
        <div class="mt-1 flex gap-2">
          <button type="submit" class="rounded-sm bg-(--marvis-border) px-2 py-0.5 text-[11px] text-(--marvis-text)">
            Save
          </button>
          <button
            type="button"
            class="rounded-sm px-2 py-0.5 text-[11px] text-(--marvis-text-faint) hover:text-(--marvis-text)"
            @click="editingId = null"
          >
            Cancel
          </button>
        </div>
      </form>
      <p v-else class="mt-0.5 whitespace-pre-wrap text-(--marvis-text)">{{ note.content }}</p>
      <p v-if="note.outdated" class="mt-0.5 text-[10px] text-(--marvis-red)">
        The line this note points at changed. It is kept out of the review sent to the agent.
      </p>
      <p
        v-else-if="note.status === 'sent' && verdictFor(note)"
        class="mt-0.5 text-[10px]"
        :class="verdictFor(note)!.resolvable ? 'text-(--marvis-text-dim)' : 'text-(--marvis-text-faint)'"
      >
        {{ verdictFor(note)!.message }}
      </p>
    </li>
  </ul>
</template>
