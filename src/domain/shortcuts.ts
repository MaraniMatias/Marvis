/**
 * The modifier this platform's shortcuts are pressed with.
 *
 * macOS is ⌘ and everywhere else is Ctrl, and the two are not two spellings of one thing: on
 * macOS `Ctrl` is the Control key, which the terminal and the editor already answer to. `⌘N` and
 * `Ctrl+N` are the same chord in a Linux terminal panel and two different chords on a Mac, where
 * the second is readline's next-history. A shortcut that accepts both takes that key away from
 * whatever had the focus, which is the one thing a window-wide shortcut must not do.
 *
 * The platform is read at each call rather than once at import, so a window that moves between
 * machines — or a test that says which machine it is — is not answered from a value decided
 * before anybody asked.
 */

/** The part of a key event this decides from: which modifier keys are held down. */
export interface ModifierKey {
  metaKey: boolean;
  ctrlKey: boolean;
}

/** Whether what this window reports names a Mac. Anything else is the Ctrl platform. */
export function isMacPlatform(platform: string): boolean {
  // `Darwin` is in the test because a webview can report `X11; Darwin arm64` where another
  // reports `MacIntel`, and Darwin is the only part of the first that tells macOS apart from the
  // Linux it is otherwise shaped like. Neither string names the other.
  return /mac|darwin/i.test(platform);
}

/**
 * Whether this key carries the modifier a shortcut on this platform is pressed with.
 *
 * `platform` is the string to read, defaulting to what this window reports. It is the platform and
 * not the user agent: a webview that reports one always reports it — `MacIntel` on a Mac whatever
 * its architecture — while the user agent is a document about every browser there is, and reading
 * it would make a test unable to say which machine it is on.
 */
export function carriesAppModifier(event: ModifierKey, platform: string = navigator.platform): boolean {
  return isMacPlatform(platform) ? event.metaKey : event.ctrlKey;
}
