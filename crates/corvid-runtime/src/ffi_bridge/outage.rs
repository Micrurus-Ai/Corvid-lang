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
static PENDING: Mutex<Option<Held>> = Mutex::new(None);

/// A held outage: the line it stops the program with, and, for a
/// timeout, the stack position of the deadline that fired.
struct Held {
    message: String,
    deadline: Option<usize>,
}

fn pending() -> std::sync::MutexGuard<'static, Option<Held>> {
    PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A failure at a runtime entry point. `message` is the full line the
/// program prints if it stops. An outage is held when the program can
/// catch one, and the caller then returns zero bits; anything else, and
/// any outage in a program that cannot catch it, stops the program now.
pub(crate) fn fail(message: String, outage: bool) {
    fail_held(message, outage, None);
}

/// [`fail`] for an error the runtime classifies: a timeout remembers the
/// deadline that fired, so only the `try` that set it, or one inside it,
/// catches it.
pub(crate) fn fail_error(message: String, error: &crate::RuntimeError) {
    let deadline = matches!(error, crate::RuntimeError::TimedOut { .. })
        .then(|| DEADLINES.with(|stack| stack.borrow().last().map(|d| d.owner)))
        .flatten();
    fail_held(message, error.is_outage(), deadline);
}

fn fail_held(message: String, outage: bool, deadline: Option<usize>) {
    if outage && CATCHING.load(Ordering::Relaxed) {
        let mut slot = pending();
        if slot.is_none() {
            *slot = Some(Held { message, deadline });
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

/// 1 when an outage is unwinding that the `try` which read `depth` from
/// [`corvid_deadline_depth`] at its start may catch: any outage but a
/// timeout of a deadline set outside it. On the interpreter an outer
/// try's timeout cancels everything inside it, so no inner `try` sees it.
#[no_mangle]
pub extern "C" fn corvid_outage_pending_for(depth: i64) -> i64 {
    i64::from(pending().as_ref().is_some_and(|held| held.deadline.is_none_or(|owner| owner as i64 >= depth)))
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
    if let Some(held) = pending().take() {
        eprintln!("{}", held.message);
        std::process::exit(1);
    }
}

/// The deadlines of the `try ... timeout` attempts native code is inside,
/// innermost last, with the milliseconds each was given (TE-3e). Native
/// code runs on one thread, so the stack is that thread's.
struct Deadline {
    at: std::time::Instant,
    milliseconds: u64,
    /// The stack position of the deadline that sets this one's time: its
    /// own, or an outer one that is nearer.
    owner: usize,
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
        let mut owner = stack.len();
        if let Some(outer) = stack.last() {
            if outer.at < at {
                at = outer.at;
                reported = outer.milliseconds;
                owner = outer.owner;
            }
        }
        stack.push(Deadline { at, milliseconds: reported, owner });
    });
}

/// How many deadlines are active: a `try` reads this when it starts,
/// and [`corvid_outage_pending_for`] compares a timeout's deadline to it.
#[no_mangle]
pub extern "C" fn corvid_deadline_depth() -> i64 {
    DEADLINES.with(|stack| stack.borrow().len() as i64)
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

    /// The tests share the process-wide holding switch and slot, so they
    /// take turns; a `fail` with holding off would exit the test process.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn an_outage_is_held_only_while_catching_and_a_clear_forgets_it() {
        let _serial = serial();
        // The exit paths cannot run in a unit test; holding and clearing can.
        CATCHING.store(true, Ordering::Relaxed);
        fail("corvid: `lookup`: first".into(), true);
        fail("corvid: `lookup`: second".into(), true);
        assert_eq!(corvid_outage_pending(), 1);
        assert_eq!(pending().as_ref().map(|held| held.message.as_str()), Some("corvid: `lookup`: first"), "the first outage is the one reported");
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

    #[test]
    fn an_outer_timeout_is_caught_only_by_the_try_that_set_it() {
        let _serial = serial();
        CATCHING.store(true, Ordering::Relaxed);
        let outer_try = corvid_deadline_depth();
        corvid_deadline_begin(30);
        let inner_try = corvid_deadline_depth();
        corvid_deadline_begin(10_000);
        // The inner call meets the outer deadline, the nearer one.
        fail_error("corvid: `slow`: timed out after 30ms".into(), &crate::RuntimeError::TimedOut { milliseconds: 30 });
        assert_eq!(corvid_outage_pending_for(inner_try), 0, "the inner try does not own it");
        assert_eq!(corvid_outage_pending_for(outer_try), 1, "the outer try does");
        corvid_outage_clear();
        // Any other outage is the inner try's to catch.
        fail("corvid: `lookup`: down".into(), true);
        assert_eq!(corvid_outage_pending_for(inner_try), 1);
        corvid_outage_clear();
        corvid_deadline_end();
        corvid_deadline_end();
        CATCHING.store(false, Ordering::Relaxed);
    }
}
