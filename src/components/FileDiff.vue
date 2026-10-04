<script setup lang="ts">
/* eslint-disable vue/html-indent, vue/html-closing-bracket-newline, vue/html-self-closing */
import { Check as CheckIcon, ChevronDown as ChevronDownIcon } from "@lucide/vue";
import { DiffFile, DiffModeEnum, DiffViewWithMultiSelect, updateSelectionVisual_Unified } from "@git-diff-view/vue";
import type { DiffFileHighlighter, LineRange } from "@git-diff-view/vue";
import "@git-diff-view/vue/styles/diff-view-pure.css";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, inject, nextTick, onUnmounted, ref, shallowRef, watch } from "vue";
import type { GitFileDiff, GitDiffPageLine } from "../domain/git";
import { ALL_CHANGES_LABEL } from "../domain/main-document";
import { isIpcError } from "../domain/ipc";
import { agentAttention, sortAgentSessions } from "../domain/agent";
import { isReviewableNote, readDiffLines, reviewRangeCode } from "../domain/review";
import type { AnchorOutcome, DiffLine, ReviewNote, ReviewSide } from "../domain/review";
import type { Checkout } from "../domain/workspace";
import type { EditorSettings } from "../domain/settings";
import { DEFAULT_SETTINGS } from "../domain/settings";
import { detectedLanguageName } from "../lib/source-languages";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import type { ActiveReviewNotes } from "../presentation/review-notes";
import { REVIEW_SENDER } from "../presentation/review-notes";
import type { ReviewAnchorCheck } from "../domain/review";
import { getGitDiff } from "../lib/ipc";
import { theme } from "../presentation/theme";
import { diffRowHeight, useLargeDiff } from "./use-large-diff";
import ReviewComposer from "./ReviewComposer.vue";
import ReviewNoteList from "./ReviewNoteList.vue";
import SelectControl from "./ui/select/SelectControl.vue";

const props = withDefaults(
  defineProps<{
    checkout: Checkout;
    gitSnapshot: ActiveGitSnapshot;
    review: Pick<
      ActiveReviewNotes,
      "notes" | "addNote" | "updateNote" | "deleteNote" | "verifyAnchors" | "clearOutdated" | "resolveNote"
    >;
    /** Null is the whole change set: one diff per changed file, stacked in status order. */
    path: string | null;
    scrollTop: number;
    /** Nested in the change-set stack, where the row above already names the file. */
    embedded?: boolean;
    /** The diff reads at the editor's size, so the file it opens and the diff of it are one size. */
    editorSettings?: EditorSettings;
  }>(),
  { embedded: false, editorSettings: () => DEFAULT_SETTINGS.editor },
);
const emit = defineEmits<{
  ready: [path: string];
  scrollPositionChanged: [top: number];
}>();

const diff = shallowRef<GitFileDiff | null>(null);
const diffHunks = shallowRef<Array<{ title: string; file: DiffFile }>>([]);
const collapsedHunks = ref<number[]>([]);
const diffState = ref<"idle" | "loading" | "ready" | "error">("idle");
/**
 * The reading the file on screen is highlighted with, once it has arrived, and the diff it was read
 * for. Keyed on that diff because a path is not an identity: two checkouts hold their own
 * `src/App.vue`, and two versions of one file are two readings of it. See `loadHighlighter`.
 */
const loadedHighlighter = shallowRef<{ source: GitFileDiff; highlighter: DiffFileHighlighter }>();
/** Why the diff is not on screen. It is the panel's whole content, so it is drawn here. */
const diffError = ref("");
const hasTextHunks = computed(() => Boolean(diff.value?.patch.includes("@@") || diff.value?.totalLines));
const showNoTextHunks = computed(
  () =>
    diffState.value === "ready" &&
    !diff.value?.isBinary &&
    diff.value?.symlinkTarget === undefined &&
    !diff.value?.tooLarge &&
    !hasTextHunks.value,
);
const diffViewport = ref<HTMLElement | null>(null);
const diffScrollTop = ref(props.scrollTop);
const selectedPath = ref<string | null>(null);
/**
 * The highlighter handed to the library, and only while it is the one read for the diff on screen: a
 * reading that arrives after the user has moved on is worth nothing, and handing the library one that
 * cannot read the language it is about to be given sends the file through the wrong grammar.
 */
const diffHighlighter = computed(() =>
  loadedHighlighter.value?.source === diff.value ? loadedHighlighter.value.highlighter : undefined,
);
/** The files of the whole change set the user has opened. */
const expandedPaths = ref<string[]>([]);
const changedFiles = computed(() => props.gitSnapshot.status?.files ?? []);
const title = computed(() => props.path ?? ALL_CHANGES_LABEL);
const branch = computed(() => props.gitSnapshot.status?.branch ?? props.gitSnapshot.status?.head ?? "");
const diffFontSize = computed(() => props.editorSettings.fontSize);
/**
 * The height one diff row paints at, which the markup and the window's arithmetic both have to be
 * told separately: the stylesheet sets the row, and the composable counts rows of that height.
 */
const diffRowPx = computed(() => diffRowHeight(diffFontSize.value));
const largeDiff = useLargeDiff(
  () => props.checkout.id,
  selectedPath,
  diff,
  collapsedHunks,
  diffScrollTop,
  diffViewport,
  () => diffRowPx.value,
);
const { diffPages, loadVisiblePages, visibleLargeDiffWindow } = largeDiff;
/** A note being written. `end` is the line its `+` was on and `start` the top of its range. */
const draft = ref<{ side: ReviewSide; start: number; end: number } | null>(null);
const draftError = ref("");
/**
 * The range a small diff's note covers, once its composer has moved it.
 *
 * On a large diff the note is written in Marvis' own rows and `draft` is all of it. On a small one
 * the library draws the rows and holds the selection that opened the note, so until the composer
 * changes something the library's own band is already the truth and there is nothing to keep.
 */
const libraryDraft = ref<{ side: ReviewSide; start: number; end: number } | null>(null);
/** Each hunk's own element, which is where the library's selection band is painted. */
const hunkSections = ref<Array<HTMLElement | null>>([]);
/** Every line of the diff on screen, keyed by `${side}:${line}`, with the sign it carries. */
const diffLines = computed(() => {
  if (!diff.value?.large || diff.value.tooLarge) return readDiffLines(diff.value?.patch ?? "");
  const lines = new Map<string, DiffLine>();
  for (const page of Object.values(diffPages.value)) {
    for (const line of page.lines) {
      const anchor = rowAnchor(line);
      if (!anchor) continue;
      const mark = line.text[0];
      const entry: DiffLine = { text: line.text.slice(1), mark: mark === "+" || mark === "-" ? mark : " " };
      lines.set(`${anchor.side}:${anchor.line}`, entry);
      // A line the diff left alone is on both sides. `rowAnchor` names the one a row is clicked by,
      // and a range is walked by numbering, so a note on the old side has to reach over one.
      if (line.kind === "context") {
        if (line.oldLineNumber) lines.set(`old:${line.oldLineNumber}`, entry);
        if (line.newLineNumber) lines.set(`new:${line.newLineNumber}`, entry);
      }
    }
  }
  return lines;
});
const lineTexts = computed(() => new Map([...diffLines.value].map(([key, line]) => [key, line.text])));
const fileNotes = computed(() => props.review.notes.filter((note) => note.path === props.path));
/** Notes per line, shaped for the diff view's `extendData` slot. */
const extendData = computed(() => {
  const data: { oldFile: Record<string, { data: ReviewNote[] }>; newFile: Record<string, { data: ReviewNote[] }> } = {
    oldFile: {},
    newFile: {},
  };
  for (const note of fileNotes.value) {
    const target = note.side === "old" ? data.oldFile : data.newFile;
    const key = String(note.lineStart);
    target[key] = { data: [...(target[key]?.data ?? []), note] };
  }
  return data;
});
/** Anchor text of every note whose line the diff currently renders, or null while it does not. */
function anchorText(note: ReviewNote): string | null {
  const key = `${note.side}:${note.lineStart}`;
  return lineTexts.value.has(key) ? (lineTexts.value.get(key) ?? "") : null;
}
const anchorChecks = computed(() => {
  const checks: ReviewAnchorCheck[] = [];
  for (const note of fileNotes.value) {
    const currentCode = anchorText(note);
    if (currentCode !== null) checks.push({ id: note.id, currentCode });
  }
  return checks;
});
/** A large diff only knows the lines it has loaded, so it cannot call a line deleted. */
const wholeFileInMemory = computed(() => !diff.value?.large || Boolean(diff.value?.tooLarge));
/**
 * What the diff can prove about each note's anchor, for the notes the agent has seen.
 *
 * `outdated` is the backend's hash comparison and is authoritative; whether the line is
 * still present is what only this view can answer, because only it has the diff.
 */
