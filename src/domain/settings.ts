/**
 * The preferences the Settings dialog owns, in the shape they are written to `~/.marvis/config.yml`.
 *
 * They are preferences rather than layout: a person sets them once and forgets they made them, so
 * they live in a file they can open and edit, not in the database that remembers which checkouts
 * exist and how wide a pane is. Everything a window remembers instead — the mode, the three widths —
 * stays in `src/domain/ui-state.ts`.
 */
import type { Zoom } from "./zoom";
import { ZOOM_STEPS, normalizeZoom } from "./zoom";

export const TERMINAL_SCROLLBAR_MODES = ["hidden", "auto", "always"] as const;

export type TerminalScrollbarMode = (typeof TERMINAL_SCROLLBAR_MODES)[number];

/**
 * Which palette the window is drawn in. `system` is what the operating system asks for; the other
 * two say so out loud, because a window that follows the system cannot be argued with.
 */
export const THEME_PREFERENCES = ["system", "light", "dark"] as const;

export type ThemePreference = (typeof THEME_PREFERENCES)[number];

/** The shapes xterm.js draws, named the way it names them: a cell, a line, or a line under the text. */
export const TERMINAL_CURSOR_STYLES = ["block", "bar", "underline"] as const;

export type TerminalCursorStyle = (typeof TERMINAL_CURSOR_STYLES)[number];

export interface UiSettings {
  /** CSS pixels. The whole interface's type scale is a ratio of this one number. */
  fontSize: number;
  zoom: Zoom;
  theme: ThemePreference;
}

export interface TerminalSettings {
  fontSize: number;
  ligatures: boolean;
  cursorBlink: boolean;
  /**
   * The shape of the cursor. A block by default: it is filled with the theme's `--marvis-cursor`
   * and the glyph under it is the surface, so a caret is the one thing on the page drawn the other
   * way round, and a bar is the quieter answer for a person who would rather not have a cell of
   * their screen filled in.
   */
  cursorStyle: TerminalCursorStyle;
  scrollbar: TerminalScrollbarMode;
  /**
   * Whether moving a terminal to another worktree also makes its shell change directory.
   *
   * Off by default because the two answers are both defensible and only the person using the
   * terminal knows which one they meant: a shell sitting at a prompt is happy to `cd`, and a shell
   * with a build running is not asked to.
   */
  changeDirectoryOnMove: boolean;
}

export interface IndentationSettings {
  useSpaces: boolean;
  /** Columns one level of indent is, spaces or tab stops. */
  size: number;
}

export interface EditorSettings {
  fontSize: number;
  ligatures: boolean;
  /** The same answer the terminal's own `cursorBlink` is, for the same reason: the caret is a block and a block that blinks is a distraction. */
  cursorBlink: boolean;
  indentation: IndentationSettings;
}

export interface AppSettings {
  ui: UiSettings;
  terminal: TerminalSettings;
  editor: EditorSettings;
}

/**
 * The bounds are the ones `src-tauri/src/config.rs` clamps to. They are written twice because a
 * file a person typed into arrives through the same struct as a value the dialog drew, and a
 * number outside the range is not a size either side can draw.
 */
export const UI_FONT_SIZE_LIMITS = { min: 11, max: 20 };
export const TERMINAL_FONT_SIZE_LIMITS = { min: 9, max: 32 };
export const EDITOR_FONT_SIZE_LIMITS = { min: 9, max: 32 };
export const INDENTATION_SIZE_LIMITS = { min: 1, max: 8 };

export const DEFAULT_SETTINGS: AppSettings = {
  ui: { fontSize: 14, zoom: 1, theme: "system" },
  terminal: {
    fontSize: 16,
    ligatures: true,
    cursorBlink: true,
    cursorStyle: "block",
    scrollbar: "hidden",
    changeDirectoryOnMove: false,
  },
  editor: {
    fontSize: 13,
    ligatures: true,
    cursorBlink: true,
    indentation: { useSpaces: true, size: 2 },
  },
};

export function isTerminalScrollbarMode(value: unknown): value is TerminalScrollbarMode {
  return TERMINAL_SCROLLBAR_MODES.some((mode) => mode === value);
}

export function isTerminalCursorStyle(value: unknown): value is TerminalCursorStyle {
  return TERMINAL_CURSOR_STYLES.some((style) => style === value);
}

export function isThemePreference(value: unknown): value is ThemePreference {
  return THEME_PREFERENCES.some((preference) => preference === value);
}

function boundedNumber(value: unknown, limits: { min: number; max: number }, fallback: number) {
  const number = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.min(limits.max, Math.max(limits.min, number));
}

function boundedInteger(value: unknown, limits: { min: number; max: number }, fallback: number) {
  const number = typeof value === "number" ? value : Number(value);
  if (!Number.isInteger(number)) return fallback;
  return Math.min(limits.max, Math.max(limits.min, number));
}

function flag(value: unknown, fallback: boolean) {
  return typeof value === "boolean" ? value : fallback;
}

