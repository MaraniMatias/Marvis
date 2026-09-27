<script setup lang="ts">
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";

const sidebarItems = ["Files", "Search", "Source Control", "Sessions"];
const inspectorItems = ["Document", "Changes", "Review"];
</script>

<template>
  <div class="flex h-full flex-col overflow-hidden">
    <header
      class="flex h-10 shrink-0 items-center gap-3 border-b bg-muted px-3 select-none"
      style="-webkit-app-region: drag"
    >
      <div class="flex gap-2" style="-webkit-app-region: no-drag">
        <span class="size-3 rounded-full bg-red-500/80" />
        <span class="size-3 rounded-full bg-amber-500/80" />
        <span class="size-3 rounded-full bg-green-500/80" />
      </div>
      <span class="text-xs font-medium">Marvis</span>
    </header>

    <ResizablePanelGroup direction="horizontal" class="min-h-0 flex-1">
      <ResizablePanel :default-size="18" :min-size="10" class="bg-muted">
        <nav class="flex flex-col gap-0.5 p-2">
          <span
            v-for="item in sidebarItems"
            :key="item"
            class="rounded-sm px-2 py-1.5 text-xs text-muted-foreground first:bg-accent first:text-accent-foreground"
          >
            {{ item }}
          </span>
        </nav>
      </ResizablePanel>

      <ResizableHandle with-handle />

      <ResizablePanel :default-size="64" :min-size="30">
        <ResizablePanelGroup direction="vertical">
          <ResizablePanel :default-size="70">
            <div class="p-4 text-sm">Editor</div>
          </ResizablePanel>
          <ResizableHandle />
          <ResizablePanel :default-size="30" class="bg-muted">
            <div class="p-4 text-sm text-muted-foreground">Terminal</div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </ResizablePanel>

      <ResizableHandle with-handle />

      <ResizablePanel :default-size="18" :min-size="10" class="bg-muted">
        <div class="flex flex-col gap-0.5 p-2">
          <span
            v-for="item in inspectorItems"
            :key="item"
            class="rounded-sm px-2 py-1.5 text-xs text-muted-foreground first:bg-accent first:text-accent-foreground"
          >
            {{ item }}
          </span>
        </div>
      </ResizablePanel>
    </ResizablePanelGroup>
  </div>
</template>
