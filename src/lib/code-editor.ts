import { cpp } from "@codemirror/lang-cpp";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { java } from "@codemirror/lang-java";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { sql } from "@codemirror/lang-sql";
import { xml } from "@codemirror/lang-xml";
import { yaml } from "@codemirror/lang-yaml";
import {
  StreamLanguage,
  syntaxHighlighting,
  HighlightStyle,
  foldGutter,
  foldKeymap,
  indentOnInput,
  bracketMatching,
  type StringStream,
} from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { history, defaultKeymap, historyKeymap } from "@codemirror/commands";
import {
  EditorView,
  crosshairCursor,
  drawSelection,
  dropCursor,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { csharp, kotlin, objectiveC } from "@codemirror/legacy-modes/mode/clike";
import { diff } from "@codemirror/legacy-modes/mode/diff";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { go } from "@codemirror/legacy-modes/mode/go";
import { gql } from "@codemirror/legacy-modes/mode/sql";
import { http } from "@codemirror/legacy-modes/mode/http";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { lua } from "@codemirror/legacy-modes/mode/lua";
import { perl } from "@codemirror/legacy-modes/mode/perl";
import { powerShell } from "@codemirror/legacy-modes/mode/powershell";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { ruby } from "@codemirror/legacy-modes/mode/ruby";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { swift } from "@codemirror/legacy-modes/mode/swift";
import { toml } from "@codemirror/legacy-modes/mode/toml";

export interface CodeEditorOptions {
  parent: HTMLElement;
  content: string;
  language: string | null;
  readingPosition: { top: number; left: number };
  onChange: (content: string) => void;
  onScroll: (position: { top: number; left: number }) => void;
}

/**
 * `defaultHighlightStyle` is a light palette — dark red keywords, mid-blue strings — and it was
 * being laid over Marvis' `#17191f` editor background, where the darker half of it is barely
 * readable. These are the colors the `github-dark-default` Shiki theme already uses to render the
 * same file read-only, so a file stops changing color when it becomes editable.
 *
 * This has to name every tag a readable color is wanted for, and it is now the only palette in
 * play: nothing injects `defaultHighlightStyle` as a fallback, so a tag missing here is painted in
 * the editor's own text color rather than in a light one.
 */
export const marvisHighlightStyle = HighlightStyle.define([
  { tag: tags.comment, color: "#8b949e" },
  { tag: tags.keyword, color: "#ff7b72" },
  { tag: tags.controlKeyword, color: "#ff7b72" },
  { tag: tags.moduleKeyword, color: "#ff7b72" },
  { tag: tags.definitionKeyword, color: "#ff7b72" },
  { tag: tags.operatorKeyword, color: "#ff7b72" },
  { tag: tags.string, color: "#a5d6ff" },
  { tag: tags.special(tags.string), color: "#a5d6ff" },
  { tag: tags.escape, color: "#a5d6ff" },
  { tag: tags.regexp, color: "#a5d6ff" },
  { tag: tags.atom, color: "#79c0ff" },
  { tag: tags.bool, color: "#79c0ff" },
  { tag: tags.literal, color: "#79c0ff" },
  { tag: tags.null, color: "#79c0ff" },
  { tag: tags.number, color: "#79c0ff" },
  { tag: tags.contentSeparator, color: "#79c0ff" },
  { tag: tags.meta, color: "#79c0ff" },
  { tag: tags.propertyName, color: "#79c0ff" },
  { tag: tags.constant(tags.variableName), color: "#79c0ff" },
  { tag: tags.special(tags.variableName), color: "#79c0ff" },
  { tag: tags.function(tags.variableName), color: "#d2a8ff" },
  { tag: tags.macroName, color: "#d2a8ff" },
  { tag: tags.definition(tags.variableName), color: "#ffa657" },
  { tag: tags.definition(tags.propertyName), color: "#79c0ff" },
  { tag: tags.inserted, color: "#7ee787" },
  { tag: tags.deleted, color: "#ffa198" },
  { tag: tags.labelName, color: "#ffa657" },
  { tag: tags.local(tags.variableName), color: "#ffa657" },
  { tag: tags.variableName, color: "#ffa657" },
  { tag: tags.namespace, color: "#ffa657" },
  { tag: tags.url, color: "#a5d6ff" },
  { tag: tags.className, color: "#7ee787" },
  { tag: tags.tagName, color: "#7ee787" },
  { tag: tags.attributeName, color: "#7ee787" },
  { tag: tags.typeName, color: "#7ee787" },
  { tag: tags.heading, color: "#79c0ff", fontWeight: "bold" },
  { tag: tags.quote, color: "#7ee787" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strong, fontWeight: "bold" },
  { tag: tags.strikethrough, textDecoration: "line-through" },
  { tag: tags.link, color: "#a5d6ff" },
  { tag: tags.invalid, color: "#ffa198" },
]);

/**
 * A gitignore is a list of bare globs, which the `properties` mode the other INI files use reads as
 * one very long key — and a leading `!` negation as a comment, which is backwards. Only two
 * characters carry meaning: `#` opens a comment and must sit at the start of the line, and `!` at
 * that same spot re-includes a pattern. The glob metacharacters are marked so a `*.log` reads as
 * glob syntax rather than prose; everything else is a path, and stays in the base color.
 */
const gitignoreLanguage = StreamLanguage.define({
  name: "gitignore",
  token(stream: StringStream) {
    if (stream.sol()) {
      if (stream.eat("#")) {
        stream.skipToEnd();
        return "comment";
      }
      if (stream.eat("!")) return "keyword";
    }
    if (stream.eat(/[[\]*?]/)) return "operator";
    stream.next();
    return null;
  },
});

/**
 * What the editor needs for itself: a height it does not grow past, and a scroller that scrolls.
 *
 * The two colors CodeMirror hardcodes for a light page — a lavender selection, a black caret —
 * are not here: `EditorView.theme` refuses the selectors its own base theme uses (`&dark`), and a
 * shorter selector loses to that theme anyway. They live in `DocumentPane`'s stylesheet, which can
 * out-specify it.
 */
const marvisTheme = EditorView.theme({
  "&": { height: "100%" },
  ".cm-scroller": { overflow: "auto" },
});

/**
 * The editor is imported by DocumentPane only after a Code view is opened. Shiki remains the
 * renderer for the read-only view, while this small CM6 setup owns editing, history, gutters, and
 * the language selected by the existing toolbar.
 */
export function createCodeEditor(options: CodeEditorOptions): EditorView {
  const view = new EditorView({
    state: EditorState.create({
      doc: options.content,
      extensions: [
        // This is `basicSetup` written out, minus the search panel, the linter and the completion
        // popup. Those three are the only reason the `codemirror` package was a dependency, and
        // none of them is a thing this editor does: it draws source and takes typed text. Two
        // consequences worth keeping in mind before adding one back — the light palette
        // `basicSetup` also injects as a fallback does not come with it (so every readable color is
        // named in `marvisHighlightStyle`), and a package that is not in `package.json` cannot be
        // imported here at all.
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightSpecialChars(),
        drawSelection(),
        dropCursor(),
        rectangularSelection(),
        crosshairCursor(),
        highlightActiveLine(),
        foldGutter(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        history(),
        EditorState.allowMultipleSelections.of(true),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...foldKeymap]),
        // CodeMirror assumes a light page unless told otherwise, and the two things it paints
        // itself say so: a lavender selection and a black caret, both unreadable here. The
        // selection is drawn on its own layer, which the shell's `::selection` cannot reach.
        EditorView.darkTheme.of(true),
        marvisTheme,
        languageExtension(options.language),
        syntaxHighlighting(marvisHighlightStyle),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) options.onChange(update.state.doc.toString());
        }),
        EditorView.domEventHandlers({
          scroll: (_event, editor) => {
            options.onScroll({ top: editor.scrollDOM.scrollTop, left: editor.scrollDOM.scrollLeft });
            return false;
          },
        }),
      ],
    }),
    parent: options.parent,
  });
  view.scrollDOM.scrollTop = options.readingPosition.top;
  view.scrollDOM.scrollLeft = options.readingPosition.left;
  return view;
}

