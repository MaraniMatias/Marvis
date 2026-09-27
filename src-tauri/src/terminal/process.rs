//! The name of the program sitting in front of a terminal, read from the OS.
//!
//! Marvis only ever launches a login shell, so every other program in a terminal is one the
//! user typed: `opencode`, `nvim`, `vim`. The PTY knows which process group holds the
//! foreground, and the OS knows what that group is running, so the two together name the
//! program without reading a single byte of terminal output.

use std::path::Path;

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;

    use super::file_name;

    extern "C" {
        /// libproc, part of libSystem: the full path of the executable behind `pid`.
        ///
        /// Returns the bytes written, not counting the terminator, or 0 when the process is
        /// gone or owned by someone else.
        fn proc_pidpath(
            pid: libc::c_int,
            buffer: *mut c_void,
            buffersize: libc::c_int,
        ) -> libc::c_int;
    }

    pub fn executable_name(pid: u32) -> Option<String> {
        let mut buffer = [0_u8; 4096];
        // SAFETY: `buffer` is a live allocation of exactly the length handed over, and
        // libproc writes at most that many bytes into it. A pid we did not ask about, or one
        // that exited, is reported as a failure rather than read.
        let written = unsafe {
            proc_pidpath(
                pid as libc::c_int,
                buffer.as_mut_ptr().cast(),
                buffer.len() as libc::c_int,
            )
        };
        if written <= 0 {
            return None;
        }
        let path = std::str::from_utf8(&buffer[..usize::try_from(written).ok()?.min(buffer.len())])
            .ok()?;
        file_name(path)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::file_name;

    pub fn executable_name(pid: u32) -> Option<String> {
        // The kernel publishes the same name libproc would, truncated to 15 bytes.
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        file_name(comm.trim())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    pub fn executable_name(_pid: u32) -> Option<String> {
        None
    }
}

/// The last path component, which is what a row in the sidebar has room for.
fn file_name(path: &str) -> Option<String> {
    let name = Path::new(path).file_name()?.to_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub use imp::executable_name;

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn only_the_last_component_of_a_path_is_a_program_name() {
        assert_eq!(
            file_name("/Users/matiasmarani/.opencode/bin/opencode").as_deref(),
            Some("opencode")
        );
        assert_eq!(file_name("/bin/zsh").as_deref(), Some("zsh"));
        // A kernel that reports a bare name, and a path with nothing left to name.
        assert_eq!(file_name("vim").as_deref(), Some("vim"));
        assert_eq!(file_name("/"), None);
        assert_eq!(file_name(""), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn names_this_process_and_nobody_at_all() {
        // The strongest claim available without spawning anything: we can name ourselves.
        // Cargo suffixes the test binary, so the stem is the fixed part.
        let named = super::executable_name(std::process::id()).expect("this process has a name");
        assert!(named.starts_with("marvis"), "{named}");
        // u32::MAX is never a live pid, so the OS reports a failure and we say nothing.
        assert_eq!(super::executable_name(u32::MAX), None);
    }
}
