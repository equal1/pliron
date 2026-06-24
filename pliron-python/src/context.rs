//! [`PyContext`] — the Python-side owner of the pliron compiler context.

use pyo3::prelude::*;

use std::boxed::Box;

use super::{clear_active_ctx, set_active_ctx};
use ::pliron::context::Context;

/// A pliron compiler context.
///
/// Must be used as a context manager so that IR objects can resolve the active
/// context implicitly:
///
/// ```python
/// with pliron.Context() as ctx:
///     # build IR here
///     ...
/// ```
///
/// Only one context may be active per OS thread at a time.
#[pyclass(unsendable, name = "Context")]
pub struct PyContext {
    /// Boxed context so we have a stable address for the thread-local pointer.
    ctx: Option<Box<Context>>,
}

impl Default for PyContext {
    fn default() -> Self {
        Self::new()
    }
}

#[pymethods]
impl PyContext {
    /// Create a new (inactive) pliron context.
    #[new]
    pub fn new() -> Self {
        PyContext {
            ctx: Some(Box::new(Context::new())),
        }
    }

    /// Activate this context (called by `with` statement entry).
    ///
    /// Returns `self` so callers can write `with pliron.Context() as ctx:`.
    fn __enter__(mut slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        let ptr: *mut Context = slf
            .ctx
            .as_mut()
            .ok_or_else(|| super::PlironError::new_err("Context has already been consumed"))?
            .as_mut();
        set_active_ctx(ptr)?;
        Ok(slf)
    }

    /// Deactivate this context (called by `with` statement exit).
    ///
    /// Always returns `False` so any in-flight Python exception propagates.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _exc_tb=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_val: Option<&Bound<'_, PyAny>>,
        _exc_tb: Option<&Bound<'_, PyAny>>,
    ) -> bool {
        clear_active_ctx();
        false
    }

    /// Return `True` if the IR in this context contains no operations,
    /// basic blocks, or regions.
    fn is_ir_empty(&self) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        Ok(ctx.is_ir_empty())
    }
}
