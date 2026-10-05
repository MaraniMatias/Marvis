// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Terminal as Headless } from "@xterm/headless";
import type { ILink, ILinkProvider } from "@xterm/xterm";
import { registerFilePathLinks } from "./terminal-file-links";

/**
 * The probe, stubbed at the IPC boundary rather than at the filesystem: what is under test is
 * which paths reach the backend, which come back as links, and what a click does with one.
 *
 * The terminal, on the other hand, is a real one. The coordinates this feature depends on —
 * xterm's 1-based rows against `getLine`'s 0-based ones, the marker that is an offset from the
 * cursor rather than a row, a wide glyph occupying two cells — are exactly the ones a hand-written
 * mock gets wrong in the direction that looks fine, so they are read off a real buffer instead.
 */
const probe = vi.hoisted(() => vi.fn<(checkoutId: string, path: string) => Promise<{ path: string } | null>>());

vi.mock("./ipc", () => ({ probeCheckoutFile: probe }));

/** The columns a link covers, as a test reads them: 1-based, last cell inclusive. */
function columns(link: ILink): [number, number] {
  return [link.range.start.x, link.range.end.x];
}

let terminal: Headless;
let provider: ILinkProvider | null;
let decorations: { x: number; width: number; foregroundColor?: string }[] = [];
let markerOffsets: number[] = [];
let open: ReturnType<typeof vi.fn<(path: string) => void>>;

async function write(text: string) {
  await new Promise<void>((resolve) => terminal.write(text, resolve));
}

/** Puts the links on a real terminal, with the drawing calls recorded rather than performed. */
function mount() {
  provider = null;
  decorations = [];
  markerOffsets = [];
  open = vi.fn<(path: string) => void>();
  const real = terminal as unknown as Record<string, unknown>;
  real.registerLinkProvider = (registered: ILinkProvider) => {
    provider = registered;
    return { dispose: () => (provider = null) };
  };
  real.registerMarker = (offset: number) => {
    markerOffsets.push(offset);
    return { dispose: () => {} };
  };
  real.registerDecoration = (options: { x: number; width: number; foregroundColor?: string }) => {
    decorations.push(options);
    return { dispose: () => (decorations = decorations.filter((item) => item !== options)) };
  };
  return registerFilePathLinks(terminal as never, { checkoutId: "checkout:one", open });
}

/** What the provider offers for one 1-based row, which is what xterm underlines. */
async function linksOn(row: number): Promise<ILink[] | undefined> {
  return new Promise((resolve) => {
    if (!provider) throw new Error("no provider registered");
    provider.provideLinks(row, resolve);
  });
}

beforeEach(async () => {
  terminal = new Headless({ allowProposedApi: true, cols: 80, rows: 40 });
  probe.mockReset();
  probe.mockResolvedValue(null);
  document.documentElement.style.setProperty("--marvis-accent", "#74ade8");
  // An empty write is still a write: it is what flushes xterm's parser, so the rows a test writes
  // after it start at row 1 rather than after whatever the last one left on the screen.
  await write("");
});

