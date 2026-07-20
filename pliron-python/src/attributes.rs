//! [`PyAttribute`] — Python wrapper for a pliron attribute (`AttrObj`).

use pyo3::{ffi::Py_hash_t, prelude::*};

use std::{format, string::String};

use ::pliron::{
    attribute::{AttrId, AttrName, AttrObj},
    dialect::DialectName,
    printable::Printable,
};

#[pyclass(unsendable, name = "Attribute")]
pub struct PyAttrId {
    pub inner: AttrId,
}

#[pymethods]
impl PyAttrId {
    pub fn __init__(&mut self, dialect: &str, name: &str) -> PyResult<()> {
        let dialect = DialectName::new(dialect);
        let name = AttrName::new(name);

        self.inner = AttrId { dialect, name };
        Ok(())
    }

    fn __str__(&self) -> PyResult<String> {
        Ok(format!("{}", self.inner))
    }

    fn dialect(&self) -> PyResult<String> {
        Ok(format!("{}", self.inner.dialect))
    }

    fn name(&self) -> PyResult<String> {
        Ok(format!("{}", self.inner.name))
    }
}

/// A pliron attribute value (non-SSA data attached to operations).
///
/// Attributes are not uniqued; each `Attribute` object is an owned clone.
#[pyclass(unsendable, name = "Attribute")]
pub struct PyAttribute {
    pub inner: AttrObj,
}

#[pymethods]
impl PyAttribute {
    /// The fully-qualified name of this attribute type, e.g. `"builtin.integer"`.
    fn attr_name(&self) -> String {
        format!("{}", self.inner.get_attr_id())
    }

    /// Clone this attribute, returning a new independent copy.
    fn clone_attr(&self) -> PyAttribute {
        PyAttribute {
            inner: ::pliron::dyn_clone::clone_box(&*self.inner),
        }
    }

    fn get_attr_id(&self) -> PyAttrId {
        PyAttrId {
            inner: self.inner.get_attr_id(),
        }
    }

    // ------------------------------------------------------------------
    // Printing / verification
    // ------------------------------------------------------------------

    fn __str__(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", self.inner.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }

    fn verify(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        ::pliron::attribute::verify_attr(&*self.inner, ctx).map_err(super::to_py_err)
    }

    fn __eq__(&self, other: &PyAttribute) -> PyResult<bool> {
        Ok(self.inner.eq_attr(&*other.inner))
    }

    fn __hash__(&self) -> PyResult<Py_hash_t> {
        use core::hash::{Hash, Hasher};
        let ctx = super::get_ctx()?;
        let mut h = rustc_hash::FxHasher::default();
        // Consistent with `__eq__`/`eq_attr`: structurally-equal attributes
        // print identically, so hashing the canonical text respects the
        // Python invariant `a == b => hash(a) == hash(b)`.
        format!("{}", self.inner.disp(ctx)).hash(&mut h);
        Ok(h.finish() as Py_hash_t)
    }

    fn __ne__(&self, other: &PyAttribute) -> PyResult<bool> {
        Ok(!self.inner.eq_attr(&*other.inner))
    }
}
