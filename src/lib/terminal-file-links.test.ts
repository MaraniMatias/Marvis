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
const probe = vi.hoisted(() =>
  vi.fn<(checkoutId: string, path: string, workingDirectory?: string) => Promise<{ path: string } | null>>(),
);

vi.mock("./ipc", () => ({ probeCheckoutFile: probe }));

let terminal: Headless;
let provider: ILinkProvider | null;
let decorations: { x: number; width: number; foregroundColor?: string }[] = [];
let markerOffsets: number[] = [];
let open: ReturnType<typeof vi.fn<(path: string) => void>>;
let openUrl: ReturnType<typeof vi.fn<(url: string) => void>>;
/** The directory the fake terminal is sitting in, which is what a bare name is read against. */
let shellDirectory: string | undefined;

async function write(text: string) {
  await new Promise<void>((resolve) => terminal.write(text, resolve));
}

/** Puts the links on a real terminal, with the drawing calls recorded rather than performed. */
function mount() {
  provider = null;
  decorations = [];
  markerOffsets = [];
  open = vi.fn<(path: string) => void>();
  openUrl = vi.fn<(url: string) => void>();
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
  return registerFilePathLinks(terminal as never, {
    get checkoutId() {
      return "checkout:one";
    },
    get workingDirectory() {
      return shellDirectory;
    },
    open,
    openUrl,
  });
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
  shellDirectory = undefined;
  document.documentElement.style.setProperty("--muster-accent", "#74ade8");
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

    // One link over both rows, because xterm underlines every row a link's range spans and a
    // decoration here would clear that underline rather than add to it.
    expect(links).toHaveLength(1);
    // The path is 28 characters and the row is 20 wide, so it fills the first row and takes 8
    // columns of the second.
    expect(links[0].range).toEqual({ start: { x: 1, y: 1 }, end: { x: 8, y: 2 } });
    expect(links[0].text).toBe("verylongdirectory/src/app.ts");
  });

  it("registers no decoration, because that is what cleared xterm's own underline", async () => {
    // Registering one fires `onDecorationRegistered`, which makes xterm clear and repaint the
    // screen, and the repaint drops the underline it drew for the hovered link. A hovered path came
    // out recoloured and not underlined. Nothing here paints, so nothing can clear it.
    terminal = new Headless({ allowProposedApi: true, cols: 20, rows: 5 });
    await write("");
    await write("verylongdirectory/src/app.ts\r\n");
    probe.mockResolvedValue({ path: "verylongdirectory/src/app.ts" });
    mount();

    const [link] = (await linksOn(2))!;
    link.hover?.({} as MouseEvent, link.text);

    expect(decorations).toHaveLength(0);
    expect(markerOffsets).toEqual([]);
    // xterm underlines the link itself, which is the only underline there is now.
    expect(link.decorations).toEqual({ pointerCursor: true, underline: true });
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

  /// The listing: `ls` prints names with no directory on them, so the backend is told where the
  /// shell is, because that is what a bare name is relative to.
  it("asks about a bare name against the directory the shell is in", async () => {
    await write("LICENSE   src/lib/foo.ts\r\n");
    shellDirectory = "/work/plain/src/lib";
    probe.mockImplementation(async (_checkoutId, path, directory) =>
      path === "LICENSE" && directory === "/work/plain/src/lib" ? { path } : null,
    );
    mount();

    const [link] = (await linksOn(1))!;

    expect(link.text).toBe("LICENSE");
    expect(probe).toHaveBeenCalledWith("checkout:one", "LICENSE", "/work/plain/src/lib");
    link.activate({ ctrlKey: true } as MouseEvent, link.text);
    expect(open).toHaveBeenCalledWith("LICENSE");
  });

  it("asks again once the shell has moved, rather than answering from where it was", async () => {
    await write("LICENSE\r\n");
    probe.mockImplementation(async (_checkoutId, path) => ({ path }));
    shellDirectory = "/work/plain/src";
    mount();
    expect((await linksOn(1))?.[0].text).toBe("LICENSE");

    // The same name in another directory is another file, so an answer cached for the directory
    // the shell has just left is not the answer to this one.
    shellDirectory = "/work/plain/docs";
    const links = registerFilePathLinks(terminal as never, {
      checkoutId: "checkout:one",
      get workingDirectory() {
        return shellDirectory;
      },
      open,
      openUrl,
    });
    await linksOn(1);

    expect(probe.mock.calls.filter(([, , directory]) => directory === "/work/plain/docs")).toHaveLength(1);
    expect(probe.mock.calls.filter(([, , directory]) => directory === "/work/plain/src")).toHaveLength(1);
    links.dispose();
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

  it("takes the provider back when it is disposed", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockResolvedValue({ path: "src/lib/foo.ts" });
    const links = mount();
    await linksOn(1);

    links.dispose();

    expect(provider).toBeNull();
  });

  it("answers nothing once it is disposed, rather than painting a terminal that is gone", async () => {
    await write("src/lib/foo.ts\r\n");
    probe.mockImplementation(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
      return { path: "src/lib/foo.ts" };
    });
    const links = mount();

    // The callback is deliberately never called once disposed, so this cannot await it. What is
    // asserted is that the answer stayed unsent, which is the same thing the panel closing means:
    // there is nothing left to answer.
    let settled = false;
    const pending = linksOn(1).then(() => {
      settled = true;
    });
    links.dispose();
    await new Promise((resolve) => setTimeout(resolve, 5));

    expect(settled).toBe(false);
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

describe("registerFilePathLinks web links", () => {
  it("underlines the address curl printed, without asking the disk about it", async () => {
    await write("curl https://www.example.com/\r\n");
    mount();

    const [link] = (await linksOn(1))!;

    expect(link.text).toBe("https://www.example.com/");
    // The address starts at column 6 and is 24 characters, so it covers columns 6 through 29.
    expect(link.range).toEqual({ start: { x: 6, y: 1 }, end: { x: 29, y: 1 } });
    expect(link.decorations).toEqual({ pointerCursor: true, underline: true });
    // Nothing to confirm: a page is not a file in this checkout, so the one thing that would decide
    // a path never runs for it. The whole line costs no IPC at all.
    expect(probe).not.toHaveBeenCalled();
  });

  it("opens the address on ctrl+click and nothing at all on a plain click", async () => {
    await write("curl https://www.example.com/\r\n");
    mount();

    const [link] = (await linksOn(1))!;
    link.activate({} as MouseEvent, link.text);
    expect(openUrl).not.toHaveBeenCalled();

    link.activate({ ctrlKey: true } as MouseEvent, link.text);
    link.activate({ metaKey: true } as MouseEvent, link.text);
    expect(openUrl).toHaveBeenCalledTimes(2);
    expect(openUrl).toHaveBeenCalledWith("https://www.example.com/");
    // The browser is not the preview's business: neither handler is the other one's.
    expect(open).not.toHaveBeenCalled();
  });

  it("offers the path and the address on a line that prints both", async () => {
    await write("docs https://example.com/guide, see src/lib/foo.ts\r\n");
    probe.mockImplementation(async (_checkoutId, path) => (path === "src/lib/foo.ts" ? { path } : null));
    mount();

    const links = await linksOn(1);

    expect(links?.map((link) => link.text)).toEqual(["https://example.com/guide", "src/lib/foo.ts"]);
  });

  it("leaves alone anything that names a file or an app rather than a page", async () => {
    // The opener on the other end opens `http` and `https` and refuses the rest, so underlining one
    // of these would promise a click that cannot work.
    await write("file:///etc/passwd and vscode://file/tmp/x\r\n");
    mount();

    expect(await linksOn(1)).toBeUndefined();
    // Both are refused here rather than sent over the bridge to be refused there.
    expect(probe).not.toHaveBeenCalled();
  });

  it("takes the addresses back with the provider", async () => {
    await write("curl https://www.example.com/\r\n");
    const links = mount();
    await linksOn(1);

    links.dispose();

    expect(provider).toBeNull();
  });
});
