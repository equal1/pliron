//! Python bindings for [`crate::irbuild`], exposed as the `pliron.irbuild`
//! Python submodule.
//!
//! Layout convention: each folder under `src/python/` is one Python submodule,
//! populated flat by its files; the folder's `mod.rs` builds it via
//! [`register`].

pub mod cloning;
pub mod inserter;
pub mod listener;
pub mod rewriter;

use pyo3::prelude::*;

use crate::irbuild::IRStatus;

/// Build the `pliron.irbuild` submodule and register this folder's classes
/// into it. Called by `register_core_types`.
pub fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = crate::python::get_or_create_submodule(parent, "irbuild")?;
    m.add_class::<PyIRStatus>()?;
    m.add_class::<inserter::PyOpInsertionPoint>()?;
    m.add_class::<inserter::PyBlockInsertionPoint>()?;
    m.add_class::<inserter::PyIRInserter>()?;
    m.add_class::<rewriter::PyIRRewriter>()?;
    m.add_class::<cloning::PyIrMapping>()?;
    m.add_function(wrap_pyfunction!(cloning::clone_operation, &m)?)?;
    m.add_function(wrap_pyfunction!(cloning::clone_region_into, &m)?)?;
    m.add_function(wrap_pyfunction!(cloning::clone_blocks_into, &m)?)?;
    m.add_class::<listener::PyDummyListener>()?;
    m.add_class::<listener::PyRecorder>()?;
    Ok(())
}

/// Python binding for [`IRStatus`], exposed as `pliron.irbuild.IRStatus`.
///
/// It is truthy when the IR was changed, so it works both as an enum and as a
/// plain condition:
///
/// ```python
/// status = ...                            # any API returning an IRStatus
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