const anchorOutcomes = computed(() => {
  const outcomes = new Map<string, AnchorOutcome>();
  for (const note of fileNotes.value) {
    const currentCode = anchorText(note);
    if (currentCode === null) {
      outcomes.set(note.id, wholeFileInMemory.value ? "missing" : "unknown");
      continue;
    }
    outcomes.set(note.id, note.outdated ? "changed" : "unchanged");
  }
  return outcomes;
});
/**
 * Notes the agent has seen whose line is gone from the diff.
 *
 * A deleted line has no row to hang a note on, so without this the one case where "the
 * agent acted on it" is most likely would be the one case the user could not see. Only
 * offered when the whole diff is in memory, because a large diff that has not loaded a line
 * cannot claim the line is gone.
 */
const notesWithoutLine = computed(() =>
  fileNotes.value.filter((note) => note.status === "sent" && wholeFileInMemory.value && anchorText(note) === null),
);

// The diff owns which notes go out, while the shell owns what the selected destination does with
// them. The distinction matters here because only an agent handoff can be queued or interrupted.

/** Absent when no shell is above this diff, which is the case in the diff's own tests. */
const sender = inject(REVIEW_SENDER, null);
const includeOutdated = ref(false);
/** Open while the target is working and the user has not yet chosen what to do about it. */
const busyChoiceOpen = ref(false);
const targetOpen = ref(false);
const targetQuery = ref("");
const targetIndex = ref(0);
/** A list this short is picked by looking at it; past this it earns a search of its own. */
const searchesTargets = computed(() => sender?.target === "opencode" && (sender.sessions.length ?? 0) > 5);
/** Newest first, the order the rest of the app reads sessions in. */
const targetRows = computed(() => {
  const rows = sortAgentSessions(sender?.sessions ?? []);
  const query = targetQuery.value.trim().toLowerCase();
  return query ? rows.filter((row) => row.title.toLowerCase().includes(query)) : rows;
});
const activeTargetRow = computed(() => Math.min(targetIndex.value, Math.max(targetRows.value.length - 1, 0)));

// Each opening starts on the row already chosen, so the arrow keys start where the user already
// is, and without the filter of a menu that is no longer open.
watch(targetOpen, (open) => {
  if (!open) return;
  targetQuery.value = "";
  targetIndex.value = Math.max(
    targetRows.value.findIndex((row) => row.id === sender?.targetId),
    0,
  );
});

watch(targetQuery, () => {
  targetIndex.value = 0;
});
const sendableNotes = computed(() => {
  const pending = props.review.notes.filter(isReviewableNote);
  // An outdated note points at a line that has since changed, so it is held back unless the
  // user says otherwise: that is the same rule the note's own card states.
  return includeOutdated.value ? pending : pending.filter((note) => !note.outdated);
});
const draftCount = computed(() => sendableNotes.value.filter((note) => note.status === "draft").length);
const outdatedCount = computed(() => props.review.notes.filter((note) => note.outdated).length);
const targetSession = computed(() => sender?.sessions.find((session) => session.id === sender.targetId) ?? undefined);
/** A blocked session is not working, so only a real turn makes the send a decision. */
const targetBusy = computed(() => agentAttention(targetSession.value) === "busy");
const showBusyChoice = computed(() => sender?.target === "opencode" && busyChoiceOpen.value && targetBusy.value);
const canSend = computed(() => sendableNotes.value.length > 0);

function sendNow(queue: boolean) {
  busyChoiceOpen.value = false;
  void sender?.send(
    sendableNotes.value.map((note) => note.id),
    queue,
  );
}

/** A working agent cannot be interrupted, so the button asks instead of sending into a turn. */
function requestSend() {
  if (sender?.target === "markdown") {
    sendNow(false);
    return;
  }
  if (targetBusy.value) {
    busyChoiceOpen.value = true;
    return;
  }
  sendNow(false);
}

function chooseTarget(sessionId: string) {
  sender?.selectTarget(sessionId);
  targetOpen.value = false;
}

/** The two destinations a review can leave by, so the picker is a list rather than a control. */
const reviewTargets = [
  { value: "markdown", label: "Markdown" },
  { value: "opencode", label: "OpenCode" },
];

function chooseReviewTarget(target?: string) {
  if (target === "markdown" || target === "opencode") sender?.selectReviewTarget(target);
}

function moveTargetRow(step: number) {
  const total = targetRows.value.length;
  if (total > 0) targetIndex.value = (targetIndex.value + step + total) % total;
}

function chooseActiveTarget() {
  const session = targetRows.value[activeTargetRow.value];
  if (session) chooseTarget(session.id);
}

let diffGeneration = 0;
/** The reading of a file in flight, so the one that answers is the one the diff on screen asked for. */
let highlighterRequest = 0;
/** Names each set of hunks, which is what tells two checkouts' identical windows apart. */
let diffIdentity = 0;
let mounted = true;

/**
 * Opens a draft on the line the `+` was on, which is the last line of its range.
 *
 * A range is written the way GitLab writes one: the `+` fixes where the note ends, and a `+`
 * clicked on a line above it says where it starts. So a click above an open draft moves the top
 * of the range and leaves the line the note is anchored to alone, and a click at or below it opens
 * a new note instead of stretching one backwards.
 */
function openDraft(side: ReviewSide, line: number) {
  draftError.value = "";
  const open = draft.value;
  if (open && open.side === side && line < open.end) {
    draft.value = { ...open, start: line };
    return;
  }
  draft.value = { side, start: line, end: line };
}

function cancelDraft() {
  draft.value = null;
  draftError.value = "";
}

/** A saved or cancelled note leaves no range behind, on either kind of diff. */
function closeLibraryDraft(onClose: () => void) {
  libraryDraft.value = null;
  onClose();
}

function sideName(side: number): ReviewSide {
  return side === 1 ? "old" : "new";
}

/**
 * The sign a line carries in the diff gutter, which is what tells an added line from a removed one
 * at a glance: the two are different lines that can share a number across the sides.
 */
function lineMarker(side: ReviewSide, line: number): string {
  return diffLines.value.get(`${side}:${line}`)?.mark ?? " ";
}

/**
 * The lines a note may start on, named by the sign they carry and their number.
 *
 * The walk stops at the first line that is not there, which is what keeps a range inside one hunk
 * and inside the pages a large diff has loaded: two hunks are never adjacent in numbering, so the
 * line above the top of one is never on the diff at all. A range over lines the diff removed is
 * walked on the old side, which is where their numbers live.
 */
function startCandidates(side: ReviewSide, end: number) {
  const texts = lineTexts.value;
  const candidates: number[] = [];
  for (let line = end; line >= 1 && texts.has(`${side}:${line}`); line -= 1) candidates.push(line);
  // Walked up from the line the note ends on, then listed the way the diff reads: top of the file
  // first, so the picker is scrolled to the one that matters.
  return candidates.reverse().map((line) => ({
    value: String(line),
    label: `${lineMarker(side, line)}${line}`,
  }));
}