function languageExtension(language: string | null): Extension {
  switch (language) {
    case "c":
    case "cpp":
      return cpp();
    case "csharp":
      return StreamLanguage.define(csharp);
    case "css":
    case "scss":
    case "less":
      return css();
    case "docker":
      return StreamLanguage.define(dockerFile);
    case "diff":
      return StreamLanguage.define(diff);
    case "go":
      return StreamLanguage.define(go);
    case "graphql":
      return StreamLanguage.define(gql);
    case "html":
    case "vue":
      return html();
    case "http":
      return StreamLanguage.define(http);
    // A dotenv is a properties file, and the legacy modes have no tokenizer of its own for either.
    case "ini":
    case "dotenv":
      return StreamLanguage.define(properties);
    case "gitignore":
      return gitignoreLanguage;
    case "java":
      return java();
    case "javascript":
      return javascript();
    case "typescript":
    case "tsx":
      return javascript({ typescript: true, jsx: language === "tsx" });
    case "json":
    case "jsonc":
    case "json5":
      return json();
    case "kotlin":
      return StreamLanguage.define(kotlin);
    case "lua":
      return StreamLanguage.define(lua);
    case "markdown":
      return markdown();
    case "objective-c":
      return StreamLanguage.define(objectiveC);
    case "powershell":
      return StreamLanguage.define(powerShell);
    case "nginx":
      return StreamLanguage.define(nginx);
    case "perl":
      return StreamLanguage.define(perl);
    case "python":
      return python();
    case "ruby":
      return StreamLanguage.define(ruby);
    case "rust":
      return rust();
    case "shellscript":
      return StreamLanguage.define(shell);
    case "sql":
      return sql();
    case "swift":
      return StreamLanguage.define(swift);
    case "toml":
      return StreamLanguage.define(toml);
    case "xml":
      return xml();
    case "yaml":
      return yaml();
    case "plain-text":
    case "plaintext":
    case null:
      return [];
    default:
      // Unsupported Shiki grammars still get a real editor; the known names above retain the
      // extension/override selection while this fallback avoids pretending plain text is invalid.
      return [];
  }
}
