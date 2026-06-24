//! Python bindings for [`crate::irbuild`].
//!
//! Mirrors the layout of the `irbuild` module in the main tree.

pub mod builder;
pub mod rewriter;

use pyo3::prelude::*;

use crate::irbuild::IRStatus;

/// Python binding for [`IRStatus`], exposed as `pliron.IRStatus`.
///
/// It is truthy when the IR was changed, so it works both as an enum and as a
/// plain condition:
///
/// ```python
/// status = pliron.apply_match_rewrite(root_op, FoldAddZero())
/// if status == pliron.IRStatus.Changed:   # explicit
///     ...
/// if status:                              # truthy shorthand
///     ...
/// ```
#[pyclass(eq, eq_int, name = "IRStatus")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyIRStatus {
    /// The IR was not modified.
    Unchanged,
    /// The IR was modified.
    Changed,
}

impl From<IRStatus> for PyIRStatus {
    fn from(status: IRStatus) -> Self {
        match status {
            IRStatus::Unchanged => PyIRStatus::Unchanged,
            IRStatus::Changed => PyIRStatus::Changed,
        }
    }
}

#[pymethods]
impl PyIRStatus {
    /// `True` if the IR was modified.
    fn is_changed(&self) -> bool {
        matches!(self, PyIRStatus::Changed)
    }

    /// `bool(status)` is `True` iff the IR was changed.
    fn __bool__(&self) -> bool {
        self.is_changed()
    }

    fn __repr__(&self) -> &'static str {
        match self {
            PyIRStatus::Unchanged => "IRStatus.Unchanged",
            PyIRStatus::Changed => "IRStatus.Changed",
        }
    }
}
