//! Panic containment. Bluey builds with `panic = "unwind"` only so a crashing
//! third-party parser can be refused instead of taking the app down (CRIT-002);
//! every other panic stays fatal. The app's panic hook asks [`is_contained`]
//! and aborts the process unless the panic happened inside [`contain`].

use std::cell::Cell;
use std::panic::UnwindSafe;

thread_local! {
    /// How many [`contain`] calls are running on this thread.
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
}

/// Run `f`, turning a panic into `Err` instead of aborting the app.
pub fn contain<T>(f: impl FnOnce() -> T + UnwindSafe) -> std::thread::Result<T> {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            CONTAINED.with(|depth| depth.set(depth.get() - 1));
        }
    }
    CONTAINED.with(|depth| depth.set(depth.get() + 1));
    let _leave = Leave;
    std::panic::catch_unwind(f)
}

/// Whether a panic on the current thread would be caught by [`contain`].
pub fn is_contained() -> bool {
    CONTAINED.with(|depth| depth.get() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_code_inside_contain_is_contained() {
        assert!(!is_contained());
        let inside = contain(is_contained).unwrap();
        assert!(inside);
        assert!(!is_contained(), "left after the call returns");
    }

    #[test]
    fn a_contained_panic_is_an_error_and_leaves_the_scope() {
        let result = contain(|| -> u8 { panic!("parser bug") });
        assert!(result.is_err());
        assert!(!is_contained(), "left after unwinding");
        assert!(contain(|| contain(|| 1)).unwrap().is_ok(), "nests");
    }
}
