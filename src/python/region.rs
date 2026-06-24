//! [`PyRegion`] — Python wrapper for [`crate::region::Region`].

use pyo3::prelude::*;

use alloc::{format, string::String, vec::Vec};

use crate::{
    context::Ptr,
    linked_list::LinkedList,
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

    /// All basic blocks in this region (in order) as a list of [`PyBasicBlock`].
    fn blocks(&self) -> PyResult<Vec<PyBasicBlock>> {
        use crate::linked_list::ContainsLinkedList;
        let ctx = super::get_ctx()?;
        let mut blocks = Vec::new();
        let mut cur = self.ptr.deref(ctx).get_head();
        while let Some(block_ptr) = cur {
            cur = block_ptr.deref(ctx).get_next();
            blocks.push(PyBasicBlock { ptr: block_ptr });
        }
        Ok(blocks)
    }

    /// The entry (first) block of this region, or `None` if empty.
    fn entry_block(&self) -> PyResult<Option<PyBasicBlock>> {
        use crate::linked_list::ContainsLinkedList;
        let ctx = super::get_ctx()?;
        Ok(self
            .ptr
            .deref(ctx)
            .get_head()
            .map(|ptr| PyBasicBlock { ptr }))
    }

    /// Number of basic blocks in this region.
    fn num_blocks(&self) -> PyResult<usize> {
        use crate::linked_list::ContainsLinkedList;
        let ctx = super::get_ctx()?;
        let mut count = 0usize;
        let mut cur = self.ptr.deref(ctx).get_head();
        while let Some(block_ptr) = cur {
            cur = block_ptr.deref(ctx).get_next();
            count += 1;
        }
        Ok(count)
    }

    // ------------------------------------------------------------------
    // Navigation
    // ------------------------------------------------------------------

    /// The operation that owns this region.
    fn parent_op(&self) -> PyResult<PyOperation> {
        let ctx = super::get_ctx()?;
        Ok(PyOperation {
            ptr: self.ptr.deref(ctx).get_parent_op(),
        })
    }

    /// Index of this region among siblings in the parent operation.
    fn index_in_parent(&self) -> PyResult<usize> {
        let ctx = super::get_ctx()?;
        Ok(self.ptr.deref(ctx).find_index_in_parent(ctx))
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
