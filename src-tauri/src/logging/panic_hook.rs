//! The panic hook. Bluey unwinds on panic only so a crashing document parser
//! can be refused (`bluey_core::panic`); any other panic — in a Tauri command,
//! a streaming task, the helper supervisor — must not leave the app running
//! half-updated with nothing in the log (a Finder-launched app discards
//! stderr). The hook writes where it happened to the log file, then aborts,
//! which leaves a macOS crash report as `panic = "abort"` did.
//!
//! The payload is never logged: a panic message can quote document text.

/// Install the hook; call once, after [`super::Logging::init`].
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "unknown".to_string(), ToString::to_string);
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("unnamed");
        if bluey_core::panic::is_contained() {
            tracing::warn!(%location, thread, "contained panic");
            return;
        }
        tracing::error!(%location, thread, "panic; aborting");
        previous(info);
        std::process::abort();
    }));
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;

    const CHILD_ENV: &str = "BLUEY_PANIC_HOOK_CHILD";
    const SIGABRT: i32 = 6;

    /// Runs only inside the child process `panics_abort_unless_contained`
    /// spawns: the hook is process-wide and ends the process.
    #[test]
    fn panic_hook_child() {
        let Some(dir) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let _logging = super::super::Logging::init(dir.into(), "info");
        super::install();
        assert!(bluey_core::panic::contain(|| panic!("secret text")).is_err());
        let _ = std::thread::spawn(|| panic!("secret text")).join();
        std::process::exit(0);
    }

    #[test]
    fn panics_abort_unless_contained() {
        let dir = std::env::temp_dir().join(bluey_core::new_id("bluey-panic-test"));
        std::fs::create_dir_all(&dir).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "logging::panic_hook::tests::panic_hook_child"])
            .args(["--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, &dir)
            .status()
            .unwrap();

        assert_eq!(status.signal(), Some(SIGABRT), "{status}");
        let log = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert!(log.contains("contained panic"), "{log}");
        assert!(log.contains("panic; aborting"), "{log}");
        assert!(log.contains("panic_hook.rs"), "the location is logged");
        assert!(!log.contains("secret text"), "the payload is not");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
