//! Document/page functions usable in expressions (§2.12). Page-state functions
//! (`page()`, `pages()`, `counter()`) arrive with the pagination phases; this
//! module provides `now()`, whose value is pinned per render for determinism
//! (§3.3, `RenderOptions.now`).

use std::cell::Cell;

use minijinja::value::Value;
use minijinja::{Error, ErrorKind};

thread_local! {
    /// The pinned "now" for the in-flight render on this thread, as a Unix
    /// timestamp in seconds. `None` means the caller did not pin a clock.
    static NOW: Cell<Option<i64>> = const { Cell::new(None) };
}

/// Pin (or clear) the current-render clock for this thread. Rendering is
/// single-threaded per call, so a thread-local keeps `Program: Send + Sync`
/// while giving each render its own deterministic `now()`.
pub fn set_now(ts: Option<i64>) {
    NOW.with(|c| c.set(ts));
}

/// The `now()` template function: returns the pinned timestamp (seconds), or a
/// typed error if the caller did not set `RenderOptions.now`.
pub fn now() -> Result<Value, Error> {
    NOW.with(Cell::get).map(Value::from).ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidOperation,
            "now() requires RenderOptions.now",
        )
    })
}
