//! [`PyType`] — Python wrapper for a pliron type (`TypeHandle`).

use pyo3::{ffi::Py_hash_t, prelude::*};

use alloc::{format, string::String, vec::Vec};

use crate::{common_traits::Verify, printable::Printable, r#type::TypeHandle};

/// A handle to a uniqued pliron type.
///
/// Types are globally uniqued and immutable after creation.
/// Two `Type` objects are equal if and only if they point to the same unique instance.
#[pyclass(unsendable, name = "Type")]
pub struct PyType {
    pub ptr: TypeHandle,
}

#[pymethods]
impl PyType {
    /// The fully-qualified name of this type, e.g. `"llvm.integer"`.
    fn type_name(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", self.ptr.deref(ctx).get_type_id()))
    }

    // ------------------------------------------------------------------
    // Printing / verification
    // ------------------------------------------------------------------

    fn __str__(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", self.ptr.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }

    fn verify(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.verify(ctx).map_err(super::to_py_err)
    }

    // ------------------------------------------------------------------
    // Identity — types are uniqued, so pointer equality is semantic equality
    // ------------------------------------------------------------------

    fn __eq__(&self, other: &PyType) -> bool {
        self.ptr == other.ptr
    }

    fn __hash__(&self) ->  PyResult<Py_hash_t>  {
        let ctx = super::get_ctx()?;
        Ok(u64::from(self.ptr.deref(ctx).hash_type()) as Py_hash_t)
    }

}

/// Coerce any Python "type" object into a [`TypeHandle`].
///
/// Accepts either the generic [`PyType`] or any derive-generated typed wrapper
/// (e.g. `PyIntegerType`). Typed wrappers are projected to the generic handle via
/// their generated `to_type()` method. This mirrors how [`set_attribute`] accepts
/// either a generic `PyAttribute` or a typed attribute via `into_attr()`.
///
/// [`set_attribute`]: super::operation::PyOperation
pub fn type_handle_from_any(obj: &Bound<'_, PyAny>) -> PyResult<TypeHandle> {
    if let Ok(ty) = obj.extract::<PyRef<'_, PyType>>() {
        return Ok(ty.ptr);
    }
    let generic = obj.call_method0("to_type")?;
    let ty = generic.extract::<PyRef<'_, PyType>>()?;
    Ok(ty.ptr)
}

/// Coerce a list of Python "type" objects into [`TypeHandle`]s.
///
/// Convenience wrapper over [`type_handle_from_any`] for the common case of a
/// list-of-types parameter (function signatures, block argument types).
pub fn type_handles_from_any(objs: &[Bound<'_, PyAny>]) -> PyResult<Vec<TypeHandle>> {
    objs.iter().map(type_handle_from_any).collect()
}
