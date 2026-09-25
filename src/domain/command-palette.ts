export type PaletteCommandId =
  | "open-directory"
  | "toggle-focus"
  | "toggle-sidebar"
  | "toggle-inspector"
  | "toggle-status-bar"
  | "toggle-transparency"
  | "new-worktree"
  | "new-terminal"
  | "open-file"
  | "open-changes"
  | "open-preview"
  | "open-zed"
  | "open-neovim";

export interface PaletteCommand {
  id: PaletteCommandId;
  label: string;
  enabled: boolean;
  disabledReason?: string;
}

export interface PaletteCapabilities {
  hasCheckout: boolean;
  isMissing: boolean;
  isGit: boolean;
  hasSelectedFile: boolean;
  zedAvailable: boolean;
  neovimAvailable: boolean;
}

export function getPaletteCommands(capabilities: PaletteCapabilities): PaletteCommand[] {
  const commands: PaletteCommand[] = [
    { id: "open-directory", label: "Open Directory", enabled: true },
    { id: "toggle-focus", label: "Toggle Focus Mode", enabled: true },
    { id: "toggle-sidebar", label: "Toggle Navigation Sidebar", enabled: true },
    { id: "toggle-inspector", label: "Toggle Files and Changes Inspector", enabled: true },
    { id: "toggle-status-bar", label: "Toggle Status Bar", enabled: true },
    { id: "toggle-transparency", label: "Toggle Reduce Transparency", enabled: true },
  ];
  if (!capabilities.hasCheckout || capabilities.isMissing) return commands;

  if (capabilities.isGit) {
    commands.push({ id: "new-worktree", label: "New Worktree", enabled: true });
  }
  commands.push({ id: "new-terminal", label: "New Terminal", enabled: true });
  commands.push({ id: "open-file", label: "Open File", enabled: true });
  if (capabilities.isGit) commands.push({ id: "open-changes", label: "Open Changes", enabled: true });
  if (capabilities.hasSelectedFile) commands.push({ id: "open-preview", label: "Open Preview", enabled: true });
  commands.push({
    id: "open-zed",
    label: "Open in Zed",
    enabled: capabilities.zedAvailable,
    ...(!capabilities.zedAvailable && { disabledReason: "Zed is unavailable; install it and add `zed` to PATH." }),
  });
  commands.push({
    id: "open-neovim",
    label: "Open in Neovim",
    enabled: capabilities.neovimAvailable,
    ...(!capabilities.neovimAvailable && {
      disabledReason: "Neovim is unavailable; install it and add `nvim` to PATH.",
    }),
  });
  return commands;
}
