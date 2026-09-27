<script setup lang="ts">
import { computed, ref } from "vue";
import {
  DropdownMenuContent,
  DropdownMenuFilter,
  DropdownMenuItem,
  DropdownMenuPortal,
  DropdownMenuRoot,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "reka-ui";
import { Check as CheckIcon } from "@lucide/vue";
import type { TitlebarMenuItem, TitlebarMenuSection } from "../domain/titlebar-menu";
import { visibleSections } from "../domain/titlebar-menu";

const props = withDefaults(
  defineProps<{
    /** What the crumb says. It is the whole name too, so a truncated crumb can be read. */
    label: string;
    sections: TitlebarMenuSection[];
    /**
     * The crumb menus share one open state in the shell, so opening one closes the others.
     * The component does not decide that on its own: it reports and obeys.
     */
    open: boolean;
    /** When set, the menu opens on a search field over the rows. */
    searchPlaceholder?: string;
    align?: "start" | "end";
    /** Where this crumb sits in the line, which is what says how bright it reads and what it
     *  gives up when the line runs out of room. */
    crumb?: "workdir" | "branch" | "item";
    testid?: string;
  }>(),
  { searchPlaceholder: undefined, align: "start", crumb: "item", testid: undefined },
);

const emit = defineEmits<{ "update:open": [boolean] }>();

const query = ref("");
const sections = computed(() => visibleSections(props.sections, query.value));
/**
 * What the search emptied, to say so. A menu that was never a list — one that only ever had a
 * note and an action — has nothing to report, and the actions at the foot never count as a hit.
 */
const nothingMatched = computed(
  () =>
    !!props.searchPlaceholder &&
    !sections.value.some((section) => section.kind !== "separator" && section.items.some((item) => !item.pinned)),
);

function rows(section: TitlebarMenuSection): TitlebarMenuItem[] {
  return section.kind === "separator" ? [] : section.items;
}

/** Closing forgets what was typed, so the menu opens on the whole list again. */
function onOpenChange(open: boolean) {
  if (!open) query.value = "";
  emit("update:open", open);
}
</script>

<template>
  <DropdownMenuRoot :open="open" @update:open="onOpenChange">
    <DropdownMenuTrigger
      class="crumb-control select-none"
      :class="`crumb-${crumb}`"
      :data-testid="testid"
      :title="label"
    >
      <span class="truncate">{{ label }}</span>
    </DropdownMenuTrigger>
    <DropdownMenuPortal>
      <DropdownMenuContent side="bottom" :align="align" :side-offset="4" class="surface-popover titlebar-menu">
        <DropdownMenuFilter
          v-if="searchPlaceholder"
          v-model="query"
          :placeholder="searchPlaceholder"
          :auto-focus="true"
          class="titlebar-menu-search"
        />
        <template v-for="(section, index) in sections" :key="index">
          <DropdownMenuSeparator v-if="section.kind === 'separator'" class="menu-separator" />
          <div v-else>
            <p v-if="section.kind === 'group'" class="group-header">{{ section.label }}</p>
            <DropdownMenuItem
              v-for="item in rows(section)"
              :key="item.id"
              class="menu-item select-none"
              :class="{ 'is-pinned': item.pinned }"
              :disabled="item.disabled"
              :title="item.title"
              :data-testid="`menu-item-${item.id}`"
              @select="item.run()"
            >
              <span class="menu-item-label">{{ item.label }}</span>
              <CheckIcon v-if="item.checked" class="icon-xxs menu-check" aria-hidden="true" />
              <span v-if="item.hint" class="menu-item-hint">{{ item.hint }}</span>
            </DropdownMenuItem>
          </div>
        </template>
        <p v-if="nothingMatched" class="menu-note">No matches</p>
      </DropdownMenuContent>
    </DropdownMenuPortal>
  </DropdownMenuRoot>
</template>
