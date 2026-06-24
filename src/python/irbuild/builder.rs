//! [`PyIRBuilder`] — Python wrapper for IR insertion operations.

use pyo3::prelude::*;

use alloc::vec::Vec;

use crate::irbuild::{
    inserter::{BlockInsertionPoint, IRInserter, Inserter, OpInsertionPoint},
    listener::DummyListener,
};

use crate::python::{
    basic_block::PyBasicBlock,
    operation::PyOperation,
    region::PyRegion,
    to_py_err,
};

/// A helper for inserting operations at a specific point in the IR.
///
/// Create one with one of the factory methods (`at_block_start`, `at_block_end`,
/// `after_op`, `before_op`), then call `append_op` or `insert_op` to place
/// operations at that point.
///
/// ```python
/// b = pliron.IRBuilder.at_block_end(block)
/// b.append_op(my_op)
/// ```
#[pyclass(unsendable, name = "IRBuilder")]
pub struct PyIRBuilder {
    inner: IRInserter<DummyListener>,
}

#[pymethods]
impl PyIRBuilder {
    // ------------------------------------------------------------------
    // Constructors / factory methods
    // ------------------------------------------------------------------

    /// Create a builder whose insertion point is at the end of `block`.
    #[staticmethod]
    fn at_block_end(block: &PyBasicBlock) -> PyIRBuilder {
        PyIRBuilder {
            inner: IRInserter::new_at_block_end(block.ptr),
        }
    }

    /// Create a builder whose insertion point is at the start of `block`.
    #[staticmethod]
    fn at_block_start(block: &PyBasicBlock) -> PyIRBuilder {
        PyIRBuilder {
            inner: IRInserter::new_at_block_start(block.ptr),
        }
    }

    /// Create a builder whose insertion point is *after* `op`.
    #[staticmethod]
    fn after_op(op: &PyOperation) -> PyIRBuilder {
        PyIRBuilder {
            inner: IRInserter::new(OpInsertionPoint::AfterOperation(op.ptr)),
        }
    }

    /// Create a builder whose insertion point is *before* `op`.
    #[staticmethod]
    fn before_op(op: &PyOperation) -> PyIRBuilder {
        PyIRBuilder {
            inner: IRInserter::new(OpInsertionPoint::BeforeOperation(op.ptr)),
        }
    }

    // ------------------------------------------------------------------
    // Insertion-point mutators
    // ------------------------------------------------------------------

    /// Move the insertion point to the end of `block`.
    fn set_at_block_end(&mut self, block: &PyBasicBlock) {
        self.inner
            .set_insertion_point(OpInsertionPoint::AtBlockEnd(block.ptr));
    }

    /// Move the insertion point to the start of `block`.
    fn set_at_block_start(&mut self, block: &PyBasicBlock) {
        self.inner
            .set_insertion_point(OpInsertionPoint::AtBlockStart(block.ptr));
    }

    /// Move the insertion point to just after `op`.
    fn set_after_op(&mut self, op: &PyOperation) {
        self.inner
            .set_insertion_point(OpInsertionPoint::AfterOperation(op.ptr));
    }

    /// Move the insertion point to just before `op`.
    fn set_before_op(&mut self, op: &PyOperation) {
        self.inner
            .set_insertion_point(OpInsertionPoint::BeforeOperation(op.ptr));
    }

    // ------------------------------------------------------------------
    // Operation insertion
    // ------------------------------------------------------------------

    /// Append `op` at the current insertion point and advance the point past it.
    fn append_op(&mut self, op: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx()?;
        self.inner.append_operation(ctx, op.ptr);
        Ok(())
    }

    /// Insert `op` at the current insertion point *without* advancing it.
    fn insert_op(&mut self, op: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx()?;
        self.inner.insert_operation(ctx, op.ptr);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Block creation
    // ------------------------------------------------------------------

    /// Create a new basic block and insert it at the end of `region`.
    ///
    /// `arg_types` is a list of [`PyType`] objects for the block arguments.
    /// `label` is an optional string label for the block.
    ///
    /// Returns the newly created block.
    #[pyo3(signature = (region, arg_types, label=None))]
    fn create_block_at_region_end(
        &mut self,
        region: &PyRegion,
        arg_types: Vec<Bound<'_, PyAny>>,
        label: Option<&str>,
    ) -> PyResult<PyBasicBlock> {
        self.create_block(arg_types, label, BlockInsertionPoint::AtRegionEnd(region.ptr))
    }

    /// Create a new basic block and insert it at the start of `region`.
    #[pyo3(signature = (region, arg_types, label=None))]
    fn create_block_at_region_start(
        &mut self,
        region: &PyRegion,
        arg_types: Vec<Bound<'_, PyAny>>,
        label: Option<&str>,
    ) -> PyResult<PyBasicBlock> {
        self.create_block(arg_types, label, BlockInsertionPoint::AtRegionStart(region.ptr))
    }
}

impl PyIRBuilder {
    /// Shared block-creation logic for the public `create_block_at_region_*`
    /// methods, parameterised by the insertion point.
    fn create_block(
        &mut self,
        arg_types: Vec<Bound<'_, PyAny>>,
        label: Option<&str>,
        insertion_point: BlockInsertionPoint,
    ) -> PyResult<PyBasicBlock> {
        let type_ptrs = crate::python::types::type_handles_from_any(&arg_types)?;
        let ctx = crate::python::get_ctx_mut()?;
        let ident = label
            .map(|s| -> crate::result::Result<crate::identifier::Identifier> {
                s.try_into()
            })
            .transpose()
            .map_err(to_py_err)?;
        let block_ptr = self
            .inner
            .create_block(ctx, insertion_point, ident, type_ptrs);
        Ok(PyBasicBlock { ptr: block_ptr })
    }
}