/**
 * Repaints the library's selection band from the note being written.
 *
 * The band is the library's own state and it cannot be set from outside: the component it is drawn
 * by exposes only events, and no prop. What it does export is the function that paints the band,
 * so the range the composer chose is drawn with the same class the library's own drag uses, and
 * clearing it is the same call with no range.
 */
function paintLibrarySelection() {
  const range = libraryDraft.value;
  diffHunks.value.forEach((hunk, index) => {
    const container = hunkSections.value[index]?.querySelector<HTMLElement>(".diff-multiselect-wrapper");
    if (!container) return;
    const painted: LineRange | null = range
      ? { side: range.side, startLineNumber: range.start, endLineNumber: range.end }
      : null;
    updateSelectionVisual_Unified(container, painted, hunk.file);
  });
}

watch([draft, libraryDraft, diffHunks], () => nextTick(paintLibrarySelection), { flush: "post" });
onUnmounted(() => {
  libraryDraft.value = null;
});

async function saveNoteAt(
  side: ReviewSide,
  lineStart: number,
  lineEnd: number | null,
  content: string,
): Promise<boolean> {
  if (props.path === null) return false;
  const range = lineEnd && lineEnd > lineStart ? lineEnd : null;
  return props.review.addNote({
    path: props.path,
    side,
    lineStart,
    lineEnd: range ?? undefined,
    content,
    code: reviewRangeCode(lineTexts.value, side, lineStart, range),
  });
}

async function saveDraft(content: string, lineStart: number): Promise<boolean> {
  const target = draft.value;
  if (!target) return false;
  draftError.value = "";
  const first = Math.min(lineStart, target.end);
  if (diff.value?.large) {
    for (let line = first; line <= target.end; line += 1) {
      if (!lineTexts.value.has(`${target.side}:${line}`)) {
        draftError.value =
          "Some selected lines are unavailable in the loaded diff pages. Choose a shorter range to include all its code.";
        return false;
      }
    }
  }
  const created = await saveNoteAt(target.side, first, target.end, content);
  if (created) cancelDraft();
  return created;
}

function rowAnchor(line: GitDiffPageLine): { side: ReviewSide; line: number } | null {
  if (line.kind === "hunk") return null;
  if (line.kind === "removed" && line.oldLineNumber) return { side: "old", line: line.oldLineNumber };
  if (line.newLineNumber) return { side: "new", line: line.newLineNumber };
  if (line.oldLineNumber) return { side: "old", line: line.oldLineNumber };
  return null;
}

function isDraftSelection(line: GitDiffPageLine | undefined): boolean {
  if (!line) return false;
  const anchor = rowAnchor(line);
  return Boolean(
    anchor &&
    draft.value &&
    anchor.side === draft.value.side &&
    anchor.line >= draft.value.start &&
    anchor.line <= draft.value.end,
  );
}

function notesForRow(line: GitDiffPageLine): ReviewNote[] {
  const anchor = rowAnchor(line);
  if (!anchor) return [];
  // A range is listed on its first line, and its composer opens on its last, where the + was.
  return fileNotes.value.filter((note) => note.side === anchor.side && note.lineStart === anchor.line);
}

function errorText(error: unknown): string {
  return isIpcError(error) ? error.message : error instanceof Error ? error.message : String(error);
}

/**
 * One `DiffFile` per hunk, which is why a hunk is handed the file's language on both sides: a hunk
 * has no name of its own to be read by.
 *
 * The library decides a language by taking everything after the last dot of the path, which names
 * one for a `src/app.vue` and nothing at all for a `Dockerfile` or a `.prettierrc`. Told the
 * language, it highlights those too. A path no grammar is detected for leaves the library to guess,
 * rather than claiming a language that does not exist.
 *
 * Each is given an identity of its own, which the library keys its own reading of a window by instead
 * of by the text of that window. Two diffs of two checkouts hold the same path and often the very same
 * lines (the same run of placeholder newlines and the same hunk) and one cache would hand the
 * second whatever it read for the first, which for the two of them is a different file's syntax. It is
 * an identity per hunk rather than per file because the key it replaces is the window's text, and two
 * hunks of one file are two different windows.
 */
function createHunks(path: string, patch: string) {
  const lang = detectedLanguageName(path);
  const identity = `${props.checkout.id}:${path}:${++diffIdentity}`;
  const preamble: string[] = [];
  const sections: Array<{ title: string; patch: string }> = [];
  let current: string[] | null = null;
  let title = "";
  for (const line of patch.split("\n")) {
    if (line.startsWith("@@")) {
      if (current) sections.push({ title, patch: [...preamble, ...current].join("\n") });
      current = [line];
      title = line;
    } else if (current) current.push(line);
    else preamble.push(line);
  }
  if (current) sections.push({ title, patch: [...preamble, ...current].join("\n") });
  return sections.map((section, index) => {
    const file = new DiffFile(`a/${path}`, "", `b/${path}`, "", [section.patch], lang, lang, `${identity}:${index}`);
    file.initTheme(theme.value);
    file.init();
    file.buildUnifiedDiffLines();
    return { title: section.title, file };
  });
}

/** What one `@@` header says: where each side of the change starts, and how many lines it covers. */
const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/**
 * The last line of the file that any hunk of this diff reaches, or nothing when a header says
 * something this cannot read.
 *
 * The library builds each hunk's window out of the file's own lines, from the first to the one that
 * hunk ends on, filling in the lines the change does not touch, so those are the only lines of the
 * file a grammar is ever asked about. A header that cannot be read leaves the answer unknown, and an
 * unknown answer is no limit at all: the whole file is read, which is what it always was.
 */
function lastLineShown(hunks: GitFileDiff["hunks"]): number | undefined {
  let last = 0;
  for (const hunk of hunks) {
    const header = HUNK_HEADER.exec(hunk.title);
    if (!header) return undefined;
    // A count git leaves off is one line, which is what `@@ -7 +7 @@` says.
    const count = (at: string | undefined) => (at === undefined ? 1 : Number(at));
    last = Math.max(last, Number(header[1]) + count(header[2]), Number(header[3]) + count(header[4]));
  }
  return last;
}

/**
 * One reading of one file at a time, and the reading of that same file that was asked for while
 * another was running as soon as the running one lands.
 *
 * Reading a file is a pass over all of it on the thread that also answers the wheel, and git reports
 * every write in the workdir, so a file being worked on is asked for over and over. A second pass
 * over the same file nobody is reading yet costs as much as the first and answers nothing sooner,
 * and the only one of such a burst worth keeping is the last. Another file is a different matter: the
 * user has moved to it and is waiting, and the reading of the file they left is one whose answer is
 * already worth nothing, so that one goes ahead rather than behind.
 */
let highlighterReading: string | null = null;
let highlighterQueued: { checkoutId: string; source: GitFileDiff } | null = null;

/**
 * Reads the grammar the file on screen is highlighted with, and the file itself for that grammar to
 * read, once the diff it is drawn from has arrived.
 *
 * Deliberately not awaited before the hunks are built: a grammar is a dynamic import and reading a
 * file is a pass over it, and waiting on either would hold back the diff text the user opened the
 * file for. Until they land the library highlights the way it always has, and `diffHighlighter`
 * changing is what makes it repaint in the same colors the editor reads the same file in. A file
 * whose language no grammar is loaded for, or whose text came back too large or not at all, simply
 * keeps the library's own highlighter, which is what it does today.
 *
 * The two sides are the whole of the file rather than the hunks of the diff, which is the only thing
 * a grammar can read: a hunk is a fragment, and the lines inside `<script setup lang="ts">` of a
 * `.vue` file are markup to a grammar that was never shown the tag that opened them. As far as the
 * diff reaches, though: a grammar is only asked about the lines the library builds a window out of,
 * which run from the first line of the file to the one the last hunk ends on, so a change near the
 * top of a large file is read as the top of that file and not as all of it.
 *
 * What comes back belongs to this diff and to nothing else, and it is published only after the guard
 * below: a reading that lands after the user has opened another file, or after this one has been read
 * again, is dropped rather than handed to a diff it is not of.
 */
