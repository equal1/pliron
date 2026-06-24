//! [`PyOperation`] — Python wrapper for [`crate::operation::Operation`].

use pyo3::prelude::*;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    context::Ptr,
    linked_list::LinkedList as _,
    operation::Operation,
    printable::Printable,
    common_traits::Verify,
};

use super::{
    attributes::PyAttribute,
    basic_block::PyBasicBlock,
    region::PyRegion,
    to_py_err,
    value::PyValue,
};

/// A handle to a pliron IR operation.
///
/// All methods resolve the IR data through the active context.
/// Holding a handle after the operation has been erased will cause a panic.
#[pyclass(unsendable, name = "Operation")]
pub struct PyOperation {
    pub ptr: Ptr<Operation>,
}

#[pymethods]
impl PyOperation {
    // ------------------------------------------------------------------
    // Structural inspection
    // ------------------------------------------------------------------

    /// The fully-qualified name of this operation, e.g. `"llvm.add"`.
    fn op_name(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", Operation::get_opid(self.ptr, ctx)))
    }

    /// Number of SSA results produced by this operation.
    fn num_results(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).get_num_results())
    }

    /// Get the `idx`-th result as a [`PyValue`].
    fn get_result(&self, idx: usize) -> PyResult<PyValue> {
        let ctx = super::get_ctx()?;
        Ok(PyValue {
            val: self.ptr.deref(ctx).get_result(idx),
        })
    }

    /// All results as a list of [`PyValue`].
    fn results(&self) -> PyResult<Vec<PyValue>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .results()
            .map(|v| PyValue { val: v })
            .collect())
    }

    /// Number of operands consumed by this operation.
    fn num_operands(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).get_num_operands())
    }

    /// Get the `idx`-th operand as a [`PyValue`].
    fn get_operand(&self, idx: usize) -> PyResult<PyValue> {
        let ctx = super::get_ctx()?;
        Ok(PyValue {
            val: self.ptr.deref(ctx).get_operand(idx),
        })
    }

    /// All operands as a list of [`PyValue`].
    fn operands(&self) -> PyResult<Vec<PyValue>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .operands()
            .map(|v| PyValue { val: v })
            .collect())
    }

    /// Number of regions nested in this operation.
    fn num_regions(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).num_regions())
    }

    /// Get the `idx`-th nested region as a [`PyRegion`].
    fn get_region(&self, idx: usize) -> PyResult<PyRegion> {
        let ctx = super::get_ctx()?;
        Ok(PyRegion {
            ptr: self.ptr.deref(ctx).get_region(idx),
        })
    }

    /// All nested regions as a list of [`PyRegion`].
    fn regions(&self) -> PyResult<Vec<PyRegion>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .regions
            .iter()
            .map(|r| PyRegion { ptr: *r })
            .collect())
    }

    /// Number of CFG successors.
    fn num_successors(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).get_num_successors())
    }

    /// Get the `idx`-th CFG successor block.
    fn get_successor(&self, idx: usize) -> PyResult<PyBasicBlock> {
        let ctx = super::get_ctx()?;
        Ok(PyBasicBlock {
            ptr: self.ptr.deref(ctx).get_successor(idx),
        })
    }

    // ------------------------------------------------------------------
    // Attributes
    // ------------------------------------------------------------------

    /// Get the named attribute, or `None` if not present.
    fn get_attribute(&self, name: &str) -> PyResult<Option<PyAttribute>> {
        let ctx = super::get_ctx()?;
        let id: crate::identifier::Identifier = name.try_into().map_err(|e: crate::result::Error| to_py_err(e))?;
        let op_ref = self.ptr.deref(ctx);
        Ok(op_ref
            .attributes
            .0
            .get(&id)
            .map(|attr| PyAttribute {
                inner: dyn_clone::clone_box(&**attr),
            }))
    }

    /// Return all attribute names as a list of strings.
    fn attribute_names(&self) -> PyResult<Vec<String>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .attributes
            .0
            .keys()
            .map(|k| k.to_string())
            .collect())
    }

    fn has_attribute(&self, name: &str) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        let id: crate::identifier::Identifier = name.try_into().map_err(|e: crate::result::Error| to_py_err(e))?;
        Ok(self.ptr.deref(ctx).attributes.0.contains_key(&id))
    }

    // ------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------

    /// The parent basic block, or `None` if unlinked.
    fn parent_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_block()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    /// The next operation in the parent block's list, or `None`.
    fn next_op(&self) -> PyResult<Option<PyOperation>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_next()
            .map(|ptr| PyOperation { ptr }))
    }

    /// The previous operation in the parent block's list, or `None`.
    fn prev_op(&self) -> PyResult<Option<PyOperation>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_prev()
            .map(|ptr| PyOperation { ptr }))
    }

    // ------------------------------------------------------------------
    // Printing / verification
    // ------------------------------------------------------------------

    /// Return a textual representation of this operation.
    fn __str__(&self) -> PyResult<String> {
        let ctx = super::get_ctx()?;
        Ok(format!("{}", self.ptr.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }

    /// Verify this operation and all its sub-structures.
    ///
    /// # Errors
    /// Raises `PlironError` if verification fails.
    fn verify(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.verify(ctx).map_err(to_py_err)
    }

    // ------------------------------------------------------------------
    // Mutation
    // ------------------------------------------------------------------

    /// Erase this operation.  All result uses must have been removed first.
    fn erase(&self) -> PyResult<()> {
        let ctx = super::get_ctx_mut()?;
        Operation::erase(self.ptr, ctx);
        Ok(())
    }

    /// Insert this operation at the front of `block`.
    fn insert_at_front(&self, block: &super::basic_block::PyBasicBlock) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.insert_at_front(block.ptr, ctx);
        Ok(())
    }

    /// Insert this operation at the back of `block`.
    fn insert_at_back(&self, block: &super::basic_block::PyBasicBlock) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.insert_at_back(block.ptr, ctx);
        Ok(())
    }

    /// Insert this operation after `other`.
    fn insert_after(&self, other: &PyOperation) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.insert_after(ctx, other.ptr);
        Ok(())
    }

    /// Insert this operation before `other`.
    fn insert_before(&self, other: &PyOperation) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        self.ptr.insert_before(ctx, other.ptr);
        Ok(())
    }

    /// Set a named attribute on this operation.
    fn set_attribute(
        &self,
        name: &str,
        attr: &pyo3::Bound<'_, pyo3::PyAny>,
    ) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        let id: crate::identifier::Identifier = name.try_into().map_err(super::to_py_err)?;
        let attr = if let Ok(attr) = attr.extract::<pyo3::PyRef<'_, super::attributes::PyAttribute>>() {
            dyn_clone::clone_box(&*attr.inner)
        } else {
            let attr = attr.call_method0("into_attr")?;
            let attr = attr.extract::<pyo3::PyRef<'_, super::attributes::PyAttribute>>()?;
            dyn_clone::clone_box(&*attr.inner)
        };
        self.ptr.deref_mut(ctx).attributes.0.insert(id, attr);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Identity
    // ------------------------------------------------------------------

    fn __eq__(&self, other: &PyOperation) -> bool {
        self.ptr == other.ptr
    }

    fn __hash__(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.ptr.hash(&mut h);
        h.finish() as usize
    }
}
