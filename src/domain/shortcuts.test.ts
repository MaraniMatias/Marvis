import { describe, expect, it } from "vitest";
import { carriesAppModifier, isMacPlatform } from "./shortcuts";

describe("the platform's shortcut modifier", () => {
  it("is Cmd on a Mac and Ctrl everywhere else", () => {
    // The two are not two spellings of one chord: on a Mac Ctrl is the Control key, and the
    // terminal answers to it — Ctrl+N walks back through the history — so a shortcut that takes
    // both takes a key that was already spoken for.
    expect(carriesAppModifier({ metaKey: true, ctrlKey: false }, "MacIntel")).toBe(true);
    expect(carriesAppModifier({ metaKey: false, ctrlKey: true }, "MacIntel")).toBe(false);
    expect(carriesAppModifier({ metaKey: false, ctrlKey: true }, "X11; Darwin arm64")).toBe(false);

    expect(carriesAppModifier({ metaKey: false, ctrlKey: true }, "Linux x86_64")).toBe(true);
    expect(carriesAppModifier({ metaKey: true, ctrlKey: false }, "Linux x86_64")).toBe(false);

    expect(carriesAppModifier({ metaKey: false, ctrlKey: true }, "Win32")).toBe(true);
    expect(carriesAppModifier({ metaKey: true, ctrlKey: false }, "Win32")).toBe(false);
  });

  it("reads this window's own platform when it is not told which one to read", () => {
    // No argument is the app's own call, so what it answers is a fact about the machine this
    // window is on rather than one passed in by whoever is asking. The platform is enough on its
    // own, which is what lets a test say which machine it is on.
    expect(carriesAppModifier({ metaKey: true, ctrlKey: false })).toBe(isMacPlatform(navigator.platform));
  });

  it("answers nothing for a key carrying no modifier at all", () => {
    for (const platform of ["MacIntel", "Linux x86_64"]) {
      expect(carriesAppModifier({ metaKey: false, ctrlKey: false }, platform)).toBe(false);
    }
  });

  it("reads the platform rather than the user agent's opinion of it", () => {
    // The two halves disagree on the same machine: an older WebKit names `MacIntel` and a current
    // one on Apple silicon names `X11; Darwin arm64`, and only the second is caught by its kernel.
    expect(isMacPlatform("MacIntel")).toBe(true);
    expect(isMacPlatform("macOS")).toBe(true);
    expect(isMacPlatform("X11; Darwin arm64")).toBe(true);
    expect(isMacPlatform("MacIntel Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe(true);

    // Linux is the same X11 without the Darwin, which is exactly the pair that has to differ.
    expect(isMacPlatform("X11; Linux x86_64 Mozilla/5.0 (X11; Linux x86_64)")).toBe(false);
    expect(isMacPlatform("Linux x86_64")).toBe(false);
    expect(isMacPlatform("Win32 Mozilla/5.0 (Windows NT 10.0; Win64; x64)")).toBe(false);
  });
});
