<script setup lang="ts">
import { computed } from "vue";
import InspectorPane from "./components/InspectorPane.vue";
import GitStatusBar from "./components/GitStatusBar.vue";
import SessionPane from "./components/SessionPane.vue";
import Sidebar from "./components/Sidebar.vue";
import { useWorkspaceState } from "./presentation/workspace";

const {
  workspace,
  activeCheckout,
  isOpening,
  error,
  chooseFolder,
  selectCheckout,
  selectSession,
  updateWorkspace,
  promptForDefaultBranchIfNeeded,
} = useWorkspaceState();
const activeRepo = computed(
  () =>
    workspace.value.repos.find((repo) => repo.checkouts.some((checkout) => checkout.id === activeCheckout.value?.id)) ??
    null,
);

function handleDefaultBranchUnknown() {
  void promptForDefaultBranchIfNeeded(true);
}
</script>

<template>
  <div class="flex h-full min-w-[900px] flex-col bg-[#111318] text-zinc-100">
    <div class="flex min-h-0 flex-1">
      <Sidebar
        :repos="workspace.repos"
        :active-checkout-id="workspace.activeCheckoutId"
        :active-session-id="workspace.activeSessionId"
        :is-opening="isOpening"
        @open-folder="chooseFolder"
        @select-checkout="selectCheckout"
        @select-session="selectSession"
      />
      <SessionPane
        :checkout="activeCheckout"
        :active-session-id="workspace.activeSessionId"
        :is-opening="isOpening"
        @open-folder="chooseFolder"
        @select-session="selectSession"
        @workspace-updated="updateWorkspace"
      />
      <InspectorPane
        :checkout="activeCheckout"
        :repo="activeRepo"
        @default-branch-unknown="handleDefaultBranchUnknown"
      />
    </div>
    <GitStatusBar :checkout="activeCheckout" :repo="activeRepo" @default-branch-unknown="handleDefaultBranchUnknown" />
    <div
      v-if="error"
      role="alert"
      class="absolute bottom-12 left-1/2 max-w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 rounded-lg border border-red-400/20 bg-[#242126] px-4 py-3 text-sm text-red-200 shadow-xl"
    >
      {{ error }}
    </div>
  </div>
</template>