async function loadHighlighter(checkoutId: string, source: GitFileDiff) {
  const language = detectedLanguageName(source.path);
  if (language === undefined) return;
  if (highlighterReading === source.path) {
    highlighterQueued = { checkoutId, source };
    return;
  }
  const request = ++highlighterRequest;
  highlighterReading = source.path;
  try {
    const { prepareDiffHighlighting } = await import("../lib/diff-highlighter");
    const highlighter = await prepareDiffHighlighting(
      language,
      { old: source.oldContent, new: source.newContent },
      lastLineShown(source.hunks),
    );
    // What this waits for is a dynamic import and a read of the file, either of which can land
    // after the user has opened another file or after this one has been read again, and either of
    // which is worth nothing to a diff that is no longer the one on screen.
    if (!highlighter || !mounted || request !== highlighterRequest) return;
    if (props.checkout.id !== checkoutId || diff.value !== source) return;
    loadedHighlighter.value = { source, highlighter };
  } catch {
    // A grammar that is not there, or one that fails to load, leaves the library to highlight the
    // file its own way. Neither is worth a toast: the diff is already on screen without them.
  } finally {
    highlighterReading = null;
    const queued = highlighterQueued;
    highlighterQueued = null;
    if (queued) void loadHighlighter(queued.checkoutId, queued.source);
  }
}

async function loadDiff(path: string, preservePosition = false) {
  const request = ++diffGeneration;
  const checkoutId = props.checkout.id;
  const oldScrollTop = preservePosition ? diffScrollTop.value : props.scrollTop;
  // Whether this reload is the same file again. A different one resets the pages through the
  // path watcher inside the composable, so it is the only case left to cover here.
  const sameFile = selectedPath.value === path;
  // E.3: a different file is a different selection, so the previous diff goes away rather
  // than sitting under the loading state. A refresh of the same file keeps its place.
  const keepPreviousDiff = diff.value !== null && sameFile;
  selectedPath.value = path;
  if (!keepPreviousDiff) {
    diff.value = null;
    diffHunks.value = [];
    collapsedHunks.value = [];
  }
  diffScrollTop.value = oldScrollTop;
  diffError.value = "";
  diffState.value = "loading";
  try {
    const result = await getGitDiff(checkoutId, path);
    if (
      !mounted ||
      request !== diffGeneration ||
      selectedPath.value !== path ||
      props.checkout.id !== checkoutId ||
      props.path !== path
    )
      return;
    // A refresh that changed nothing must not redraw. The diff view keeps the open note
    // composer in state that a new DiffFile identity wipes, and git reports every write in the
    // workdir, not just in the file on screen: rebuilding on each one closed the composer the
    // moment the user started typing, for a diff that had not moved.
    if (result.patch !== diff.value?.patch) {
      // A moved patch moves every line after the edit, so the pages on hand are stale and go.
      // An unmoved one leaves the line numbers they are indexed by exactly as they were, and
      // dropping them is what put a virtualized diff in a permanent "Loading diff page" loop:
      // each refresh blanked the window and the next one arrived before the refill had landed.
      if (sameFile) largeDiff.reset();
      diff.value = result;
      collapsedHunks.value = [];
      if (!result.isBinary && !result.symlinkTarget && result.patch.includes("@@")) {
        diffHunks.value = createHunks(path, result.patch);
        void loadHighlighter(checkoutId, result);
      }
    }
    diffState.value = "ready";
    emit("ready", path);
    await nextTick();
    if (request !== diffGeneration || props.checkout.id !== checkoutId || props.path !== path) return;
    if (diffViewport.value) diffViewport.value.scrollTop = oldScrollTop;
    if (result.large && !result.tooLarge && !result.isBinary) loadVisiblePages();
  } catch (error) {
    if (
      !mounted ||
      request !== diffGeneration ||
      selectedPath.value !== path ||
      props.checkout.id !== checkoutId ||
      props.path !== path
    )
      return;
    // A diff that cannot be read leaves the panel with nothing to show, so the reason is
    // drawn in it: a toast would expire and leave an empty panel unexplained.
    diffError.value = errorText(error);
    diffState.value = "error";
  }
}

function toggleFile(path: string) {
  expandedPaths.value = expandedPaths.value.includes(path)
    ? expandedPaths.value.filter((open) => open !== path)
    : [...expandedPaths.value, path];
}

watch(
  () => [props.checkout.id, props.path] as const,
  ([, path]) => {
    if (path !== null) void loadDiff(path);
  },
  { immediate: true, flush: "sync" },
);

/**
 * The library writes the theme onto the wrapper from the `DiffFile` it was handed, so a switch
 * reaches the diffs that are already open by telling each of them, which then repaints what it has
 * already built. Nothing is re-fetched and nothing is rebuilt, so a theme change costs a repaint and
 * not the scroll position, the expanded hunks or a half-written note under it.
 */
watch(theme, (palette) => {
  for (const hunk of diffHunks.value) {
    hunk.file.initTheme(palette);
    hunk.file.notifyAll();
  }
});

// What is open in the change set belongs to the workdir it was opened in.
watch(
  () => props.checkout.id,
  () => {
    expandedPaths.value = [];
  },
);

watch(
  () => [props.checkout.id, props.path] as const,
  () => {
    draft.value = null;
    lastAnchorChecks = [];
  },
);

let lastAnchorChecks: ReviewAnchorCheck[] = [];

// Reports the anchor text of every note whose line the diff renders, so the backend can
// stamp drift. It only fires when a resolved text changes, never on plain scrolling.
watch(
  anchorChecks,
  (checks) => {
    if (checks.length === 0 || props.path === null) {
      lastAnchorChecks = [];
      return;
    }
    const alreadyAsked =
      checks.length === lastAnchorChecks.length &&
      checks.every(
        (check, index) =>
          check.id === lastAnchorChecks[index]?.id && check.currentCode === lastAnchorChecks[index]?.currentCode,
      );
    if (alreadyAsked) return;
    lastAnchorChecks = checks.map((check) => ({ ...check }));
    void props.review.verifyAnchors(props.path, checks);
  },
  { deep: true },
);

// Git reports every write in the workdir, on a 220ms debounce, and an agent writing a file
// produces a steady stream of them. Reloading on each one spends the whole stream fetching a
// diff the user never sees land, so a burst collapses into the single reload it amounts to.
const STATUS_REFRESH_DEBOUNCE = 400;

watch(
  () => props.gitSnapshot.statusRevision,
  (revision, previous, onCleanup) => {
    if (revision === previous || props.path === null) return;
    if (props.gitSnapshot.checkoutId !== props.checkout.id || !props.gitSnapshot.status) return;
    const path = props.path;
    const timer = setTimeout(() => {
      if (props.gitSnapshot.status?.files.some((file) => file.path === path)) {
        void loadDiff(path, true);
        return;
      }
      diffGeneration += 1;
      diff.value = null;
      diffHunks.value = [];
      diffScrollTop.value = 0;
      diffError.value = "This file is no longer in the current Git changes.";
      diffState.value = "error";
    }, STATUS_REFRESH_DEBOUNCE);
    onCleanup(() => clearTimeout(timer));
  },
);

