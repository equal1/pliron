//! Python bindings for [`::pliron::irbuild::cloning`] — cloning IR entities with a
//! value / block / op remapping. Exposed as functions on the `pliron.irbuild`
//! submodule, plus the [`IrMapping`] helper.
//!
//! The core cloning functions insert new IR through a
//! [`Rewriter`](::pliron::irbuild::rewriter::Rewriter) (to notify any listener it
//! carries). Each wrapper builds a throw-away `IRRewriter<PyRewriteListener>`
//! internally, attaching the caller's optional `listener` object to it.

use std::vec::Vec;

use pyo3::prelude::*;

use ::pliron::{
    basic_block::BasicBlock,
    context::Ptr,
    irbuild::{
        cloning::{self, IrMapping},
        rewriter::IRRewriter,
    },
};

use super::listener::PyRewriteListener;
use crate::{basic_block::PyBasicBlock, operation::PyOperation, region::PyRegion, value::PyValue};

/// A mapping from original IR entities to their clones, threaded through the
/// `clone_*` functions.
///
/// Seed it (`map_value` / `map_block` / `map_op`) to redirect operands and
/// successors while cloning; anything absent resolves to itself. Passing it also
/// lets several clone calls share one remapping and lets you read back what each
/// original became.
#[pyclass(unsendable, name = "IrMapping")]
pub struct PyIrMapping {
    pub(crate) inner: IrMapping,
}

#[pymethods]
impl PyIrMapping {
    /// Create an empty mapping.
    #[new]
    fn new() -> Self {
        PyIrMapping {
            inner: IrMapping::new(),
        }
    }

    /// Record that `from` clones to `to` (overwrites any existing entry).
    fn map_value(&mut self, from: &PyValue, to: &PyValue) {
        self.inner.map_value(from.val, to.val);
    }

    /// Record that `from` clones to `to` (overwrites any existing entry).
    fn map_block(&mut self, from: &PyBasicBlock, to: &PyBasicBlock) {
        self.inner.map_block(from.ptr, to.ptr);
    }

    /// Record that `from` clones to `to` (overwrites any existing entry).
    fn map_op(&mut self, from: &PyOperation, to: &PyOperation) {
        self.inner.map_op(from.ptr, to.ptr);
    }

    /// The clone recorded for `from`, or `None`.
    fn lookup_value(&self, from: &PyValue) -> Option<PyValue> {
        self.inner.lookup_value(from.val).map(|val| PyValue { val })
    }

    /// The clone recorded for `from`, or `None`.
    fn lookup_block(&self, from: &PyBasicBlock) -> Option<PyBasicBlock> {
        self.inner
            .lookup_block(from.ptr)
            .map(|ptr| PyBasicBlock { ptr })
    }

    /// The clone recorded for `from`, or `None`.
    fn lookup_op(&self, from: &PyOperation) -> Option<PyOperation> {
        self.inner
            .lookup_op(from.ptr)
            .map(|ptr| PyOperation { ptr })
    }

    /// The clone recorded for `from`, or `from` itself if none was recorded.
    fn lookup_value_or_default(&self, from: &PyValue) -> PyValue {
        PyValue {
            val: self.inner.lookup_value_or_default(from.val),
        }
    }

    /// The clone recorded for `from`, or `from` itself if none was recorded.
    fn lookup_block_or_default(&self, from: &PyBasicBlock) -> PyBasicBlock {
        PyBasicBlock {
            ptr: self.inner.lookup_block_or_default(from.ptr),
        }
    }
}

/// Set up the throw-away [`Rewriter`](::pliron::irbuild::rewriter::Rewriter) (with
/// the caller's `listener` attached, if any) and the `&mut IrMapping` (the
/// caller's if provided, else a fresh one), then run `f`. Lets each wrapper call
/// its native `cloning::*` function exactly once.
///
/// New blocks/ops are inserted through the rewriter, so an attached `listener`
/// receives `notify_operation_inserted` / `notify_block_inserted` for the clones.
fn with_clone_env<R>(
    mapping: Option<PyRefMut<'_, PyIrMapping>>,
    listener: Option<Py<PyAny>>,
    f: impl FnOnce(&mut IRRewriter<PyRewriteListener>, &mut IrMapping) -> R,
) -> R {
    let mut rewriter = IRRewriter::<PyRewriteListener>::default();
    if let Some(obj) = listener {
        rewriter.set_listener(PyRewriteListener::new(obj));
    }
    match mapping {
        Some(mut m) => f(&mut rewriter, &mut m.inner),
        None => f(&mut rewriter, &mut IrMapping::new()),
    }
}

/// Clone `op` (and the contents of its regions), remapping operands and
/// successors through `mapping` (a fresh empty mapping if omitted).
///
/// The returned operation is **unlinked** — insert it yourself (e.g. via
/// `IRBuilder` or `op.insert_at_back(block)`). Operands/successors absent from
/// the mapping are kept as-is, so uses of values defined outside `op` are shared
/// with the original. An optional `listener` is notified of blocks/ops created
/// while cloning nested regions.
#[pyfunction]
#[pyo3(signature = (op, mapping=None, listener=None))]
pub fn clone_operation(
    op: &PyOperation,
    mapping: Option<PyRefMut<'_, PyIrMapping>>,
    listener: Option<Py<PyAny>>,
) -> PyResult<PyOperation> {
    let ctx = crate::get_ctx_mut()?;
    let new_op = with_clone_env(mapping, listener, |rewriter, mapper| {
        cloning::clone_operation(op.ptr, ctx, rewriter, mapper)
    });
    Ok(PyOperation { ptr: new_op })
}

/// Clone every block of `src` into `dest`, appended at its end, remapping
/// through `mapping` (a fresh empty mapping if omitted). An optional `listener`
/// is notified of the created blocks/ops.
#[pyfunction]
#[pyo3(signature = (src, dest, mapping=None, listener=None))]
pub fn clone_region_into(
    src: &PyRegion,
    dest: &PyRegion,
    mapping: Option<PyRefMut<'_, PyIrMapping>>,
    listener: Option<Py<PyAny>>,
) -> PyResult<()> {
    let ctx = crate::get_ctx_mut()?;
    with_clone_env(mapping, listener, |rewriter, mapper| {
        cloning::clone_region_into(src.ptr, dest.ptr, ctx, rewriter, mapper)
    });
    Ok(())
}

/// Clone `blocks` (and their operations) into `dest`, appended at its end in the
/// given order, remapping through `mapping` (a fresh empty mapping if omitted).
/// An optional `listener` is notified of the created blocks/ops.
///
/// Order-independent: forward branches, back-edges, and operands whose def is
/// cloned later all still resolve to their clones.
#[pyfunction]
#[pyo3(signature = (blocks, dest, mapping=None, listener=None))]
pub fn clone_blocks_into(
    blocks: Vec<PyRef<'_, PyBasicBlock>>,
    dest: &PyRegion,
    mapping: Option<PyRefMut<'_, PyIrMapping>>,
    listener: Option<Py<PyAny>>,
) -> PyResult<()> {
    let block_ptrs: Vec<Ptr<BasicBlock>> = blocks.iter().map(|b| b.ptr).collect();
    let ctx = crate::get_ctx_mut()?;
    with_clone_env(mapping, listener, |rewriter, mapper| {
        cloning::clone_blocks_into(&block_ptrs, dest.ptr, ctx, rewriter, mapper)
    });
    Ok(())
}
