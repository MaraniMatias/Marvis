import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const stylesheet = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "..", "marvis.css"), "utf8");
const sidebarStyles = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "components", "Sidebar.vue"),
  "utf8",
);
const fileDiffTemplate = readFileSync(
  join(dirname(fileURLToPath(import.meta.url)), "..", "components", "FileDiff.vue"),
  "utf8",
);
const chromeStart = stylesheet.indexOf("/* UI chrome");

function block(selector: string, css = stylesheet): string {
  const selectorStart = css.indexOf(selector, css === stylesheet ? chromeStart : 0);
  const open = css.indexOf("{", selectorStart);
  return css.slice(open + 1, css.indexOf("}", open));
}

function declarations(css: string): Record<string, string> {
  return Object.fromEntries(
    [...css.matchAll(/(--[\w-]+):\s*([^;]+);/g)].map(([, name, value]) => [name!, value!.trim()]),
  );
}

const darkTokens = declarations(block(":root {"));
const themes = {
  dark: darkTokens,
  light: { ...darkTokens, ...declarations(block(':root[data-theme="light"]')) },
};
const referenceSwatches = {
  dark: {
    "--marvis-bg-0": "#16181d",
    "--marvis-bg-1": "#1c1f26",
    "--marvis-bg-2": "#242830",
    "--marvis-border": "#2c313b",
    "--marvis-border-strong": "#3a404c",
    "--marvis-text": "#d7dae0",
    "--marvis-text-muted": "#8b909c",
    "--marvis-text-disabled": "#565b67",
    "--marvis-el": "#242830",
    "--marvis-el-hover": "#2d323c",
    "--marvis-el-active": "#363c48",
    "--marvis-el-selected": "#2c3b57",
    "--marvis-accent": "#5b97ff",
    "--marvis-accent-fg": "#0d1626",
    "--marvis-accent-tint": "#1f2f4d",
    "--marvis-danger": "#f07178",
    "--marvis-success": "#5fc98a",
    "--marvis-warning": "#e3b341",
    "--marvis-focus": "#5b97ff",
  },
  light: {
    "--marvis-bg-0": "#f6f6f7",
    "--marvis-bg-1": "#ffffff",
    "--marvis-bg-2": "#ffffff",
    "--marvis-border": "#e1e2e5",
    "--marvis-border-strong": "#c9cbd0",
    "--marvis-text": "#1c1e23",
    "--marvis-text-muted": "#6b7080",
    "--marvis-text-disabled": "#a9adb8",
    "--marvis-el": "#eef0f2",
    "--marvis-el-hover": "#e4e6ea",
    "--marvis-el-active": "#d9dce2",
    "--marvis-el-selected": "#d6e4ff",
    "--marvis-accent": "#2f6fed",
    "--marvis-accent-fg": "#ffffff",
    "--marvis-accent-tint": "#e3ecff",
    "--marvis-danger": "#d63b3b",
    "--marvis-success": "#1f9d55",
    "--marvis-warning": "#c98a0a",
    "--marvis-focus": "#2f6fed",
  },
};
const referenceContentSwatches = {
  dark: {
    "--marvis-content-bg-0": "#282c33",
    "--marvis-content-bg-1": "#2f343e",
    "--marvis-content-bg-2": "#363c46",
    "--marvis-content-border": "#464b57",
    "--marvis-content-text": "#dce0e5",
    "--marvis-content-text-muted": "#a9afbc",
    "--marvis-content-text-faint": "#767d8d",
    "--marvis-content-accent": "#74ade8",
    "--marvis-content-added": "#a1c181",
    "--marvis-content-removed": "#d97f84",
    "--marvis-selection": "#74ade83d",
  },
  light: {
    "--marvis-content-bg-0": "#fafafa",
    "--marvis-content-bg-1": "#ebebec",
    "--marvis-content-bg-2": "#dfdfe0",
    "--marvis-content-border": "#c9c9ca",
    "--marvis-content-text": "#242529",
    "--marvis-content-text-muted": "#58585a",
    "--marvis-content-text-faint": "#8a8b8f",
    "--marvis-content-accent": "#455fd0",
    "--marvis-content-added": "#3f7a35",
    "--marvis-content-removed": "#b8453a",
    "--marvis-selection": "#455fd03d",
  },
};

function token(theme: Record<string, string>, name: string): string {
  const value = theme[name];
  if (!value) throw new Error(`Missing ${name}`);
  const ref = /^var\((--[\w-]+)\)$/.exec(value);
  return ref ? token(theme, ref[1]!) : value;
}