function onDiffScroll(event: Event) {
  diffScrollTop.value = (event.currentTarget as HTMLElement).scrollTop;
  emit("scrollPositionChanged", diffScrollTop.value);
  if (diff.value?.large) loadVisiblePages();
}

function isHunkCollapsed(index: number): boolean {
  return collapsedHunks.value.includes(index);
}

function toggleHunk(index: number) {
  collapsedHunks.value = collapsedHunks.value.includes(index)
    ? collapsedHunks.value.filter((collapsed) => collapsed !== index)
    : [...collapsedHunks.value, index];
  if (diff.value?.large) {
    void nextTick(() => {
      // Collapsing a hunk shortens the diff, and a scroll position past the new end is one the
      // browser pulls back on its own; it also moves the window, so the pages it needs are asked
      // for again from wherever it ended up.
      diffScrollTop.value = diffViewport.value?.scrollTop ?? 0;
      loadVisiblePages();
    });
  }
}

/**
 * Collapses a hunk from the header the library draws for it.
 *
 * The library already renders a row per `@@` header, styled by the properties above, so a header of
 * our own above each hunk is the same header twice and a row of the diff spent on saying it. The click
 * is delegated to the hunk's own section because that is what knows which hunk it is, and the
 * library's expand buttons live inside that same row and are left to do their own job.
 */
function toggleHunkFromLibrary(event: MouseEvent, index: number) {
  const target = event.target as HTMLElement | null;
  if (!target?.closest("tr[data-line$='-hunk']") || target.closest(".diff-widget-tooltip")) return;
  toggleHunk(index);
}

onUnmounted(() => {
  mounted = false;
  diffGeneration += 1;
  // A reading queued for a file nobody is looking at any more is a whole pass over it for nothing.
  highlighterQueued = null;
});
</script>

