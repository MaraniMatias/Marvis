<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
/**
 * The preferences, drawn from the schema in `src/domain/settings.ts`.
 *
 * The form is generated rather than written out, so adding a preference is a field in the schema
 * and not a row of template: the dialog has no per-preference branch anywhere, which is the only
 * way it stays true as the list grows.
 *
 * Editing is against a draft, and nothing reaches the app or the file until Apply. That is what
 * makes Cancel possible at all, and it is also what keeps a half-chosen font size from being drawn
 * while it is being picked.
 */
import { computed, nextTick, ref, watch } from "vue";
import { DEFAULT_SETTINGS, SETTINGS_SECTIONS, cloneSettings, valueAt, withValue } from "../domain/settings";
import type { AppSettings, SettingsField, SettingsPath, SettingsValue } from "../domain/settings";
import SelectControl from "./ui/select/SelectControl.vue";

const props = defineProps<{
  open: boolean;
  settings: AppSettings;
  saving: boolean;
}>();

const emit = defineEmits<{
  close: [];
  /** The whole set, because the file holds the whole set and a partial save is not one. */
  apply: [settings: AppSettings];
}>();

const draft = ref<AppSettings>(props.settings);
const dialogElement = ref<HTMLElement | null>(null);
const cancelButton = ref<HTMLButtonElement | null>(null);

watch(
  () => props.open,
  async (open) => {
    if (!open) return;
    await nextTick();
    cancelButton.value?.focus();
  },
  { flush: "post" },
);

watch(
  () => props.saving,
  async (saving) => {
    if (!saving) return;
    await nextTick();
    dialogElement.value?.focus();
  },
);

/** Replaced whole on every open, so a draft left half-typed is never what the next open shows. */
watch(
  () => props.open,
  (open) => {
    if (open) draft.value = cloneSettings(props.settings);
  },
  { immediate: true },
);

const isDefault = computed(() => JSON.stringify(draft.value) === JSON.stringify(DEFAULT_SETTINGS));

function change(path: SettingsPath, value: SettingsValue) {
  draft.value = withValue(draft.value, path, value);
}

/**
 * The field's own range, applied as it is typed rather than at Apply: a font size of 400 is not a
 * preference, it is a number this app cannot draw, and saying so while the cursor is still in the
 * field is cheaper than after the window has been redrawn at it.
 */
function changeNumber(field: SettingsField & { kind: "number" }, input: HTMLInputElement) {
  const parsed = Number(input.value);
  if (!Number.isFinite(parsed)) return;
  if (field.path === "editor.indentation.size" && !Number.isInteger(parsed)) {
    input.value = String(valueAt(draft.value, field.path));
    return;
  }
  change(field.path, Math.min(field.limits.max, Math.max(field.limits.min, parsed)));
}

/** Every dot, not the first: `editor.indentation.size` has two, and a `for` that points at an id
 * nothing has is a label that does nothing when it is clicked. */
function controlId(path: SettingsPath) {
  return `settings-${path.replaceAll(".", "-")}`;
}

/** Reset replaces the draft rather than editing it, for the reason `withValue` copies. */
function resetDraft() {
  draft.value = cloneSettings(DEFAULT_SETTINGS);
}

function requestClose() {
  if (!props.saving) emit("close");
}

function onDialogKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.stopPropagation();
    event.preventDefault();
    requestClose();
    return;
  }
  if (event.key !== "Tab") return;

  const dialog = dialogElement.value;
  if (!dialog) return;
  const focusable = dialog.querySelectorAll<HTMLElement>(
    'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])',
  );
  if (!focusable.length) {
    event.preventDefault();
    dialog.focus();
    return;
  }
  const first = focusable[0]!;
  const last = focusable[focusable.length - 1]!;
  const active = document.activeElement;
  if (event.shiftKey && (active === first || !dialog.contains(active))) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
    event.preventDefault();
    first.focus();
  }
}
</script>

