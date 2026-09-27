import { describe, expect, it } from "vitest";
import { languageForName } from "./source-highlighter";
import {
  LANGUAGE_BY_EXTENSION,
  LANGUAGE_BY_FILENAME,
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

  it("detects the dotfiles whose leading dot is the whole name, and the tail of .env", () => {
    expect(detectedLanguageName(".gitignore")).toBe("gitignore");
    // The basename decides, so a nested dotfile reads like the root one.
    expect(detectedLanguageName("packages/app/.gitignore")).toBe("gitignore");
    expect(detectedLanguageName(".gitconfig")).toBe("ini");
    expect(detectedLanguageName(".gitattributes")).toBe("ini");
    expect(detectedLanguageName(".prettierrc")).toBe("json");
    // `.env` is a format, not a name: the tail is unbounded, and the dot keeps `.envrc` out.
    expect(detectedLanguageName(".env")).toBe("dotenv");
    expect(detectedLanguageName(".env.local")).toBe("dotenv");
    expect(detectedLanguageName(".env.production.local")).toBe("dotenv");
    expect(detectedLanguageName("apps/api/.env.test")).toBe("dotenv");
    expect(detectedLanguageName(".envrc")).toBeUndefined();
    // A dotfile with a real extension still resolves on the extension.
    expect(detectedLanguageName(".eslintrc.json")).toBe("json");
  });

  it("names every grammar it detects, and every grammar the toolbar offers", () => {
    // The table is split across two modules, so this is what keeps a renamed grammar from turning
    // into a silent "no highlighting" for a file that used to have it.
    for (const name of new Set(LANGUAGE_BY_EXTENSION.values())) expect(languageForName(name)).toBeDefined();
    for (const name of new Set(LANGUAGE_BY_FILENAME.values())) expect(languageForName(name)).toBeDefined();
    expect(languageForName("dotenv")).toBeDefined();
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