<template>
  <section class="relative flex min-h-0 flex-1 flex-col" aria-label="File diff">
    <!-- A file of the change-set stack is headed by its own row, so it needs no header here. -->
    <header v-if="!embedded" class="document-toolbar shrink-0 border-b px-3 py-1.5">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <p class="truncate text-[0.6875rem] text-(--marvis-text-dim)" :title="title">{{ title }}</p>
          <p v-if="!path && branch" class="truncate text-[0.6875rem] text-(--marvis-text-faint)" :title="branch">
            {{ branch }}
          </p>
        </div>
      </div>
      <div v-if="sender" class="mt-1.5 flex flex-col items-end gap-1">
        <div class="flex flex-wrap items-center justify-end gap-1.5">
          <button
            type="button"
            data-testid="send-review"
            :disabled="!canSend"
            :aria-label="sender.target === 'markdown' ? 'Export as Markdown' : 'Send to opencode'"
            class="marvis-button marvis-button-tinted marvis-button-md"
            @click="requestSend"
          >
            {{ sender.target === "markdown" ? "Export as Markdown" : "Send to opencode" }}
          </button>
          <SelectControl
            :model-value="sender.target"
            :options="reviewTargets"
            label="Review destination"
            testid="review-target"
            @update:model-value="chooseReviewTarget"
          />
          <!-- One session is the default target, so the picker only earns its place above one.
               It is the app's own menu rather than a native select, and it earns a search only
               once there are more rows than a short list is worth reading. -->
          <PopoverRoot v-if="sender.target === 'opencode' && sender.sessions.length > 1" v-model:open="targetOpen">
            <PopoverTrigger
              data-testid="send-target"
              aria-label="Send review to"
              :title="targetSession?.title"
              class="marvis-select max-w-64"
            >
              <span class="min-w-0 truncate">{{ targetSession?.title ?? "Choose a session" }}</span>
              <ChevronDownIcon class="icon-xs shrink-0 text-(--marvis-text-faint)" aria-hidden="true" />
            </PopoverTrigger>
            <!-- Portalled for the same reason the document toolbar portals its own: the list is
                 absolutely positioned and the diff under it paints over anything left in place. -->
            <PopoverPortal>
              <!-- The keys are read here rather than on the search, so the menu answers them the
                   same whether or not a checkout has enough sessions to be searched. -->
              <PopoverContent
                side="bottom"
                align="end"
                :side-offset="4"
                class="surface-popover marvis-menu session-target-menu"
                @keydown.down.prevent="moveTargetRow(1)"
                @keydown.up.prevent="moveTargetRow(-1)"
                @keydown.enter.prevent="chooseActiveTarget"
              >
                <input
                  v-if="searchesTargets"
                  v-model="targetQuery"
                  type="search"
                  aria-label="Search sessions"
                  placeholder="Search…"
                  class="marvis-menu-search min-w-0 appearance-none"
                />
                <div role="listbox" aria-label="Send review to" class="marvis-menu-scroll flex flex-col">
                  <button
                    v-for="(row, index) in targetRows"
                    :key="row.id"
                    type="button"
                    role="option"
                    :aria-selected="row.id === sender.targetId"
                    :title="row.title"
                    class="menu-item session-target-option select-none text-left"
                    :class="{ 'is-active': index === activeTargetRow }"
                    @click="chooseTarget(row.id)"
                  >
                    <span class="menu-item-label session-target-label">{{ row.title }}</span>
                    <CheckIcon v-if="row.id === sender.targetId" class="icon-xxs menu-check" aria-hidden="true" />
                  </button>
                  <p v-if="!targetRows.length" class="menu-note">No session matches "{{ targetQuery }}".</p>
                </div>
              </PopoverContent>
            </PopoverPortal>
          </PopoverRoot>
          <span v-if="canSend" data-testid="send-count" class="text-[0.6875rem] text-(--marvis-text-faint)">
            {{ sendableNotes.length }} {{ sendableNotes.length === 1 ? "note" : "notes" }}
            <template v-if="draftCount">· {{ draftCount }} {{ draftCount === 1 ? "draft" : "drafts" }}</template>
          </span>
        </div>
        <div v-if="showBusyChoice" data-testid="send-busy" class="flex flex-col items-end gap-1">
          <p class="max-w-prose text-right text-[0.6875rem] text-(--marvis-text-secondary)">
            “{{ targetSession?.title }}” is mid-task. Sending now lands inside its current turn; queueing waits for it
            to finish. This OpenCode version cannot cancel a turn.
          </p>
          <div class="flex items-center gap-1.5">
            <button
              type="button"
              data-testid="send-now"
              class="marvis-button marvis-button-tinted marvis-button-xs"
              @click="sendNow(false)"
            >
              Send now
            </button>
            <button
              type="button"
              data-testid="send-queue"
              class="marvis-button marvis-button-subtle marvis-button-xs"
              @click="sendNow(true)"
            >
              Queue
            </button>
            <button
              type="button"
              data-testid="send-not-now"
              class="marvis-button marvis-button-ghost marvis-button-xs"
              @click="busyChoiceOpen = false"
            >
              Not now
            </button>
          </div>
        </div>
        <p
          v-if="sender.target === 'opencode' && sender.unfinishedRounds > 0"
          data-testid="unfinished-rounds"
          class="text-[0.6875rem] text-(--marvis-text-faint)"
        >
          {{ sender.unfinishedRounds }} {{ sender.unfinishedRounds === 1 ? "round" : "rounds" }} not finished
        </p>
        <label v-if="outdatedCount > 0" class="flex items-center gap-1.5 text-[0.6875rem] text-(--marvis-text-faint)">
          <input v-model="includeOutdated" type="checkbox" class="marvis-check" data-testid="include-outdated" />
          Include {{ outdatedCount }} outdated {{ outdatedCount === 1 ? "note" : "notes" }}
        </label>
      </div>
    </header>
    <template v-if="path === null">
      <div class="diff-files min-h-0 flex-1 overflow-auto">
        <p v-if="gitSnapshot.statusState === 'loading'" role="status" class="pane-state text-sm">Loading changes…</p>
        <p v-else-if="gitSnapshot.statusState === 'error'" role="alert" class="pane-state text-sm">
          {{ gitSnapshot.changesStatusError || gitSnapshot.statusError }}
        </p>
        <p v-else-if="changedFiles.length === 0" role="status" class="pane-state text-sm">No changed files.</p>
        <template v-else>
          <section v-for="file in changedFiles" :key="file.path" class="diff-file">
            <button
              type="button"
              class="diff-file-header"
              :aria-expanded="expandedPaths.includes(file.path)"
              :title="file.oldPath ? `${file.oldPath} → ${file.path}` : file.path"
              @click="toggleFile(file.path)"
            >
              <span class="diff-status" :data-status="file.status">{{ file.status }}</span>
              <span class="truncate" :title="file.path">{{ file.path }}</span>
              <span v-if="file.additions" class="diff-add">+{{ file.additions }}</span>
              <span v-if="file.deletions" class="diff-del">-{{ file.deletions }}</span>
            </button>
            <!-- Git pages one path at a time, so the whole change set is a stack of single-file
                 diffs rather than one merged patch. Each file is diffed when it is opened, and each
                 of those reads at the same size as the diff this one is drawn in. -->
            <FileDiff
              v-if="expandedPaths.includes(file.path)"
              :checkout="checkout"
              :git-snapshot="gitSnapshot"
              :review="review"
              :path="file.path"
              :scroll-top="0"
              :editor-settings="editorSettings"
              embedded
            />
          </section>
        </template>
      </div>
    </template>
    <template v-else>
      <p v-if="diffState === 'loading' && !diff" role="status" class="pane-state text-sm">Loading diff…</p>
      <!-- A diff that will not load leaves nothing else to show, so the reason is drawn here
           rather than left to a toast that expires over an empty panel. -->
      <p v-else-if="diffState === 'error'" role="alert" class="pane-state text-sm">
        {{ diffError || "This diff could not be read." }}
      </p>
      <!-- A state of the file itself, not a failure: the panel's whole content is the reason,
           so it stays drawn here rather than expiring in a toast. Same for the three below. -->
      <p v-else-if="diff?.isBinary" role="status" class="pane-state text-sm">Binary file; text diff is unavailable.</p>
      <p v-else-if="diff?.symlinkTarget !== undefined" role="status" class="pane-state text-sm">
        Symlink target: <code class="break-all text-(--marvis-text)">{{ diff.symlinkTarget }}</code>
      </p>
      <p v-else-if="diff?.tooLarge" role="status" class="pane-state text-sm">
        This diff exceeds safe preview limits (100,000 lines, 10,000 hunks, 32 MiB, 4 KiB hunk headers, or 64 KiB per
        line). Reduce the change size to view it.
      </p>
      <p v-else-if="showNoTextHunks" role="status" class="pane-state text-sm">
        No text hunks are available for this change.
      </p>
      <template v-else-if="(diffState === 'ready' || diffState === 'loading') && diff">
        <p v-if="diff.large" class="shrink-0 px-3 py-1 text-[0.625rem] text-(--marvis-text-faint)">
          {{ diff.totalLines.toLocaleString() }} diff rows · virtualized view · click + note on two lines to comment on
          a range
        </p>
        <!-- How a range is picked is a fact about the diff, not about the file it is of, so the
             change-set stack is told once by the diff it is showing and not once per file in it. -->
        <p v-else-if="!embedded" class="shrink-0 px-3 py-1 text-[0.625rem] text-(--marvis-text-faint)">
          Drag across line numbers to select a range, then click + note on its last line.
        </p>
        <div
          ref="diffViewport"
          class="diff-viewport min-h-0 flex-1 overflow-auto font-mono"
          :class="{ 'diff-viewport-windowed': diff.large && !diff.tooLarge }"
          :style="{
            '--marvis-diff-row-height': `${diffRowPx}px`,
            '--marvis-diff-font-size': `${diffFontSize}px`,
          }"
          aria-label="Diff contents"
          @scroll="onDiffScroll"
        >
          <template v-if="diff.large && !diff.tooLarge">
            <div
              :style="{
                paddingTop: `${visibleLargeDiffWindow.paddingTop}px`,
                paddingBottom: `${visibleLargeDiffWindow.paddingBottom}px`,
              }"
            >
              <template v-for="row in visibleLargeDiffWindow.rows" :key="row.visualIndex">
                <!-- The row is exactly one row height tall, and that is what the window above
                     measures: a row taller than the arithmetic says puts every line under it
                     somewhere the padding did not, and the diff drifts as the user scrolls. -->
                <div
                  data-testid="large-diff-row"
                  class="group flex min-w-max items-center overflow-hidden whitespace-pre"
                  :style="{ height: `${diffRowPx}px`, fontSize: 'var(--marvis-diff-font-size)' }"
                  :class="{ 'review-range-selected': isDraftSelection(row.line) }"
                >
                  <button
                    v-if="row.line?.kind === 'hunk'"
                    type="button"
                    class="diff-hunk h-full w-full truncate px-2 text-left text-(--marvis-content-accent)"
                    :aria-expanded="!row.collapsed"
                    :title="row.line.text"
                    @click="toggleHunk(row.hunkIndex)"
                  >
                    {{ row.collapsed ? "▸" : "▾" }} {{ row.line.text }}
                  </button>
                  <template v-else-if="row.line">
                    <span class="w-12 shrink-0 select-none pr-2 text-right text-(--marvis-content-text-faint)">{{
                      row.line.oldLineNumber ?? ""
                    }}</span>
                    <span class="w-12 shrink-0 select-none pr-2 text-right text-(--marvis-content-text-faint)">{{
                      row.line.newLineNumber ?? ""
                    }}</span>
                    <span
                      class="w-4 shrink-0 text-center"
                      :class="
                        row.line.kind === 'added'
                          ? 'text-(--marvis-content-added)'
                          : row.line.kind === 'removed'
                            ? 'text-(--marvis-content-removed)'
                            : 'text-(--marvis-content-text-faint)'
                      "
                      >{{ row.line.text[0] }}</span
                    >
                    <code
                      class="pr-4"
                      :class="
                        row.line.kind === 'added'
                          ? 'text-(--marvis-content-added)'
                          : row.line.kind === 'removed'
                            ? 'text-(--marvis-content-removed)'
                            : 'text-(--marvis-content-text-muted)'
                      "
                      >{{ row.line.text.slice(1) }}</code
                    >
                    <button
                      v-if="rowAnchor(row.line)"
                      type="button"
                      class="marvis-button marvis-button-ghost marvis-button-xs ml-auto shrink-0 opacity-0 group-hover:opacity-100"
                      :aria-label="`Add review note on line ${rowAnchor(row.line)!.line}`"
                      @click="openDraft(rowAnchor(row.line)!.side, rowAnchor(row.line)!.line)"
                    >
                      + note
                    </button>
                  </template>
                  <span v-else class="px-2 text-(--marvis-content-text-muted)">{{
                    row.error || "Loading diff page…"
                  }}</span>
                </div>
                <!-- The note cards stay in the flow, which is what leaves the arithmetic above
                     approximate on an annotated row. The composer does not, and is held by the
                     panel below instead; see there for why. -->
                <ReviewNoteList
                  v-if="row.line && notesForRow(row.line).length > 0"
                  :notes="notesForRow(row.line)"
                  :outcomes="anchorOutcomes"
                  @update-note="review.updateNote"
                  @delete-note="review.deleteNote"
                  @clear-outdated="review.clearOutdated"
                  @resolve-note="review.resolveNote"
                />
              </template>
            </div>
          </template>
          <div v-else class="min-w-max p-1">
            <section
              v-for="(hunk, index) in diffHunks"
              :key="`${path}-${index}`"
              :ref="(el) => (hunkSections[index] = (el as HTMLElement | null) ?? null)"
              class="relative min-w-0 [&_.diff-line-hunk]:cursor-pointer"
              @click="toggleHunkFromLibrary($event, index)"
            >
              <!-- One control per hunk, and the same one in both of its states.
                   The library draws the `@@` header itself and a click on it collapses the hunk, but
                   a table row is not a control: there is nothing on it for the keyboard to reach and
                   nothing for a screen reader to name. So the row keeps the click and this is the
                   same action as a control next to it: out of the flow and out of sight while the
                   hunk is open, because a header drawn here while the library draws one is the header
                   twice and a row of the diff spent on it. It is drawn over that header row only once
                   the keyboard has brought it there, which is the one state where a control of ours
                   belongs on the header. It is one element either way, so the focus that collapsed
                   the hunk is still on it when the hunk comes back, and a collapsed hunk gets the
                   visible header row it needs to be reopened from. -->
              <button
                type="button"
                data-testid="hunk-toggle"
                class="diff-hunk diff-hunk-toggle"
                :class="
                  isHunkCollapsed(index)
                    ? 'w-full truncate border-b border-(--marvis-content-border) px-2 py-1 text-left font-mono text-[0.625rem]'
                    : 'sr-only'
                "
                :aria-expanded="!isHunkCollapsed(index)"
                :aria-label="`${isHunkCollapsed(index) ? 'Expand' : 'Collapse'} hunk ${hunk.title}`"
                :title="hunk.title"
                @click="toggleHunk(index)"
              >
                {{ isHunkCollapsed(index) ? `▸ ${hunk.title}` : "" }}
              </button>
              <DiffViewWithMultiSelect
                v-if="!isHunkCollapsed(index)"
                :diff-file="hunk.file"
                :diff-view-mode="DiffModeEnum.Unified"
                :diff-view-theme="theme"
                :diff-view-add-widget="true"
                :extend-data="extendData"
                :diff-view-highlight="true"
                :diff-view-font-size="diffFontSize"
                :register-highlighter="diffHighlighter"
                class="min-w-0"
              >
                <template #widget="{ lineNumber, fromLineNumber, side, onClose }">
                  <!-- The library's own selection says where the range starts and ends, which is
                       already the shape a range is stored in. The composer may still move the top of
                       it, so what it emits is what gets saved rather than what the library chose. -->
                  <ReviewComposer
                    :side="sideName(side)"
                    :line-start="fromLineNumber"
                    :line-end="lineNumber"
                    :start-options="startCandidates(sideName(side), lineNumber)"
                    @update:line-start="
                      (start: number) => {
                        libraryDraft = { side: sideName(side), start, end: lineNumber };
                      }
                    "
                    @submit="
                      async (content: string, lineStart: number) => {
                        if (await saveNoteAt(sideName(side), lineStart, lineNumber, content))
                          closeLibraryDraft(onClose);
                      }
                    "
                    @cancel="closeLibraryDraft(onClose)"
                  />
                </template>
                <template #extend="{ data }">
                  <ReviewNoteList
                    :notes="data"
                    :outcomes="anchorOutcomes"
                    @update-note="review.updateNote"
                    @delete-note="review.deleteNote"
                    @clear-outdated="review.clearOutdated"
                    @resolve-note="review.resolveNote"
                  />
                </template>
              </DiffViewWithMultiSelect>
            </section>
          </div>
        </div>
        <div v-if="notesWithoutLine.length > 0" class="shrink-0 border-t border-(--marvis-border)">
          <p class="px-2 py-1 text-[0.625rem] text-(--marvis-text-faint)">
            {{ notesWithoutLine.length }}
            {{ notesWithoutLine.length === 1 ? "note points" : "notes point" }} at a line that is no longer in this diff
          </p>
          <ReviewNoteList
            :notes="notesWithoutLine"
            :outcomes="anchorOutcomes"
            @update-note="review.updateNote"
            @delete-note="review.deleteNote"
            @clear-outdated="review.clearOutdated"
            @resolve-note="review.resolveNote"
          />
        </div>
        <!-- The composer is a layer over the diff and not another row of it. Two reasons, and the
             second is the one that decides where it goes: a composer in the flow makes its row
             taller than the window's arithmetic counts, and a composer anchored to a row is gone
             the moment that row scrolls out, which is what a range spanning more than a screen
             does. Held here it survives the scroll that finishes the range. -->
        <ReviewComposer
          v-if="diff.large && !diff.tooLarge && draft"
          v-model:line-start="draft.start"
          class="absolute inset-x-0 bottom-0 z-10"
          :side="draft.side"
          :line-end="draft.end"
          :start-options="startCandidates(draft.side, draft.end)"
          :error="draftError"
          @submit="saveDraft"
          @cancel="cancelDraft"
        />
      </template>
    </template>
  </section>
