<script setup lang="ts">
import { computed } from "vue";
import {
  ZED_DIRECTORY_ICON,
  ZED_ICON_MARKUP,
  ZED_ICON_VIEWBOX,
  ZED_ICON_VIEWBOX_OVERRIDE,
  ZED_NAMED_DIRECTORY,
  zedIconFor,
} from "../lib/zed-icons";
import type { FileEntryKind } from "../domain/files";

const props = withDefaults(
  defineProps<{
    name: string;
    kind: FileEntryKind;
    /** Whether the row draws itself quieter, and how far: a dotfile, or a Git-ignored file. */
    prominence?: "normal" | "hidden" | "ignored";
  }>(),
  { prominence: "normal" },
);

const icon = computed(() =>
  props.kind === "directory"
    ? (ZED_NAMED_DIRECTORY[props.name] ?? ZED_DIRECTORY_ICON.collapsed)
    : zedIconFor(props.name),
);

const viewBox = computed(() => ZED_ICON_VIEWBOX_OVERRIDE[icon.value] ?? ZED_ICON_VIEWBOX);
</script>

<template>
  <!-- The upstream markup is inlined rather than loaded as an image: nothing is fetched, and
       the content security policy's `img-src` never enters into it. The strokes carry their own
       per-type colour, so quieting a row is opacity on this element. -->
  <svg
    class="file-icon shrink-0"
    :class="`file-icon-${prominence}`"
    :viewBox="viewBox"
    width="14"
    height="14"
    aria-hidden="true"
    v-html="ZED_ICON_MARKUP[icon]"
  />
</template>

<style scoped>
.file-icon {
  display: block;
  overflow: visible;
}

/* Three steps of prominence, all still readable: a dotfile steps down, and a file Git ignores
   steps down again. The numbers are the whole ramp, so they are the only thing to tune. */
.file-icon-hidden {
  opacity: 0.72;
}
.file-icon-ignored {
  opacity: 0.48;
}
</style>
