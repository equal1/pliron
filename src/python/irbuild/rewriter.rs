//! [`PyRewriter`] and [`PyMatchRewriteAdapter`] — Python-accessible rewrite infrastructure.

use pyo3::prelude::*;

use alloc::{format, string::String, vec::Vec};

use crate::{
    context::Ptr,
    identifier::Identifier,
    irbuild::{
        inserter::{BlockInsertionPoint, Inserter, OpInsertionPoint},
        match_rewrite::{MatchRewrite, MatchRewriter, apply_match_rewrite},
        rewriter::Rewriter,
    },
    operation::Operation,
    value::Value,
};

use super::PyIRStatus;
use crate::python::{
    basic_block::PyBasicBlock,
    operation::PyOperation,
    region::PyRegion,
    to_py_err,
    value::PyValue,
};

// ---------------------------------------------------------------------------
// PyRewriter — exposed to Python inside match/rewrite callbacks
// ---------------------------------------------------------------------------

/// A rewriter object passed to Python `rewrite(rewriter, op)` callbacks.
///
/// Wraps a raw pointer to a [`MatchRewriter`] that is owned by the Rust
/// `apply_match_rewrite` call stack.  Using the rewriter after the callback
/// returns is undefined behaviour and will panic.
#[pyclass(unsendable, name = "Rewriter")]
pub struct PyRewriter {
    /// Raw pointer to the underlying Rust rewriter, valid only during a callback.
    inner: *mut MatchRewriter,
}

impl PyRewriter {
    /// Safety: caller must guarantee `rw` lives at least as long as this PyRewriter.
    pub(crate) fn new(rw: &mut MatchRewriter) -> Self {
        PyRewriter { inner: rw as *mut _ }
    }

    fn get_rw(&self) -> PyResult<&mut MatchRewriter> {
        if self.inner.is_null() {
            Err(crate::python::PlironError::new_err(
                "Rewriter used outside of a match/rewrite callback",
            ))
        } else {
            // Safety: only one Python frame runs at a time (GIL).
            Ok(unsafe { &mut *self.inner })
        }
    }
}

#[pymethods]
impl PyRewriter {
    // ------------------------------------------------------------------
    // Insertion-point setters
    // ------------------------------------------------------------------

    fn set_at_block_end(&mut self, block: &PyBasicBlock) -> PyResult<()> {
        self.get_rw()?.set_insertion_point_to_block_end(block.ptr);
        Ok(())
    }

    fn set_at_block_start(&mut self, block: &PyBasicBlock) -> PyResult<()> {
        self.get_rw()?.set_insertion_point_to_block_start(block.ptr);
        Ok(())
    }

    fn set_after_op(&mut self, op: &PyOperation) -> PyResult<()> {
        self.get_rw()?
            .set_insertion_point_after_operation(op.ptr);
        Ok(())
    }

