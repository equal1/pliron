//! [`PyBasicBlock`] — Python wrapper for [`crate::basic_block::BasicBlock`].

use pyo3::prelude::*;

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::{
    basic_block::{BasicBlock, BlockArgument}, common_traits::Verify, context::Ptr, identifier::Identifier, linked_list::{ContainsLinkedList, LinkedList}, operation::Operation, printable::Printable, python::types::type_handle_from_any,
};

use super::{operation::PyOperation, region::PyRegion, value::PyValue};

/// A handle to a pliron basic block.
#[pyclass(unsendable, name = "BasicBlock")]
pub struct PyBasicBlock {
    pub ptr: Ptr<BasicBlock>,
}

#[pymethods]
impl PyBasicBlock {
    /// Create a new (detached) basic block.
    ///
    /// `arg_types` is a sequence whose elements are each either the generic
    /// `Type` **or** any concrete type wrapper (e.g. `IntegerType`); every element
    /// is coerced through its `to_type()` projection.
    #[staticmethod]
    #[pyo3(signature = (label, arg_types))]
    fn new(label: Option<&str>, arg_types: Vec<Bound<'_, PyAny>>) -> PyResult<PyBasicBlock> {
        let arg_types = super::types::type_handles_from_any(&arg_types)?;
        let ctx = super::get_ctx_mut()?;
        let label = label
            .map(|s| Identifier::try_from(s.to_string()))
            .transpose()
            .map_err(super::to_py_err)?;

        let ptr = BasicBlock::new(ctx, label, arg_types);
        Ok(PyBasicBlock { ptr })
    }