</template>

<style scoped>
/**
 * The diff library paints itself with `--diff-*` custom properties on `.diff-style-root`, one
 * pair per kind of line. That is the only seam it offers, so F.5 repaints those with the
 * marvis palette instead of replacing its renderer. The selector is deliberately longer than
 * the library's own `[data-theme]` rules: same-specificity rules would be settled by
 * stylesheet order, which is not something a component can rely on.
 *
 * Both attributes are matched at once through `:is()` because the library ships one copy of
 * each of its rules per theme and a diff is repainted as the theme changes: writing this once
 * for the theme in effect would leave the other palette's rules standing.
 */
.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"]) .diff-style-root
  ) {
  --diff-border--: var(--marvis-content-border);
  --diff-plain-content--: var(--marvis-content-bg-0);
  --diff-plain-lineNumber--: var(--marvis-content-bg-0);
  --diff-expand-content--: var(--marvis-content-bg-1);
  --diff-expand-lineNumber--: var(--marvis-content-bg-1);
  --diff-empty-content--: var(--marvis-content-bg-0);
  --diff-plain-lineNumber-color--: var(--marvis-content-text-faint);
  --diff-expand-lineNumber-color--: var(--marvis-content-text-faint);
  /* An added or removed line is the diff's green and red, the same ones the stats use. */
  --diff-add-content--: color-mix(in srgb, var(--marvis-content-added) 14%, var(--marvis-content-bg-0));
  --diff-del-content--: color-mix(in srgb, var(--marvis-content-removed) 14%, var(--marvis-content-bg-0));
  --diff-add-lineNumber--: color-mix(in srgb, var(--marvis-content-added) 22%, var(--marvis-content-bg-0));
  --diff-del-lineNumber--: color-mix(in srgb, var(--marvis-content-removed) 22%, var(--marvis-content-bg-0));
  --diff-add-content-highlight--: color-mix(in srgb, var(--marvis-content-added) 24%, var(--marvis-content-bg-0));
  --diff-del-content-highlight--: color-mix(in srgb, var(--marvis-content-removed) 24%, var(--marvis-content-bg-0));
  --diff-hunk-content--: var(--marvis-content-bg-1);
  --diff-hunk-lineNumber--: var(--marvis-content-bg-1);
  --diff-hunk-lineNumber-hover--: var(--marvis-content-accent);
  --diff-hunk-content-color--: var(--marvis-content-text-muted);
  --diff-add-widget--: var(--marvis-accent);
  --diff-add-widget-color--: var(--marvis-accent-fg);
  --diff-multi-select-bg: var(--marvis-accent);
  --diff-multi-select-border: var(--marvis-accent);
}

.diff-viewport :deep(.diff-add-widget),
.diff-viewport :deep(.diff-widget-tooltip),
.diff-viewport :deep(.diff-widget-tooltip::after) {
  border-radius: 0;
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"]) [data-state="diff"]
  ),
.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      [data-state="plain"]
  ),
.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"]) [data-state="hunk"]
  ) {
  color: var(--marvis-content-text);
}

