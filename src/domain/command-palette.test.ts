import { describe, expect, it } from "vitest";
import { getPaletteCommands } from "./command-palette";

const commandsFor = (overrides: Partial<Parameters<typeof getPaletteCommands>[0]> = {}) =>
  getPaletteCommands({
    hasCheckout: true,
    isMissing: false,
    isGit: false,
    hasSelectedFile: false,
    zedAvailable: true,
    neovimAvailable: true,
    ...overrides,
  }).map((command) => command.id);

describe("command palette capabilities", () => {
  it("hides Git-only actions for plain folders and adds them for Git checkouts", () => {
    const plain = commandsFor();
    expect(plain).not.toContain("new-worktree");
    expect(plain).not.toContain("open-changes");

    const git = commandsFor({ isGit: true });
    expect(git).toContain("new-worktree");
    expect(git).toContain("open-changes");
  });

  it("filters checkout actions when no usable checkout exists and gates preview on a selected file", () => {
    expect(commandsFor({ hasCheckout: false })).toEqual(["open-directory"]);
    expect(commandsFor({ isMissing: true })).toEqual(["open-directory"]);
    expect(commandsFor()).not.toContain("open-preview");
    expect(commandsFor({ hasSelectedFile: true })).toContain("open-preview");
  });

  it("leaves unavailable editors visible but disabled with an actionable explanation", () => {
    const zed = getPaletteCommands({
      hasCheckout: true,
      isMissing: false,
      isGit: false,
      hasSelectedFile: false,
      zedAvailable: false,
      neovimAvailable: false,
    });
    expect(zed.find((command) => command.id === "open-zed")).toMatchObject({
      enabled: false,
      disabledReason: expect.stringContaining("Zed is unavailable"),
    });
    expect(zed.find((command) => command.id === "open-neovim")).toMatchObject({
      enabled: false,
      disabledReason: expect.stringContaining("Neovim is unavailable"),
    });
  });
});
