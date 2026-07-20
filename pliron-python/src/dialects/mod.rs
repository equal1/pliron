//! Python bindings for the dialects shipped with pliron itself.
//!
//! Each dialect module instantiates the wrapper classes for that dialect's
//! ops/types/attributes by invoking the `__pliron_reflect_*` token-export
//! macros that pliron-derive emits at pliron's crate root (see
//! pliron-derive's `reflect` module and [`crate::derive`]).

pub mod builtin;