    fn set_before_op(&mut self, op: &PyOperation) -> PyResult<()> {
        self.get_rw()?
            .set_insertion_point_before_operation(op.ptr);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Operation-level rewrites
    // ------------------------------------------------------------------

    /// Erase `op`.  The operation must have no uses.
    fn erase_op(&mut self, op: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx_mut()?;
        self.get_rw()?.erase_operation(ctx, op.ptr);
        Ok(())
    }

    /// Replace `old_op` with `new_op`.
    /// Both must have the same result types.
    fn replace_op_with_op(&mut self, old_op: &PyOperation, new_op: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx_mut()?;
        self.get_rw()?.replace_operation(ctx, old_op.ptr, new_op.ptr);
        Ok(())
    }

    /// Replace `op` with a list of values.
    fn replace_op_with_values(
        &mut self,
        op: &PyOperation,
        values: Vec<PyRef<'_, PyValue>>,
    ) -> PyResult<()> {
        let ctx = crate::python::get_ctx_mut()?;
        let vals: Vec<Value> = values.iter().map(|v| v.val).collect();
        self.get_rw()?.replace_operation_with_values(ctx, op.ptr, vals);
        Ok(())
    }

    /// Erase `block`.  The block must have no uses.
    fn erase_block(&mut self, block: &PyBasicBlock) -> PyResult<()> {
        let ctx = crate::python::get_ctx_mut()?;
        self.get_rw()?.erase_block(ctx, block.ptr);
        Ok(())
    }

    /// Move `op` to be just after the insertion point `after`.
    fn move_op_after(&mut self, op: &PyOperation, after: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx()?;
        self.get_rw()?
            .move_operation(ctx, op.ptr, OpInsertionPoint::AfterOperation(after.ptr));
        Ok(())
    }

    /// Move `op` to be just before `before`.
    fn move_op_before(&mut self, op: &PyOperation, before: &PyOperation) -> PyResult<()> {
        let ctx = crate::python::get_ctx()?;
        self.get_rw()?
            .move_operation(ctx, op.ptr, OpInsertionPoint::BeforeOperation(before.ptr));
        Ok(())
    }

    /// Split `block` just before `op`, returning the new second half.
    ///
    /// `new_block_label` is an optional name for the new block. Pass a string
    /// to name it, or `None` (or omit it) to let the rewriter derive a name
    /// from `block`. A non-empty string that is not a valid identifier raises.
    #[pyo3(signature = (block, op, new_block_label=None))]
    fn split_block_before(
        &mut self,
        block: &PyBasicBlock,
        op: &PyOperation,
        new_block_label: Option<String>,
    ) -> PyResult<PyBasicBlock> {
        let ctx = crate::python::get_ctx_mut()?;
        let label: Option<Identifier> = new_block_label
            .map(Identifier::try_new)
            .transpose()
            .map_err(to_py_err)?;
        let new_block = self.get_rw()?.split_block(
            ctx,
            block.ptr,
            OpInsertionPoint::BeforeOperation(op.ptr),
            label,
        );
        Ok(PyBasicBlock { ptr: new_block })
    }

    /// Inline all blocks of `src_region` at the start of `dst_region`.
    fn inline_region_at_start(
        &mut self,
        src: &PyRegion,
        dst: &PyRegion,
    ) -> PyResult<()> {
        let ctx = crate::python::get_ctx()?;
        self.get_rw()?.inline_region(
            ctx,
            src.ptr,
            BlockInsertionPoint::AtRegionStart(dst.ptr),
        );
        Ok(())
    }

    /// Return `True` if the IR has been modified since this rewriter was created.
    fn is_modified(&mut self) -> PyResult<bool> {
        Ok(self.get_rw()?.is_modified())
    }
}

// ---------------------------------------------------------------------------
// PyMatchRewriteAdapter — bridge that calls Python `match` / `rewrite`
// ---------------------------------------------------------------------------

/// Adapter that turns a Python object with `match(op)` and `rewrite(rewriter, op)`
/// methods into a Rust [`MatchRewrite`] implementation.
pub(crate) struct PyMatchRewriteAdapter {
    py_obj: Py<PyAny>,
}

impl PyMatchRewriteAdapter {
    pub fn new(py_obj: Py<PyAny>) -> Self {
        Self { py_obj }
    }
}

impl MatchRewrite for PyMatchRewriteAdapter {
    fn r#match(&mut self, _ctx: &crate::context::Context, op: Ptr<Operation>) -> bool {
        Python::with_gil(|py| {
            let py_op = PyOperation { ptr: op };
            match self
                .py_obj
                .call_method1(py, "match", (py_op,))
                .and_then(|r| r.extract::<bool>(py))
            {
                Ok(b) => b,
                Err(e) => {
                    e.print(py);
                    false
                }
            }
        })
    }

    fn rewrite(
        &mut self,
        _ctx: &mut crate::context::Context,
        rewriter: &mut MatchRewriter,
        op: Ptr<Operation>,
    ) -> crate::result::Result<()> {
        Python::with_gil(|py| {
            let py_rw = Py::new(py, PyRewriter::new(rewriter))
                .expect("PyRewriter allocation failed");
            let py_op = PyOperation { ptr: op };
            let result = self.py_obj.call_method1(py, "rewrite", (py_rw.clone_ref(py), py_op));
            // Invalidate the rewriter so use-after-callback panics cleanly.
            py_rw.borrow_mut(py).inner = std::ptr::null_mut();
            result
        })
        .map(|_| ())
        .map_err(|e| {
            crate::arg_error_noloc!(crate::python::irbuild::rewriter::PythonCallbackError(format!(
                "{}",
                e
            )))
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Python callback error: {0}")]
pub struct PythonCallbackError(pub String);

// ---------------------------------------------------------------------------
// apply_match_rewrite — top-level Python function
// ---------------------------------------------------------------------------

/// Apply `rewrite_obj` (a Python object with `match` / `rewrite` methods)
/// to all matching operations in the IR tree rooted at `root_op`.
///
/// Returns an [`IRStatus`](PyIRStatus) indicating whether the IR was changed.
///
/// ```python
/// class FoldAddZero:
///     def match(self, op) -> bool:
///         return op.op_name() == "llvm.add"
///
///     def rewrite(self, rewriter, op):
///         lhs = op.get_operand(0)
///         # ... inspect & rewrite
///
/// status = pliron.apply_match_rewrite(root_op, FoldAddZero())
/// ```
#[pyfunction]
#[pyo3(name = "apply_match_rewrite")]
pub fn py_apply_match_rewrite(
    root_op: &PyOperation,
    rewrite_obj: Py<PyAny>,
) -> PyResult<PyIRStatus> {
    let ctx = crate::python::get_ctx_mut()?;
    let adapter = PyMatchRewriteAdapter::new(rewrite_obj);
    apply_match_rewrite(ctx, adapter, root_op.ptr)
        .map(PyIRStatus::from)
        .map_err(to_py_err)
}