<template>
  <div
    v-if="open"
    class="fixed inset-0 z-50 grid place-items-center bg-(--marvis-bg-0)/80 p-4"
    @click.self="requestClose"
  >
    <section
      ref="dialogElement"
      role="dialog"
      aria-modal="true"
      :aria-busy="saving"
      aria-labelledby="settings-title"
      tabindex="-1"
      class="surface-popover flex max-h-[85vh] w-full max-w-xl flex-col rounded-[var(--marvis-radius)] shadow-2xl"
      @keydown="onDialogKeydown"
    >
      <header class="flex items-start justify-between gap-4 px-5 pt-5">
        <div>
          <h2 id="settings-title" class="text-base font-semibold text-(--marvis-text)">Settings</h2>
          <p class="mt-1 text-xs text-(--marvis-text-faint)">Saved to ~/.marvis/config.yml</p>
        </div>
        <button
          type="button"
          aria-label="Close"
          :disabled="saving"
          class="marvis-control px-2 py-1 text-(--marvis-text-secondary) hover:text-(--marvis-text)"
          @click="requestClose"
        >
          ×
        </button>
      </header>

      <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
        <section v-for="section in SETTINGS_SECTIONS" :key="section.id" class="mb-5 last:mb-0">
          <h3 class="mb-2 text-[0.6875rem] font-semibold tracking-wide text-(--marvis-text-faint) uppercase">
            {{ section.title }}
          </h3>
          <div class="grid grid-cols-[1fr_8.5rem] items-center gap-x-4 gap-y-3">
            <!-- One pair per field, label then control, so a grid row is a field rather than a
                 column of labels beside a column of controls that have to be matched by eye. -->
            <template v-for="field in section.fields" :key="field.path">
              <label :for="controlId(field.path)">
                <span class="block text-xs text-(--marvis-text)">{{ field.label }}</span>
                <span v-if="field.description" class="mt-0.5 block text-[0.6875rem] text-(--marvis-text-faint)">
                  {{ field.description }}
                </span>
              </label>

              <div>
                <input
                  v-if="field.kind === 'toggle'"
                  :id="controlId(field.path)"
                  type="checkbox"
                  :disabled="saving"
                  class="marvis-check size-4"
                  :checked="valueAt(draft, field.path) === true"
                  @change="change(field.path, !valueAt(draft, field.path))"
                />

                <div v-else-if="field.kind === 'number'" class="flex items-center gap-2">
                  <input
                    :id="controlId(field.path)"
                    type="number"
                    :disabled="saving"
                    inputmode="numeric"
                    class="marvis-input w-full"
                    :min="field.limits.min"
                    :max="field.limits.max"
                    :step="field.step ?? 1"
                    :value="valueAt(draft, field.path)"
                    @change="changeNumber(field, $event.target as HTMLInputElement)"
                  />
                  <span v-if="field.unit" class="w-6 shrink-0 text-[0.6875rem] text-(--marvis-text-faint)">
                    {{ field.unit }}
                  </span>
                </div>

                <SelectControl
                  v-else
                  :id="controlId(field.path)"
                  :model-value="String(valueAt(draft, field.path))"
                  :options="field.options"
                  :disabled="saving"
                  variant="field"
                  :label="field.label"
                  @update:model-value="change(field.path, field.parse($event))"
                />
              </div>
            </template>
          </div>
        </section>
      </div>

      <footer class="flex items-center gap-2 border-t border-(--marvis-border) px-5 py-4">
        <button
          type="button"
          :disabled="isDefault || saving"
          class="marvis-control px-3 py-2 text-xs text-(--marvis-text-secondary) hover:text-(--marvis-text)"
          @click="resetDraft"
        >
          Reset to defaults
        </button>
        <span class="flex-1" />
        <button
          ref="cancelButton"
          type="button"
          autofocus
          :disabled="saving"
          class="marvis-control px-3 py-2 text-xs text-(--marvis-text-secondary) hover:text-(--marvis-text)"
          @click="requestClose"
        >
          Cancel
        </button>
        <button
          type="button"
          :disabled="saving"
          class="marvis-control px-3 py-2 text-xs text-(--marvis-text) hover:text-(--marvis-text)"
          @click="emit('apply', draft)"
        >
          {{ saving ? "Saving…" : "Apply" }}
        </button>
      </footer>
    </section>
  </div>
</template>

<style scoped>
/* The input the dialog's own numbers are drawn from, so a number and a picker in the same column
   have the same height and the same border. It matches the inputs in `WorktreeDialog` and the
   `field` variant of `SelectControl`, which is the pair the two kinds of control have to agree
   with for the column to read as one. */
.marvis-input {
  border: 1px solid var(--marvis-border);
  border-radius: var(--marvis-radius);
  background: var(--marvis-bg-0);
  padding: 8px 12px;
  font: inherit;
  font-size: 0.875rem;
  color: var(--marvis-text);
  outline: none;
}
.marvis-input:hover,
.marvis-input:focus {
  border-color: var(--marvis-text-faint);
}
.marvis-check {
  margin-left: auto;
  accent-color: var(--marvis-accent);
}
</style>
