//! The pending outage of native code (recoverable failures TE-3).
//!
//! A runtime entry point that meets a failure either stops the program,
//! as it always has, or, when the failure is an outage and the program
//! can catch one, holds it here and returns zero bits. Compiled code
//! tests the slot after each call that can fail: it unwinds through its
//! callers by their ordinary returns until a `try ... else` takes the
//! outage, or the generated entry point finds it and stops the program
//! with the message it would have printed at the failing call.
//!
//! A program with no `try` that can catch an outage never turns holding
//! on, so it stops at the failing call exactly as before. See
//! `docs/proposals/native-recoverable-failures.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static CATCHING: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<String>> = Mutex::new(None);

fn pending() -> std::sync::MutexGuard<'static, Option<String>> {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A failure at a runtime entry point. `message` is the full line the
/// program prints if it stops. An outage is held when the program can
/// catch one, and the caller then returns zero bits; anything else, and
/// any outage in a program that cannot catch it, stops the program now.
pub(crate) fn fail(message: String, outage: bool) {
    if outage && CATCHING.load(Ordering::Relaxed) {
        let mut slot = pending();
        if slot.is_none() {
            *slot = Some(message);
        }
        return;
    }
    eprintln!("{message}");
    std::process::exit(1);
}

/// Called once by the generated entry point of a program that has a
/// `try` able to catch an outage.
#[no_mangle]
pub extern "C" fn corvid_outage_catching() {
    CATCHING.store(true, Ordering::Relaxed);
}

/// 1 when an outage is unwinding, 0 otherwise.
#[no_mangle]
pub extern "C" fn corvid_outage_pending() -> i64 {
    i64::from(pending().is_some())
}

/// A `try` took the outage: forget it.
#[no_mangle]
pub extern "C" fn corvid_outage_clear() {
    pending().take();
}

/// An outage that nothing caught: stop with the failing call's message,
/// as the program would have stopped at the call itself. Called by the
/// generated entry points after the program returns, and after a call
/// whose position cannot unwind.
#[no_mangle]
pub extern "C" fn corvid_outage_stop_if_pending() {
    if let Some(message) = pending().take() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

/// The deadlines of the `try ... timeout` attempts native code is inside,
/// innermost last, with the milliseconds each was given (TE-3e). Native
/// code runs on one thread, so the stack is that thread's.
struct Deadline {
    at: std::time::Instant,
    milliseconds: u64,
}

thread_local! {
    static DEADLINES: std::cell::RefCell<Vec<Deadline>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Start an attempt that must finish within `milliseconds`. A nested
/// deadline never extends an outer one: the nearer of the two applies.
#[no_mangle]
pub extern "C" fn corvid_deadline_begin(milliseconds: i64) {
    let milliseconds = milliseconds.max(0) as u64;
    DEADLINES.with(|stack| {
        let mut stack = stack.borrow_mut();
        let mut at = std::time::Instant::now() + std::time::Duration::from_millis(milliseconds);
        let mut reported = milliseconds;
        if let Some(outer) = stack.last() {
            if outer.at < at {
                at = outer.at;
                reported = outer.milliseconds;
            }
        }
        stack.push(Deadline { at, milliseconds: reported });
    });
}

/// End the innermost attempt's deadline. Compiled code ends every attempt
/// it begins, on each path out of it; a `try ... timeout` whose body
/// could leave by `?` is refused natively.
#[no_mangle]
pub extern "C" fn corvid_deadline_end() {
    DEADLINES.with(|stack| {
        stack.borrow_mut().pop();
    });
}

/// Run `call` on the runtime's tokio handle within the innermost
/// deadline, if any: a call that does not finish in time is
/// `RuntimeError::TimedOut`, an outage.
pub(crate) fn within_deadline<T>(
    handle: &tokio::runtime::Handle,
    call: impl std::future::Future<Output = Result<T, crate::RuntimeError>>,
) -> Result<T, crate::RuntimeError> {
    let deadline = DEADLINES.with(|stack| stack.borrow().last().map(|d| (d.at, d.milliseconds)));
    match deadline {
        None => handle.block_on(call),
        Some((at, milliseconds)) => {
            let left = at.saturating_duration_since(std::time::Instant::now());
            handle
                .block_on(async move { tokio::time::timeout(left, call).await })
                .unwrap_or(Err(crate::RuntimeError::TimedOut { milliseconds }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outage_is_held_only_while_catching_and_a_clear_forgets_it() {
        // The exit paths cannot run in a unit test; holding and clearing can.
        CATCHING.store(true, Ordering::Relaxed);
        fail("corvid: `lookup`: first".into(), true);
        fail("corvid: `lookup`: second".into(), true);
        assert_eq!(corvid_outage_pending(), 1);
        assert_eq!(pending().as_deref(), Some("corvid: `lookup`: first"), "the first outage is the one reported");
        corvid_outage_clear();
        assert_eq!(corvid_outage_pending(), 0);
        CATCHING.store(false, Ordering::Relaxed);
    }

    #[test]
    fn a_nested_deadline_never_extends_the_outer_one_and_ending_restores_it() {
        corvid_deadline_begin(50);
        corvid_deadline_begin(10_000);
        DEADLINES.with(|stack| {
            let stack = stack.borrow();
            assert_eq!(stack[1].milliseconds, 50, "the nearer deadline applies");
            assert!(stack[1].at <= stack[0].at);
        });
        corvid_deadline_end();
        DEADLINES.with(|stack| assert_eq!(stack.borrow().len(), 1));
        corvid_deadline_end();
        DEADLINES.with(|stack| assert!(stack.borrow().is_empty()));
    }
}