describe("registerFilePathLinks coordinates", () => {
  it("offers the row xterm names, not the row below it", async () => {
    await write("first line\r\nerror in src/lib/foo.ts\r\n");
    probe.mockImplementation(async (_checkoutId, path) => (path === "src/lib/foo.ts" ? { path } : null));
    mount();

    // Row 2 is `error in src/lib/foo.ts`; the path starts at column 10 and the file is 14
    // characters, so it covers columns 10 through 23 in xterm's 1-based, last-cell-inclusive
    // numbering.
    const [link] = (await linksOn(2))!;

    expect(link.text).toBe("src/lib/foo.ts");
    expect(link.range).toEqual({ start: { x: 10, y: 2 }, end: { x: 23, y: 2 } });
    // Nothing on row 1, which is where the 0-based read would have landed.
    expect(await linksOn(1)).toBeUndefined();
  });

  it("anchors the marker by its offset from the cursor, not by its row", async () => {
    await write("first\r\nsecond\r\nsrc/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(3))!;
    link.hover?.({} as MouseEvent, link.text);

    // The cursor sits on the empty row after the last one written, and the link is one row above
    // it, so the offset is -1. An absolute row here would have been 2 and put the paint two rows
    // down from the path.
    expect(markerOffsets).toEqual([-1]);
  });

  it("gives a marker for a row above the cursor a negative offset", async () => {
    await write("src/lib/foo.ts\r\nmore output\r\nand more\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(1))!;
    link.hover?.({} as MouseEvent, link.text);

    expect(markerOffsets).toEqual([-3]);
  });

  it("paints a 0-based column range, which is not what the link range is", async () => {
    await write("error in src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(1))!;
    link.hover?.({} as MouseEvent, link.text);

    // The link says 1-based 10..23; a decoration is 0-based and carries its own width.
    expect(decorations).toMatchObject([{ x: 9, width: 14, foregroundColor: "#74ade8" }]);
  });

  it("finds a path that xterm wrapped across two rows", async () => {
    // Narrow enough that the path cannot fit on one row, which is the only way xterm marks a row
    // as a continuation.
    terminal = new Headless({ allowProposedApi: true, cols: 20, rows: 5 });
    await write("");
    await write("verylongdirectory/src/app.ts\r\n");
    probe.mockResolvedValue({ path: "verylongdirectory/src/app.ts" });
    mount();

    expect(terminal.buffer.active.getLine(0)?.isWrapped).toBe(false);
    expect(terminal.buffer.active.getLine(1)?.isWrapped).toBe(true);

    const links = (await linksOn(2))!;

    // One link per row, because a decoration is one row and the click has to work on either.
    expect(links).toHaveLength(2);
    // The path is 28 characters and the row is 20 wide, so it fills the first row and takes 8
    // columns of the second.
    expect(columns(links[0])).toEqual([1, 20]);
    expect(columns(links[1])).toEqual([1, 8]);
    expect(links.map((link) => link.range.start.y)).toEqual([1, 2]);
    expect(links.every((link) => link.text === "verylongdirectory/src/app.ts")).toBe(true);
  });

  it("paints every row of a wrapped path, whichever row the pointer is on", async () => {
    // The bug this covers is visual: xterm underlines only the link under the pointer, so an accent
    // drawn for one row leaves the rest of the path looking cut off at the wrap.
    terminal = new Headless({ allowProposedApi: true, cols: 20, rows: 5 });
    await write("");
    await write("verylongdirectory/src/app.ts\r\n");
    probe.mockResolvedValue({ path: "verylongdirectory/src/app.ts" });
    mount();

    // Hover the second row, the one no earlier test looked at.
    const [second] = [(await linksOn(2))![1]];
    second.hover?.({} as MouseEvent, second.text);

    // Row 1 fills its 20 columns; the path is 28 characters, so row 2 takes the remaining 8.
    expect(decorations).toMatchObject([
      { x: 0, width: 20 },
      { x: 0, width: 8 },
    ]);
    // One marker per painted row, and both rows of the path.
    expect(markerOffsets).toEqual([-2, -1]);

    second.leave?.({} as MouseEvent, second.text);
    expect(decorations).toHaveLength(0);
  });
});

describe("registerFilePathLinks behaviour", () => {
  it("underlines a path the checkout holds and leaves the rest of the line alone", async () => {
    await write("error in src/lib/foo.ts, see src/lib/bar.ts\r\n");
    probe.mockImplementation(async (_checkoutId, path) => (path === "src/lib/foo.ts" ? { path } : null));
    mount();

    const links = await linksOn(1);

    expect(links?.map((link) => link.text)).toEqual(["src/lib/foo.ts"]);
    expect(links?.[0].decorations).toEqual({ pointerCursor: true, underline: true });
  });

  it("offers nothing at all for a line that mentions no file it holds", async () => {
    await write("could not read src/lib/missing.ts\r\n");
    mount();
    expect(await linksOn(1)).toBeUndefined();
  });

  it("asks the backend once per path while the answer is still fresh", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    await linksOn(1);
    await linksOn(1);

    expect(probe).toHaveBeenCalledTimes(1);
  });

  it("opens nothing on a plain click and the file on ctrl+click", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(1))!;
    link.activate({} as MouseEvent, link.text);
    expect(open).not.toHaveBeenCalled();

    link.activate({ ctrlKey: true } as MouseEvent, link.text);
    link.activate({ metaKey: true } as MouseEvent, link.text);
    expect(open).toHaveBeenCalledTimes(2);
    expect(open).toHaveBeenCalledWith("src/lib/foo.ts");
  });

  it("opens the path the backend confirmed rather than the one the line printed", async () => {
    await write("./src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(1))!;
    link.activate({ ctrlKey: true } as MouseEvent, link.text);

    expect(open).toHaveBeenCalledWith("src/lib/foo.ts");
  });

  it("takes the paint back on leave", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [link] = (await linksOn(1))!;
    link.hover?.({} as MouseEvent, link.text);
    expect(decorations).toHaveLength(1);

    link.leave?.({} as MouseEvent, link.text);

    expect(decorations).toHaveLength(0);
  });

  it("takes the provider and the paint back when it is disposed", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    const links = mount();
    const [link] = (await linksOn(1))!;
    link.hover?.({} as MouseEvent, link.text);

    links.dispose();

    expect(provider).toBeNull();
    expect(decorations).toHaveLength(0);
  });

  it("answers nothing once it is disposed, rather than painting a terminal that is gone", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
      return { path: "src/lib/foo.ts" };
    });
    const links = mount();

    // The callback is deliberately never called once disposed, so this cannot await it. What is
    // asserted is that nothing was painted and that the answer stayed unsent, which is the same
    // thing the panel closing means: there is nothing left to answer.
    let settled = false;
    const pending = linksOn(1).then(() => {
      settled = true;
    });
    links.dispose();
    await new Promise((resolve) => setTimeout(resolve, 5));

    expect(settled).toBe(false);
    expect(decorations).toHaveLength(0);
    void pending;
  });

  it("treats a failed probe as a path it cannot open", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockRejectedValue(new Error("ipc is down"));
    mount();
    expect(await linksOn(1)).toBeUndefined();
  });

  it("drops its oldest answers rather than growing without bound", async () => {
    for (let index = 0; index < 260; index += 1) {
      await write(`file-${index}.ts\r\n`);
    }
    probe.mockResolvedValue(null);
    mount();

    for (let index = 0; index < 260; index += 1) {
      await linksOn(index + 1);
    }
    // 260 distinct paths, and the map that holds them does not grow past its bound. The bound is
    // not observable from here directly, so what is asserted is that the terminal still answers
    // for the path that was cached first, which means the map dropped it and re-probed.
    probe.mockResolvedValue({ path: "file-0.ts" });
    expect((await linksOn(1))?.[0].text).toBe("file-0.ts");
    expect(probe.mock.calls.filter(([, path]) => path === "file-0.ts")).toHaveLength(2);
  });

  it("asks once for a path two lines on the same screen both mention", async () => {
    await write("src/lib/foo.ts\r\nand again src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    mount();

    const [first] = (await linksOn(1))!;
    // Hovering while the probe is in flight is what the second line does, and it must join the
    // one that is already running rather than start another.
    const pending = linksOn(2);
    first.hover?.({} as MouseEvent, first.text);
    await pending;

    expect(probe).toHaveBeenCalledTimes(1);
  });
});
