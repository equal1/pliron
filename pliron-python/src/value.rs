//! [`PyValue`] — Python wrapper for [`::pliron::value::Value`].

use pyo3::prelude::*;

use std::{format, string::String, vec::Vec};

use ::pliron::{
    common_traits::Verify,
    printable::Printable,
    r#type::Typed,
    value::{DefiningEntity, Value},
};

use super::{basic_block::PyBasicBlock, operation::PyOperation, types::PyType};

// #[cfg(feature = "python")]
// #[pyclass(eq, eq_int, name = "DefiningEntity")]
// pub enum PyDefiningEntity {
//     Op(PyOperation),
//     Block(PyBasicBlock),
// }

/// A handle to a pliron SSA value (either an op result or a block argument).
#[pyclass(unsendable, name = "Value")]
pub struct PyValue {
    pub val: Value,
}

#[pymethods]
impl PyValue {
    // ------------------------------------------------------------------
    // Type
    // ------------------------------------------------------------------

    /// The type of this value.
    fn get_type(&self) -> PyResult<PyType> {
        let ctx = super::get_ctx()?;
        Ok(PyType {
            ptr: self.val.get_type(ctx),
        })
    }

    // ------------------------------------------------------------------
    // Variant queries
    // ------------------------------------------------------------------

    /// Return `True` if this value is an operation result.
    fn is_op_result(&self) -> bool {
        matches!(self.val.defining_entity(), DefiningEntity::Op(_))
    }

    /// Return `True` if this value is a block argument.
    fn is_block_argument(&self) -> bool {
        matches!(self.val.defining_entity(), DefiningEntity::Block(_))
    }

    /// For an op-result value, return the defining operation.  `None` for block arguments.
    fn defining_op(&self) -> PyResult<Option<PyOperation>> {
        Ok(self.val.defining_op().map(|ptr| PyOperation { ptr }))
    }

    /// For a block-argument value, return the defining block.  `None` for op results.
    fn defining_block(&self) -> PyResult<Option<PyBasicBlock>> {
        Ok(self.val.defining_block().map(|ptr| PyBasicBlock { ptr }))
    }

    // ------------------------------------------------------------------
    // Use-def
    // ------------------------------------------------------------------

    /// Number of uses of this value.
    fn num_uses(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.val.num_uses(ctx))
    }

    /// `True` if this value has at least one use.
    fn is_used(&self) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        Ok(self.val.is_used(ctx))
    }

    /// All operations that use this value, returned as a list of [`PyOperation`].
    fn users(&self) -> PyResult<Vec<PyOperation>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .val
            .uses(ctx)
            .into_iter()
            .map(|u| PyOperation { ptr: u.user_op() })
            .collect())
    }

    /// Replace all uses of this value with `other`.
    fn replace_all_uses_with(&self, other: &PyValue) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.val.replace_all_uses_with(ctx, &other.val);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Printing / verification
    // ------------------------------------------------------------------

    fn __str__(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", self.val.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }

    fn verify(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.val.verify(ctx).map_err(super::to_py_err)
    }

    // ------------------------------------------------------------------
    // Identity
    // ------------------------------------------------------------------

    fn __eq__(&self, other: &PyValue) -> bool {
        self.val == other.val
    }

    fn __hash__(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.val.hash(&mut h);
        h.finish() as usize
    }
}