/**
 * Brings whatever came off the disk or out of the dialog into the shape the rest of the app can
 * read without asking again. There is no version to refuse here: the file has no ladder to climb,
 * so an unknown key is a key this build ignores rather than a reason to throw the file away.
 */
export function normalizeSettings(value: unknown): AppSettings {
  const raw = (value ?? {}) as Partial<AppSettings>;
  const ui = (raw.ui ?? {}) as Partial<UiSettings>;
  const terminal = (raw.terminal ?? {}) as Partial<TerminalSettings>;
  const editor = (raw.editor ?? {}) as Partial<EditorSettings>;
  const indentation = (editor.indentation ?? {}) as Partial<IndentationSettings>;
  return {
    ui: {
      fontSize: boundedNumber(ui.fontSize, UI_FONT_SIZE_LIMITS, DEFAULT_SETTINGS.ui.fontSize),
      zoom: normalizeZoom(ui.zoom),
      theme: isThemePreference(ui.theme) ? ui.theme : DEFAULT_SETTINGS.ui.theme,
    },
    terminal: {
      fontSize: boundedNumber(terminal.fontSize, TERMINAL_FONT_SIZE_LIMITS, DEFAULT_SETTINGS.terminal.fontSize),
      ligatures: flag(terminal.ligatures, DEFAULT_SETTINGS.terminal.ligatures),
      cursorBlink: flag(terminal.cursorBlink, DEFAULT_SETTINGS.terminal.cursorBlink),
      cursorStyle: isTerminalCursorStyle(terminal.cursorStyle)
        ? terminal.cursorStyle
        : DEFAULT_SETTINGS.terminal.cursorStyle,
      scrollbar: isTerminalScrollbarMode(terminal.scrollbar) ? terminal.scrollbar : DEFAULT_SETTINGS.terminal.scrollbar,
      changeDirectoryOnMove: flag(terminal.changeDirectoryOnMove, DEFAULT_SETTINGS.terminal.changeDirectoryOnMove),
    },
    editor: {
      fontSize: boundedNumber(editor.fontSize, EDITOR_FONT_SIZE_LIMITS, DEFAULT_SETTINGS.editor.fontSize),
      ligatures: flag(editor.ligatures, DEFAULT_SETTINGS.editor.ligatures),
      cursorBlink: flag(editor.cursorBlink, DEFAULT_SETTINGS.editor.cursorBlink),
      indentation: {
        useSpaces: flag(indentation.useSpaces, DEFAULT_SETTINGS.editor.indentation.useSpaces),
        size: boundedInteger(indentation.size, INDENTATION_SIZE_LIMITS, DEFAULT_SETTINGS.editor.indentation.size),
      },
    },
  };
}

/** The scale every size in the interface is drawn at, relative to the 14px the design is drawn at. */
export function uiFontScale(fontSize: number) {
  return fontSize / DEFAULT_SETTINGS.ui.fontSize;
}

/** A dotted path to one field, which is what the schema below names and the form reads and writes. */
export type SettingsPath =
  | "ui.fontSize"
  | "ui.zoom"
  | "ui.theme"
  | "terminal.fontSize"
  | "terminal.ligatures"
  | "terminal.cursorBlink"
  | "terminal.cursorStyle"
  | "terminal.scrollbar"
  | "terminal.changeDirectoryOnMove"
  | "editor.fontSize"
  | "editor.ligatures"
  | "editor.cursorBlink"
  | "editor.indentation.useSpaces"
  | "editor.indentation.size";

/** What a field reads and writes: the three kinds of value a preference holds. */
export type SettingsValue = string | number | boolean;

interface SettingsFieldBase {
  /** Names the control for a screen reader and draws the row's label. */
  label: string;
  /** One line under the label, saying what the field changes rather than repeating it. */
  description?: string;
}

export type SettingsField =
  | (SettingsFieldBase & {
      kind: "number";
      path: SettingsPath;
      limits: { min: number; max: number };
      step?: number;
      /** Drawn after the input, so "14 px" reads as what it is without a label saying "pixels". */
      unit?: string;
    })
  | (SettingsFieldBase & { kind: "toggle"; path: SettingsPath })
  | (SettingsFieldBase & {
      kind: "select";
      path: SettingsPath;
      options: { value: string; label: string }[];
      /**
       * The trigger is a set of buttons over strings, so the field says what the string means.
       * `String(valueAt(...))` is the other half of it: the model is always read back the same way.
       */
      parse: (value: string) => SettingsValue;
    });

export interface SettingsSection {
  id: string;
  title: string;
  fields: SettingsField[];
}

const IDENTIFIER = (value: string): SettingsValue => value;
const NUMBER = (value: string): SettingsValue => Number(value);
const BOOLEAN = (value: string): SettingsValue => value === "true";

/**
 * The dialog's shape, kept out of the dialog: a preference added here needs a field in the
 * schema and a line in the stylesheet, not a branch in the template.
 */
