import type { Component } from "vue";
import {
  Folder as FolderIcon,
  FolderGit2 as FolderGit2Icon,
  FolderX as FolderXIcon,
  GitBranch as GitBranchIcon,
  House as HouseIcon,
  SquareTerminal as SquareTerminalIcon,
} from "@lucide/vue";
import type { WorkdirIconKind } from "../domain/workspace";

/**
 * The glyph each role of a checkout wears, and nothing else: the shape of an icon is a drawing
 * decision, so the map lives here with the rest of the drawing and the rule that picks a key out
 * of it lives in the domain, where `workdirIconKind` is written.
 */
export const WORKDIR_ICONS: Record<WorkdirIconKind, Component> = {
  /** A repo root: a folder that is also a Git directory. */
  git: FolderGit2Icon,
  /** A worktree is a branch checked out in a directory of its own, not a fork of anything. */
  worktree: GitBranchIcon,
  folder: FolderIcon,
  home: HouseIcon,
  /** The directory is gone: a folder with a cross, painted in the disabled colour by the row. */
  missing: FolderXIcon,
  terminal: SquareTerminalIcon,
};