    /// The optional label of this block, as a `str` property.
    ///
    /// Read `block.label`; assign `block.label = "name"` (or `block.label = None`
    /// to clear it) via the paired setter below.
    #[getter]
    fn label(&self) -> PyResult<Option<String>> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).label.as_ref().map(|id| id.to_string()))
    }

    /// Setter for the `label` property (`block.label = "name"`).
    #[setter]
    fn set_label(&self, value: Option<&str>) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        let label = value
            .map(|s| Identifier::try_from(s.to_string()))
            .transpose()
            .map_err(super::to_py_err)?;
        self.ptr.deref_mut(ctx).set_label(ctx, label);
        Ok(())
    }


    // ------------------------------------------------------------------
    // Block arguments
    // ------------------------------------------------------------------

    /// Get the `idx`-th block argument as a [`PyValue`].
    fn get_argument(&self, idx: usize) -> PyResult<PyValue> {
        let ctx = super::get_ctx()?;
        Ok(PyValue {
            val: self.ptr.deref(ctx).get_argument(idx),
        })
    }

    /// All block arguments as a list of [`PyValue`].
    #[getter]
    fn arguments(&self) -> PyResult<Vec<PyValue>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .arguments()
            .map(|v| PyValue { val: v })
            .collect())
    }

    pub fn push_argument(&self, arg: &Bound<'_, PyAny>) -> PyResult<usize> {
        let handle = type_handle_from_any(arg)?;
        let ctx = super::get_ctx_mut()?;

        let idx = BasicBlock::push_argument(self.ptr, ctx, handle);
        Ok(idx)
    }

    pub fn pop_argument(&self) -> PyResult<()> {
        let ctx = super::get_ctx_mut()?;
        BasicBlock::pop_argument(self.ptr, ctx);
        Ok(())
    }

    pub fn insert_argument(&self, idx: usize, arg: &Bound<'_, PyAny>) -> PyResult<()> {
        let handle = type_handle_from_any(arg)?;
        let ctx = super::get_ctx_mut()?;

        BasicBlock::insert_argument(self.ptr, ctx, idx, handle);
        Ok(())
    }

    pub fn remove_argument(&self, idx: usize) -> PyResult<()> {
        let ctx = super::get_ctx_mut()?;
        BasicBlock::remove_argument(self.ptr, ctx, idx);
        Ok(())
    }

    /// Number of block arguments.
    #[getter]
    fn num_arguments(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).get_num_arguments())
    }

    // ------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------
    /// All successor blocks reachable through this block's terminator.
    #[getter]
    fn successors(&self) -> PyResult<Vec<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .succs(ctx)
            .into_iter()
            .map(|ptr| PyBasicBlock { ptr })
            .collect())
    }

    fn has_successor(&self) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).has_succ(ctx))
    }

    fn is_successor(&self, other: &PyBasicBlock) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).is_succ(ctx, other.ptr))
    }

    fn get_successor(&self, idx: usize) -> PyResult<PyBasicBlock> {
        let ctx = super::get_ctx()?;
        let ptr = self.ptr.deref(ctx).get_succ(ctx, idx);
        Ok(PyBasicBlock { ptr })
    }

    fn num_successors(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).num_succ(ctx))
    }

    /// The parent region, or `None`.
    #[getter]
    fn parent_region(&self) -> PyResult<Option<PyRegion>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_region()
            .map(|ptr| PyRegion { ptr }))
    }

    /// The parent operation (via parent region), or `None`.
    #[getter]
    fn parent_op(&self) -> PyResult<Option<PyOperation>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_op(ctx)
            .map(|ptr| PyOperation { ptr }))
    }

    /// The parent block, or `None`.
    #[getter]
    fn parent_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_block(ctx)
            .map(|ptr| PyBasicBlock { ptr }))
    }

    // ------------------------------------------------------------------
    // Operations
    // ------------------------------------------------------------------

    /// The block's terminator operation, or `None` if absent.
    fn get_terminator(&self) -> PyResult<Option<PyOperation>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_terminator(ctx)
            .map(|ptr| PyOperation { ptr }))
    }

    fn drop_all_uses(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        BasicBlock::drop_all_uses(self.ptr, ctx);
        Ok(())
    }

    // TODO: this renders the py'side obect invalid, since the ptr is deallocated.
    // This should not cause segfaults since all the code should safely try-to-deref
    // everything and result in a panic, which is handled.
    // Need a consistend design for how to safely model this
    fn erase(&self) -> PyResult<()> {
        let ctx = super::get_ctx_mut()?;
        BasicBlock::erase(self.ptr, ctx);
        Ok(())
    }

    /// The block's operations as a lazy iterable: `for op in block.ops():`.
    ///
    /// Walks the intrusive op list on demand — no upfront allocation. It is safe
    /// to erase the just-yielded op inside the loop; mutating *ahead* of the
    /// cursor can dangle it (use [`get_ops()`](Self::get_ops) for a stable
    /// snapshot). Each call returns a fresh iterator.
    fn ops(&self) -> PyResult<PyBlockOpsIter> {
        let ctx = super::get_ctx()?;
        Ok(PyBlockOpsIter {
            next: self.ptr.deref(ctx).get_head(),
        })
    }


    // ------------------------------------------------------------------
    // LinkedList
    // ------------------------------------------------------------------
    fn get_next(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_next()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    fn get_prev(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_prev()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    fn get_container(&self) -> PyResult<Option<PyRegion>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_container()
            .map(|ptr| PyRegion { ptr }))
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
    // Identity
    // ------------------------------------------------------------------

    fn __eq__(&self, other: &PyBasicBlock) -> bool {
        self.ptr == other.ptr
    }

    fn __hash__(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.ptr.hash(&mut h);
        h.finish() as usize
    }
}

/// Lazy iterator over a [`PyBasicBlock`]'s operations — an intrusive-linked-list
/// cursor. Produced by `block.ops()`; yields [`PyOperation`]s until exhausted.
#[pyclass(unsendable, name = "BlockOpsIterator")]
pub struct PyBlockOpsIter {
    /// The next operation to yield, or `None` once exhausted.
    next: Option<Ptr<Operation>>,
}

#[pymethods]
impl PyBlockOpsIter {
    /// An iterator is its own iterable (`iter(it) is it`).
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Advance the cursor and return the next op, or `None` (→ `StopIteration`).
    fn __next__(&mut self) -> PyResult<Option<PyOperation>> {
        let ctx = super::get_ctx()?;
        match self.next {
            Some(cur) => {
                // Advance *before* yielding so that erasing the yielded op inside
                // the loop body does not dangle this cursor (it already points at
                // the successor). Mutating *ahead* of the cursor can still dangle.
                self.next = cur.deref(ctx).get_next();
                Ok(Some(PyOperation { ptr: cur }))
            }
            None => Ok(None),
        }
    }
}
