//! Compile- and render-time options (§8.1). Only the fields needed by the
//! templating layer are wired so far; later phases extend these structs.

use std::collections::HashMap;

use minijinja::UndefinedBehavior;

/// How undefined / missing values are handled during render (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingPolicy {
    /// Undefined variable or out-of-range access raises a render error (default).
    #[default]
    Strict,
    /// Undefined renders as empty string (Jinja lenient printing).
    Empty,
}

impl MissingPolicy {
    /// Map to the MiniJinja behavior that implements this policy.
    pub fn undefined_behavior(self) -> UndefinedBehavior {
        match self {
            MissingPolicy::Strict => UndefinedBehavior::Strict,
            MissingPolicy::Empty => UndefinedBehavior::Lenient,
        }
    }
}

/// Options controlling [`crate::compile`].
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Named partial sources for `{% include %}` / `{% import %}` (§2.8).
    pub partials: HashMap<String, String>,
    /// Undefined/missing handling (§2.9).
    pub missing_policy: MissingPolicy,
    /// Maximum include/import/macro recursion depth (§2.8). 0 selects the default.
    pub include_max_depth: u32,
}

impl CompileOptions {
    /// The effective recursion depth, applying the default when unset.
    pub fn effective_depth(&self) -> u32 {
        if self.include_max_depth == 0 {
            DEFAULT_INCLUDE_DEPTH
        } else {
            self.include_max_depth
        }
    }
}

/// Default `{% include %}`/macro recursion depth (§2.8).
pub const DEFAULT_INCLUDE_DEPTH: u32 = 64;
