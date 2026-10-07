//! Hands a web link to the browser this machine has.
//!
//! The webview cannot follow one: it is not a browser, and a document's `[link](https://…)` is
//! not a thing the window can render. So the URL travels back over the bridge and leaves here,
//! where it is a web address and nothing else.
//!
//! What makes that safe is what this file refuses rather than what it runs. The opener is spawned
//! with the URL as its own argument and no shell in between, so there is nothing to inject into;
//! what a document can reach is a command line. A URL that is not `http` or `https` would be a
//! way to name something else entirely — `file:`, a custom scheme an installed app has registered,
//! anything the platform would rather open than draw — and a URL carrying whitespace or control
//! characters is one the shell's word splitting has no business keeping intact. Both are refused
//! here, and the scheme check is also what guarantees the argument can never start with a `-`, so
//! there is no flag for a document to smuggle through.

use std::process::Command;

/// Long enough for the deepest link anybody writes, short enough that a document cannot hand the
/// platform a megabyte of its own text.
const MAX_URL_CHARS: usize = 2_048;

/// The opener for this platform: what a desktop session already uses to show a web page.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(not(target_os = "macos"))]
const OPENER: &str = "xdg-open";

/// Whether this URL is a web address, and so the only thing this is willing to open.
///
/// An `http` or `https` URL only, with the scheme in front of the `://`: that is what makes the
/// argument unforgeable as anything but a page. The character pass afterwards is what keeps a
/// line break or a null out of the argument, and the length is a bound rather than a rule.
fn is_web_url(url: &str) -> bool {
    if url.len() > MAX_URL_CHARS {
        return false;
    }
    if !url
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        && !url
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
    {
        return false;
    }
    url.chars()
        .all(|character| !character.is_control() && !character.is_whitespace())
}

/// Opens `url` in the browser, or says why it did not.
///
/// A refusal is a fact about the link rather than a failure of the machine, so it is an `Err` the
/// caller can show: a document naming something that is not a web page is something the person
/// reading it has to be told about, not something to swallow.
pub fn open_url(url: &str) -> Result<(), String> {
    if !is_web_url(url) {
        return Err("only an http or https link can be opened in the browser".into());
    }
    Command::new(OPENER)
        .arg(url)
        .status()
        .map(|_| ())
        .map_err(|error| format!("{OPENER} could not open the link: {error}"))
}

#[cfg(test)]
mod tests {
    use super::is_web_url;

    #[test]
    fn web_addresses_are_the_only_ones_it_opens() {
        for url in [
            "https://example.com",
            "https://example.com/a/b?c=d#e",
            "http://127.0.0.1:1420/index.html",
            "HTTPS://EXAMPLE.COM",
        ] {
            assert!(is_web_url(url), "{url} is a web address");
        }
    }

    #[test]
    fn everything_else_is_refused() {
        for url in [
            // A scheme that names a file, an app or the platform rather than a page.
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:someone@example.com",
            "vscode://file/tmp/x",
            // No scheme at all, which is a path and not a URL.
            "/etc/passwd",
            "example.com",
            // A flag smuggled in as the argument.
            "-a",
            // Carrying what a command line is split on.
            "https://example.com/a b",
            "https://example.com\na",
            "https://example.com/a\u{0}b",
            "",
        ] {
            assert!(!is_web_url(url), "{url} was not refused");
        }
    }

    #[test]
    fn a_link_a_document_wrote_does_not_become_an_argument_list() {
        // Long enough to be a bound rather than a rule, and it is the only reason the opener ever
        // waits on a child: everything above the bound is refused before a process is started.
        let url = format!("https://example.com/{}", "a".repeat(super::MAX_URL_CHARS));
        assert!(!is_web_url(&url));
    }
}
