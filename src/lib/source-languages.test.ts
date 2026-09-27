import { describe, expect, it } from "vitest";
import { languageForName } from "./source-highlighter";
import {
  LANGUAGE_BY_EXTENSION,
  PLAIN_TEXT,
  SELECTABLE_LANGUAGES,
  detectedLanguageName,
  languageExtensions,
  languageLabel,
} from "./source-languages";

describe("source languages", () => {
  it("detects the grammar a path's extension asks for, whatever the case", () => {
    expect(detectedLanguageName("src/app.TS")).toBe("typescript");
    expect(detectedLanguageName("config/app.yml")).toBe("yaml");
    expect(detectedLanguageName("src/main.rs")).toBe("rust");
    // An extension that names no grammar is a plain file, not a guess at one.
    expect(detectedLanguageName("notes.txt")).toBeUndefined();
    expect(detectedLanguageName("")).toBeUndefined();
  });

  it("names every grammar it detects, and every grammar the toolbar offers", () => {
    // The table is split across two modules, so this is what keeps a renamed grammar from turning
    // into a silent "no highlighting" for a file that used to have it.
    for (const name of new Set(LANGUAGE_BY_EXTENSION.values())) expect(languageForName(name)).toBeDefined();
    for (const language of SELECTABLE_LANGUAGES) expect(languageForName(language.name)).toBeDefined();
    // Plain text is the one name on offer that is deliberately not a grammar.
    expect(SELECTABLE_LANGUAGES.map((language) => language.name)).not.toContain(PLAIN_TEXT);
  });

  it("labels a grammar by its written name, and title-cases the ones it has no name for", () => {
    expect(languageLabel("csharp")).toBe("C#");
    expect(languageLabel("shellscript")).toBe("Shell");
    expect(languageLabel("common-lisp")).toBe("Common Lisp");
    expect(languageLabel("mermaid")).toBe("Mermaid");
  });

  it("lists the extensions that already ask for a grammar, so the search finds it by suffix", () => {
    expect(languageExtensions("yaml")).toEqual(["yaml", "yml"]);
    expect(languageExtensions("dart")).toEqual(["dart"]);
    expect(languageExtensions("markdown")).toEqual(["md", "mdown", "markdown"]);
    expect(languageExtensions(PLAIN_TEXT)).toEqual([]);
  });
});