function channels(hex: string): number[] {
  const value = hex.replace(/^#/, "");
  return [0, 2, 4].map((offset) => Number.parseInt(value.slice(offset, offset + 2), 16));
}

function hex(rgb: number[]): string {
  return `#${rgb.map((value) => Math.round(value).toString(16).padStart(2, "0")).join("")}`;
}

function mix(first: string, second: string, firstShare: number): string {
  const a = channels(first);
  const b = channels(second);
  return hex(a.map((value, index) => value * firstShare + b[index]! * (1 - firstShare)));
}

function brightness(color: string, factor: number): string {
  return hex(channels(color).map((value) => Math.min(255, value * factor)));
}

function composite(foreground: string, background: string, opacity: number): string {
  const fg = channels(foreground);
  const bg = channels(background);
  return hex(fg.map((value, index) => value * opacity + bg[index]! * (1 - opacity)));
}

function luminance(color: string): number {
  const [r, g, b] = channels(color).map((value) => {
    const channel = value / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

function contrast(foreground: string, background: string): number {
  const a = luminance(foreground);
  const b = luminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

function expectReadable(foreground: string, background: string) {
  expect(contrast(foreground, background), `${foreground} on ${background}`).toBeGreaterThanOrEqual(4.5);
}

describe("UI foreground tokens", () => {
  it("preserves the exact Raymond palette swatches", () => {
    for (const [mode, expected] of Object.entries(referenceSwatches)) {
      for (const [name, color] of Object.entries(expected)) {
        expect(token(themes[mode as keyof typeof themes], name), `${mode} ${name}`).toBe(color);
      }
    }
  });

  it("keeps the editor and diff content palettes unchanged", () => {
    for (const [mode, expected] of Object.entries(referenceContentSwatches)) {
      for (const [name, color] of Object.entries(expected)) {
        expect(token(themes[mode as keyof typeof themes], name), `${mode} ${name}`).toBe(color);
      }
    }
  });

  it("keeps pinned agent labels readable on hovered and selected rows", () => {
    const rule = block(".workdir-item:hover .workdir-meta-pinned .agent-chip,", sidebarStyles);
    expect(rule).toContain("color: var(--marvis-text);");
    for (const theme of Object.values(themes)) {
      const foreground = token(theme, "--marvis-text");
      expectReadable(foreground, token(theme, "--marvis-el-hover"));
      expectReadable(foreground, token(theme, "--marvis-el-selected"));
    }
  });

  it("uses a readable content foreground for virtualized diff errors", () => {
    const message = fileDiffTemplate.indexOf('row.error || "Loading diff page…"');
    const element = fileDiffTemplate.slice(
      fileDiffTemplate.lastIndexOf("<span", message),
      fileDiffTemplate.indexOf("</span>", message),
    );
    expect(element).toContain("--marvis-content-text-muted");
    for (const theme of Object.values(themes)) {
      expectReadable(token(theme, "--marvis-content-text-muted"), token(theme, "--marvis-content-bg-0"));
    }
  });

  it("keep small semantic text readable across themes and interactive surface states", () => {
    for (const [mode, theme] of Object.entries(themes)) {
      const get = (name: string) => token(theme, name);
      const surfaces = [
        "--marvis-bg-0",
        "--marvis-bg-1",
        "--marvis-bg-2",
        "--marvis-el",
        "--marvis-el-hover",
        "--marvis-el-active",
        "--marvis-el-selected",
      ];

      for (const [role, alphas] of [
        ["danger", [0.12, 0.2]],
        ["success", []],
        ["warning", []],
      ] as const) {
        const foreground = get(`--marvis-${role}-fg`);
        for (const surface of surfaces) {
          const background = get(surface);
          expectReadable(foreground, background);
          for (const alpha of alphas) {
            expectReadable(foreground, composite(get(`--marvis-${role}`), background, alpha));
          }
        }
      }

      const tint = get("--marvis-accent-tint");
      const tintForeground = get("--marvis-accent-tint-fg");
      expectReadable(tintForeground, tint);
      expectReadable(brightness(tintForeground, 0.96), brightness(tint, 0.96));
      expectReadable(tintForeground, mix(tint, get("--marvis-accent"), 0.82));

      const accent = get("--marvis-accent");
      const accentForeground = get("--marvis-accent-fg");
      expectReadable(accentForeground, accent);
      if (mode === "light") {
        expectReadable(accentForeground, mix(accent, get("--marvis-text"), 0.88));
        expectReadable(accentForeground, mix(accent, get("--marvis-text"), 0.84));
      } else {
        for (const factor of [1.08, 0.94]) {
          expectReadable(brightness(accentForeground, factor), brightness(accent, factor));
        }
      }

      const placeholder = get("--marvis-placeholder-fg");
      for (const surface of ["--marvis-bg-0", "--marvis-bg-1", "--marvis-bg-2"]) {
        expectReadable(placeholder, get(surface));
      }
    }
  });
});