export const SETTINGS_SECTIONS: SettingsSection[] = [
  {
    id: "interface",
    title: "Interface",
    fields: [
      {
        kind: "number",
        path: "ui.fontSize",
        label: "Font size",
        description: "Scales every size the interface draws itself in, from the sidebar to the dialogs.",
        limits: UI_FONT_SIZE_LIMITS,
        unit: "px",
      },
      {
        kind: "select",
        path: "ui.theme",
        label: "Theme",
        description: "The app's light and dark palettes; code keeps its own colors.",
        options: [
          { value: "system", label: "System" },
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ],
        parse: IDENTIFIER,
      },
      {
        kind: "select",
        path: "ui.zoom",
        label: "Interface scale",
        description: "Also on ⌘+ and ⌘−, and back to 100% on ⌘0.",
        options: ZOOM_STEPS.map((step) => ({ value: String(step), label: `${Math.round(step * 100)}%` })),
        parse: NUMBER,
      },
    ],
  },
  {
    id: "terminal",
    title: "Terminal",
    fields: [
      {
        kind: "number",
        path: "terminal.fontSize",
        label: "Font size",
        limits: TERMINAL_FONT_SIZE_LIMITS,
        unit: "px",
      },
      {
        kind: "toggle",
        path: "terminal.ligatures",
        label: "Ligatures",
        description: "Joins the sequences the font draws as one glyph, such as => or !=.",
      },
      {
        kind: "select",
        path: "terminal.cursorStyle",
        label: "Cursor",
        description: "The cell it fills, a line beside it, or a line under it.",
        options: [
          { value: "block", label: "Block" },
          { value: "bar", label: "Bar" },
          { value: "underline", label: "Underline" },
        ],
        parse: IDENTIFIER,
      },
      { kind: "toggle", path: "terminal.cursorBlink", label: "Blink cursor" },
      {
        kind: "select",
        path: "terminal.scrollbar",
        label: "Scrollbar",
        options: [
          { value: "hidden", label: "Hidden" },
          { value: "auto", label: "While scrolling" },
          { value: "always", label: "Always" },
        ],
        parse: IDENTIFIER,
      },
      {
        kind: "toggle",
        path: "terminal.changeDirectoryOnMove",
        label: "Change directory when moved",
        description:
          "Moving a terminal hands the session to another worktree and leaves the process alone. Turn this on and a shell sitting at a prompt also changes directory; one with a command running is left where it is.",
      },
    ],
  },
  {
    id: "editor",
    title: "Editor",
    fields: [
      {
        kind: "number",
        path: "editor.fontSize",
        label: "Font size",
        limits: EDITOR_FONT_SIZE_LIMITS,
        unit: "px",
      },
      { kind: "toggle", path: "editor.ligatures", label: "Ligatures" },
      { kind: "toggle", path: "editor.cursorBlink", label: "Blink cursor" },
      {
        kind: "select",
        path: "editor.indentation.useSpaces",
        label: "Indent with",
        options: [
          { value: "true", label: "Spaces" },
          { value: "false", label: "Tabs" },
        ],
        parse: BOOLEAN,
      },
      {
        kind: "number",
        path: "editor.indentation.size",
        label: "Indent size",
        description: "Columns one level of indent is, whether it is spaces or a tab stop.",
        limits: INDENTATION_SIZE_LIMITS,
        unit: "cols",
      },
    ],
  },
];

/**
 * A copy of the settings, field by field.
 *
 * `structuredClone` is not an option: the settings arrive as a reactive proxy, and a proxy is not
 * something the structured clone algorithm can walk. Copying the branches by hand is also what
 * keeps the type honest — a field added to `AppSettings` has to be named here rather than copied
 * by accident and silently dropped.
 */
export function cloneSettings(settings: AppSettings): AppSettings {
  return {
    ui: { ...settings.ui },
    terminal: { ...settings.terminal },
    editor: { ...settings.editor, indentation: { ...settings.editor.indentation } },
  };
}

/** Reads one field out of the settings, as the string a select's trigger is showing. */
export function valueAt(settings: AppSettings, path: SettingsPath): SettingsValue {
  const keys = path.split(".");
  let branch = settings as unknown as Record<string, unknown>;
  for (const key of keys.slice(0, -1)) branch = branch[key] as Record<string, unknown>;
  return branch[keys.at(-1)!] as SettingsValue;
}

/**
 * Writes one field back. The tree is copied rather than edited in place, because the dialog
 * replaces its draft wholesale on Reset, and a tree that was edited in place is not the tree that
 * got replaced.
 */
export function withValue(settings: AppSettings, path: SettingsPath, value: SettingsValue): AppSettings {
  const next = cloneSettings(settings) as unknown as Record<string, unknown>;
  const keys = path.split(".");
  const leaf = keys.pop()!;
  let branch = next;
  for (const key of keys) branch = branch[key] as Record<string, unknown>;
  branch[leaf] = value;
  return next as unknown as AppSettings;
}
