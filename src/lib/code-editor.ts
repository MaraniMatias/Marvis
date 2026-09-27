import { basicSetup } from "codemirror";
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
import { StreamLanguage, syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
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
 * The editor is imported by DocumentPane only after a Code view is opened. Shiki remains the
 * renderer for read-only source, while this small CM6 setup owns editing, history, gutters, and
 * the language selected by the existing toolbar.
 */
export function createCodeEditor(options: CodeEditorOptions): EditorView {
  const view = new EditorView({
    state: EditorState.create({
      doc: options.content,
      extensions: [
        basicSetup,
        languageExtension(options.language),
        syntaxHighlighting(defaultHighlightStyle),
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
    case "ini":
      return StreamLanguage.define(properties);
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
