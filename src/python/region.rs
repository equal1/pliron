//! [`PyRegion`] — Python wrapper for [`crate::region::Region`].

use pyo3::prelude::*;

use alloc::{format, string::String};

use crate::{
    basic_block::BasicBlock,
    context::Ptr,
    linked_list::{ContainsLinkedList, LinkedList},
    printable::Printable,
    common_traits::Verify,
    region::Region,
};

use super::{basic_block::PyBasicBlock, operation::PyOperation};

/// A handle to a pliron IR region.
#[pyclass(unsendable, name = "Region")]
pub struct PyRegion {
    pub ptr: Ptr<Region>,
}

#[pymethods]
impl PyRegion {
    // ------------------------------------------------------------------
    // Blocks
    // ------------------------------------------------------------------

    /// The region's blocks as a lazy iterable: `for b in region.blocks():`.
    ///
    /// Walks the intrusive block list on demand — no upfront allocation; use
    /// `list(region.blocks())` for a materialised snapshot. It is safe to erase
    /// the just-yielded block inside the loop; mutating *ahead* of the cursor can
    /// dangle it. Each call returns a fresh iterator.
    fn blocks(&self) -> PyResult<PyRegionBlocksIter> {
        let ctx = super::get_ctx()?;
        Ok(PyRegionBlocksIter {
            next: self.ptr.deref(ctx).get_head(),
        })
    }

    /// The entry (first) block of this region, or `None` if empty.
    fn entry_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_head()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    /// The last block of this region, or `None` if empty.
    fn last_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_tail()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    /// Number of basic blocks in this region.
    #[getter]
    fn num_blocks(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).iter(ctx).count())
    }

    // ------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------

    /// The operation that owns this region.
    #[getter]
    fn parent_op(&self) -> PyResult<PyOperation> {
        let ctx = super::get_ctx()?;
        Ok(PyOperation {
            ptr: self.ptr.deref(ctx).get_parent_op(),
        })
    }

    /// The region enclosing this one (via the parent op), or `None`.
    #[getter]
    fn parent_region(&self) -> PyResult<Option<PyRegion>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_region(ctx)
            .map(|ptr| PyRegion { ptr }))
    }

    /// The block enclosing this region (via the parent op), or `None`.
    #[getter]
    fn parent_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_parent_block(ctx)
            .map(|ptr| PyBasicBlock { ptr }))
    }

    /// Index of this region among siblings in the parent operation.
    #[getter]
    fn index_in_parent(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).find_index_in_parent(ctx))
    }

    // ------------------------------------------------------------------
    // Queries / mutation
    // ------------------------------------------------------------------

    /// Does this region use SSA dominance?
    fn has_ssa_dominance(&self) -> PyResult<bool> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).has_ssa_dominance(ctx))
    }

    /// Move this region to (the end of) `new_parent_op`.
    ///
    /// No-op if it is already that op's region. Note this invalidates the
    /// `index_in_parent` of the other regions in the current parent.
    fn move_to_op(&self, new_parent_op: &PyOperation) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        Region::move_to_op(self.ptr, new_parent_op.ptr, ctx);
        Ok(())
    }

    /// Drop all uses held by every block in this region.
    fn drop_all_uses(&self) -> PyResult<()> {
        let ctx = super::get_ctx()?;
        Region::drop_all_uses(self.ptr, ctx);
        Ok(())
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

    fn __eq__(&self, other: &PyRegion) -> bool {
        self.ptr == other.ptr
    }

    fn __hash__(&self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.ptr.hash(&mut h);
        h.finish() as usize
    }
}

/// Lazy iterator over a [`PyRegion`]'s basic blocks — an intrusive-linked-list
/// cursor. Produced by `region.blocks()`; yields [`PyBasicBlock`]s until exhausted.
#[pyclass(unsendable, name = "RegionBlocksIterator")]
pub struct PyRegionBlocksIter {
    /// The next block to yield, or `None` once exhausted.
    next: Option<Ptr<BasicBlock>>,
}

#[pymethods]
impl PyRegionBlocksIter {
    /// An iterator is its own iterable (`iter(it) is it`).
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Advance the cursor and return the next block, or `None` (→ `StopIteration`).
    fn __next__(&mut self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = super::get_ctx()?;
        match self.next {
            Some(cur) => {
                // Advance *before* yielding so erasing the yielded block inside the
                // loop body does not dangle this cursor. Mutating *ahead* of the
                // cursor can still dangle it.
                self.next = cur.deref(ctx).get_next();
                Ok(Some(PyBasicBlock { ptr: cur }))
            }
            None => Ok(None),
        }
    }
}