/* The syntax inside a diff is highlighted by the library's own highlight.js, which ships a GitHub
   light and a GitHub dark palette and knows nothing about these two. The classes are its own, so
   the tokens are named by what the token is rather than by what the library calls it: a diff and
   the same file read-only in the editor are one palette. */
.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs
  ) {
  color: var(--marvis-syntax-foreground);
  background: transparent;
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(
        .hljs-doctag,
        .hljs-keyword,
        .hljs-template-tag,
        .hljs-template-variable,
        .hljs-type,
        .hljs-variable.language_
      )
  ) {
  color: var(--marvis-syntax-token-keyword);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-title, .hljs-title.class_, .hljs-title.class_.inherited__, .hljs-title.function_, .hljs-section)
  ) {
  color: var(--marvis-syntax-token-function);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(
        .hljs-attr,
        .hljs-attribute,
        .hljs-literal,
        .hljs-meta,
        .hljs-number,
        .hljs-operator,
        .hljs-variable,
        .hljs-selector-attr,
        .hljs-selector-class,
        .hljs-selector-id,
        .hljs-built_in,
        .hljs-symbol
      )
  ) {
  color: var(--marvis-syntax-token-constant);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-regexp, .hljs-string, .hljs-meta .hljs-string)
  ) {
  color: var(--marvis-syntax-token-string);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-comment, .hljs-code, .hljs-formula)
  ) {
  color: var(--marvis-syntax-token-comment);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-name, .hljs-selector-tag, .hljs-selector-pseudo)
  ) {
  color: var(--marvis-syntax-token-string-expression);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-char.escape_, .hljs-link, .hljs-params, .hljs-property, .hljs-punctuation, .hljs-tag, .hljs-quote)
  ) {
  color: var(--marvis-syntax-token-punctuation);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-addition
  ) {
  color: var(--marvis-syntax-token-inserted);
  background-color: transparent;
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-deletion
  ) {
  color: var(--marvis-syntax-token-deleted);
  background-color: transparent;
}

/* These rules paint the library's own highlight.js tokens, which is what a diff is highlighted with
   when `diff-highlighter.ts` has no grammar for its language: the library falls back to its own
   highlighter for those, and these are the rules that give that output Marvis' palette rather than
   the GitHub one highlight.js ships. A diff whose grammar Marvis does have is highlighted by Shiki
   instead and needs none of this: its tokens already name the CSS variables.
   The seven added here are the classes those grammars emit that no rule above named, and which were
   therefore being painted in whatever the library's palette said. `hljs-function` is what wraps a
   call's parentheses and its callback's arrow together, so it takes the function color the call
   above it already has; `hljs-subst` is the `${…}` of a template literal and `hljs-class` the class
   a Scala or Elixir declaration is named by, both as the editor paints them. The rest are a list
   marker, emphasis, bold and strike-through, the last three styled as `marvisHighlightStyle` already
   styles them. What stays plain here is what highlight.js never classifies at all (the name a line
   declares, `=`, `!`, `||`) because it hands those back as text with no class to match. That is the
   fallback's own granularity, and the reason it is the fallback. */
.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      :is(.hljs-function, .hljs-title.function_)
  ) {
  color: var(--marvis-syntax-token-function);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-class
  ) {
  color: var(--marvis-syntax-token-string-expression);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-subst
  ) {
  color: var(--marvis-syntax-token-string);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-bullet
  ) {
  color: var(--marvis-syntax-token-punctuation);
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-emphasis
  ) {
  font-style: italic;
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-strong
  ) {
  font-weight: bold;
}

.diff-viewport
  :deep(
    :is(.diff-tailwindcss-wrapper[data-theme="dark"], .diff-tailwindcss-wrapper[data-theme="light"])
      .diff-line-syntax-raw
      .hljs-strikethrough
  ) {
  text-decoration: line-through;
}

/* Hunk headers sit on the change's own surface, as the mockup's group rows do. */
.diff-hunk {
  background: var(--marvis-content-bg-1);
  color: var(--marvis-content-text-muted);
}

.diff-hunk:hover {
  background: var(--marvis-content-bg-2);
  color: var(--marvis-content-text);
}

/**
 * The control that collapses an open hunk, out of sight for as long as the hunk is open.
 *
 * It is clipped rather than hidden, so it stays in the tab order and a screen reader still names it
 * while the eye does not see it, and it takes itself out of the pointer's way so the one pixel it
 * collapses to cannot swallow the click on the header row beside it. Nothing here depends on which
 * stylesheet came first: the rule carries a pseudo-class, so it outranks the `sr-only` it undoes,
 * which is why it clears the clip by hand: both the `clip` and the `clip-path` it may be hiding
 * behind, since which of the two a given version of the utility uses is not ours to know.
 */
.diff-hunk-toggle.sr-only {
  pointer-events: none;
}

.diff-hunk-toggle.sr-only:focus {
  position: absolute;
  top: 0;
  right: 4px;
  z-index: 1;
  width: auto;
  height: var(--marvis-diff-row-height);
  padding: 0 6px;
  margin: 0;
  overflow: visible;
  clip: auto;
  clip-path: none;
  white-space: nowrap;
  pointer-events: auto;
  border: 1px solid var(--marvis-content-border);
  border-radius: 3px;
  background: var(--marvis-content-bg-2);
  color: var(--marvis-text);
  font-family: inherit;
  font-size: 0.625rem;
}

/* One row per changed file in the whole change set, and its diff under it. */
.diff-file + .diff-file {
  border-top: 1px solid var(--marvis-content-border);
}

.diff-file-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  min-height: 22px;
  padding: 2px 6px;
  border: none;
  background: transparent;
  color: var(--marvis-text);
  font-family: inherit;
  font-size: 0.6875rem;
  text-align: left;
  white-space: nowrap;
  overflow: hidden;
  cursor: pointer;
}

.diff-file-header:hover {
  background: var(--marvis-control-hover);
}

/**
 * The range the note being written covers, as a band rather than a set of tinted rows: a rule down
 * the leading edge is what makes a block of lines read as one thing, and it stays continuous
 * however many rows the range spans.
 */
.review-range-selected {
  position: relative;
  background: color-mix(in srgb, var(--marvis-content-accent) 12%, var(--marvis-content-bg-0));
  box-shadow: inset 3px 0 0 var(--marvis-content-accent);
}

.diff-status {
  flex-shrink: 0;
  font-size: 0.625rem;
  color: var(--marvis-text-secondary);
}

.diff-status[data-status="A"] {
  color: var(--marvis-success-fg);
}

.diff-status[data-status="D"] {
  color: var(--marvis-danger-fg);
}

.diff-status[data-status="U"] {
  color: var(--marvis-text-faint);
}

.diff-file-header:hover .diff-status[data-status="U"] {
  color: var(--marvis-text);
}

/* A file inside the change-set stack scrolls on its own, so the virtual window of a large diff has
   a container to follow, and only a large diff gets one: a short file drawn at its own height is
   read off the stack's one scrollbar, and two scrollbars for one file is one too many. The cap is a
   count of diff rows rather than a share of the window, because what a diff is allowed to be tall
   has nothing to do with how tall the window is, and `vh` made the same file twice as tall on a
   large display as on a small one. */
.diff-file :deep(.diff-viewport-windowed) {
  max-height: calc(var(--marvis-diff-row-height) * 24);
}

/* The row height the virtual window's arithmetic is in, which is the whole reason the library's own
   `leading-[1.6]` is overridden: at that leading its rows are taller than the rows the arithmetic
   counts, so a virtualized diff drifts away from the line the user is looking at as they scroll.
   The selector is one class longer than the library's for the reason the rules above explain. */
.diff-viewport :deep(.diff-tailwindcss-wrapper .diff-table-body) {
  line-height: var(--marvis-diff-row-height);
}
</style>
