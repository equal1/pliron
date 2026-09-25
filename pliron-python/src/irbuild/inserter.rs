//! Python bindings for [`::pliron::irbuild::inserter`].
//!
//! - [`PyOpInsertionPoint`] / [`PyBlockInsertionPoint`] — first-class insertion
//!   point objects (`pliron.irbuild.OpInsertionPoint` / `BlockInsertionPoint`),
//!   built via static constructors (`at_block_end(block)`, …). Objects keep the
//!   API 1:1 with Rust: one `set_insertion_point(point)` instead of a method per
//!   variant.
//! - [`PyInserter`] — a Rust-side dispatch struct (like the listener structs in
//!   [`super::listener`]): holds a `Py<PyAny>` and implements the Rust
//!   [`Inserter`] trait by calling the same-named method on the wrapped Python
//!   object. Not a `#[pyclass]`; used where Rust entry points need a
//!   Python-implemented inserter. Unlike listener dispatch, inserter methods
//!   are **required effects**, so a missing/raising method is a hard failure
//!   (see `call_required`).
//! - [`PyIRInserter`] — `pliron.irbuild.IRInserter`, a `#[pyclass]` wrapping the
//!   native [`IRInserter<PyInsertionListener>`]; every method delegates to the
//!   wrapped native inserter. The optional `listener` is any (duck-typed) Python
//!   object implementing the listener protocol — see
//!   [`super::listener`].
//!
//! `ScopedInserter` is deliberately not exposed: its save/restore-on-drop
//! semantics do not map to Python object lifetimes.

use pyo3::prelude::*;
use pyo3::types::PyTuple;

use std::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use ::pliron::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    identifier::Identifier,
    irbuild::inserter::{BlockInsertionPoint, IRInserter, Inserter, OpInsertionPoint},
    operation::Operation,
    printable::Printable,
    r#type::TypeHandle,
};

use super::listener::PyInsertionListener;
use crate::{basic_block::PyBasicBlock, operation::PyOperation, region::PyRegion, types::PyType};

// ---------------------------------------------------------------------------
// Insertion points
// ---------------------------------------------------------------------------

/// Where the next [Operation](::pliron::operation::Operation) will be inserted
/// (`pliron.irbuild.OpInsertionPoint`).
#[pyclass(unsendable, name = "OpInsertionPoint")]
#[derive(Clone)]
pub struct PyOpInsertionPoint {
    pub(crate) inner: OpInsertionPoint,
}

#[pymethods]
impl PyOpInsertionPoint {
    /// An unset insertion point.
    #[staticmethod]
    fn unset() -> Self {
        Self {
            inner: OpInsertionPoint::Unset,
        }
    }

    /// Insert at the start of `block`.
    #[staticmethod]
    fn at_block_start(block: &PyBasicBlock) -> Self {
        Self {
            inner: OpInsertionPoint::AtBlockStart(block.ptr),
        }
    }

    /// Insert at the end of `block`.
    #[staticmethod]
    fn at_block_end(block: &PyBasicBlock) -> Self {
        Self {
            inner: OpInsertionPoint::AtBlockEnd(block.ptr),
        }
    }

    /// Insert just after `op`.
    #[staticmethod]
    fn after_operation(op: &PyOperation) -> Self {
        Self {
            inner: OpInsertionPoint::AfterOperation(op.ptr),
        }
    }

    /// Insert just before `op`.
    #[staticmethod]
    fn before_operation(op: &PyOperation) -> Self {
        Self {
            inner: OpInsertionPoint::BeforeOperation(op.ptr),
        }
    }

    /// Is the insertion point set?
    fn is_set(&self) -> bool {
        self.inner.is_set()
    }

    /// The block insertion will occur in, if known.
    fn get_insertion_block(&self) -> PyResult<Option<PyBasicBlock>> {
        let ctx = crate::get_ctx()?;
        Ok(self
            .inner
            .get_insertion_block(ctx)
            .map(|ptr| PyBasicBlock { ptr }))
    }

    fn __str__(&self) -> PyResult<String> {
        let ctx = crate::get_ctx()?;
        Ok(format!("{}", self.inner.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }
}

/// Where the next [BasicBlock](::pliron::basic_block::BasicBlock) will be inserted
/// (`pliron.irbuild.BlockInsertionPoint`).
#[pyclass(unsendable, name = "BlockInsertionPoint")]
#[derive(Clone)]
pub struct PyBlockInsertionPoint {
    pub(crate) inner: BlockInsertionPoint,
}

#[pymethods]
impl PyBlockInsertionPoint {
    /// An unset insertion point.
    #[staticmethod]
    fn unset() -> Self {
        Self {
            inner: BlockInsertionPoint::Unset,
        }
    }

    /// Insert at the start of `region`.
    #[staticmethod]
    fn at_region_start(region: &PyRegion) -> Self {
        Self {
            inner: BlockInsertionPoint::AtRegionStart(region.ptr),
        }
    }

    /// Insert at the end of `region`.
    #[staticmethod]
    fn at_region_end(region: &PyRegion) -> Self {
        Self {
            inner: BlockInsertionPoint::AtRegionEnd(region.ptr),
        }
    }

    /// Insert just after `block`.
    #[staticmethod]
    fn after_block(block: &PyBasicBlock) -> Self {
        Self {
            inner: BlockInsertionPoint::AfterBlock(block.ptr),
        }
    }

    /// Insert just before `block`.
    #[staticmethod]
    fn before_block(block: &PyBasicBlock) -> Self {
        Self {
            inner: BlockInsertionPoint::BeforeBlock(block.ptr),
        }
    }

    /// Is the insertion point set?
    fn is_set(&self) -> bool {
        self.inner.is_set()
    }

    /// The region insertion will occur in, if known.
    fn get_insertion_region(&self) -> PyResult<Option<PyRegion>> {
        let ctx = crate::get_ctx()?;
        Ok(self
            .inner
            .get_insertion_region(ctx)
            .map(|ptr| PyRegion { ptr }))
    }

    fn __str__(&self) -> PyResult<String> {
        let ctx = crate::get_ctx()?;
        Ok(format!("{}", self.inner.disp(ctx)))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.__str__()
    }
}

// ---------------------------------------------------------------------------
// PyInserter — a Python object as a Rust `Inserter`
// ---------------------------------------------------------------------------

/// Call `method(*args)` on a Python inserter/rewriter object, **requiring** it
/// to exist and succeed.
///
/// Unlike listener notifications, inserter/rewriter methods are effects the IR
/// depends on — skipping one would silently corrupt the IR. A missing method or
/// raising callback therefore panics (pyo3 turns this into a Python exception at
/// the boundary).
pub(crate) fn call_required<A>(obj: &Py<PyAny>, method: &str, args: A) -> Py<PyAny>
where
    A: for<'py> IntoPyObject<'py, Target = PyTuple>,
{
    Python::with_gil(|py| match obj.bind(py).call_method1(method, args) {
        Ok(ret) => ret.unbind(),
        Err(err) => {
            err.print(py);
            panic!(
                "required method `{method}` missing or failed on Python inserter/rewriter object"
            );
        }
    })
}

/// A Python object as a Rust [`Inserter`]. Each trait method dispatches to the
/// same-named method on the wrapped object (see `call_required` for the
/// failure semantics). The `&dyn Op` variants forward to the `*_operation`
/// ones, exactly as [`IRInserter`] does — Python only sees generic `Operation`
/// handles.
pub struct PyInserter {
    obj: Py<PyAny>,
}

impl PyInserter {
    /// Wrap a Python inserter object.
    pub fn new(obj: Py<PyAny>) -> Self {
        Self { obj }
    }
}

/// Implement [`Inserter`] for a dispatch struct holding a Python object in
/// `self.obj`. Written once; used by [`PyInserter`] here and `PyRewriter` in the
/// sibling `rewriter` module (which must also implement `Inserter`, since
/// `Rewriter: Inserter`).
macro_rules! impl_python_inserter {
    ($ty:ty) => {
        impl Inserter for $ty {
            fn append_operation(&mut self, _ctx: &Context, operation: Ptr<Operation>) {
                call_required(
                    &self.obj,
                    "append_operation",
                    (PyOperation { ptr: operation },),
                );
            }

            fn append_op(&mut self, ctx: &Context, op: &dyn ::pliron::op::Op) {
                self.append_operation(ctx, ::pliron::op::Op::get_operation(op));
            }

            fn insert_operation(&mut self, _ctx: &Context, operation: Ptr<Operation>) {
                call_required(
                    &self.obj,
                    "insert_operation",
                    (PyOperation { ptr: operation },),
                );
            }

            fn insert_op(&mut self, ctx: &Context, op: &dyn ::pliron::op::Op) {
                self.insert_operation(ctx, ::pliron::op::Op::get_operation(op));
            }

            fn insert_block(
                &mut self,
                _ctx: &Context,
                insertion_point: BlockInsertionPoint,
                block: Ptr<BasicBlock>,
            ) {
                call_required(
                    &self.obj,
                    "insert_block",
                    (
                        PyBlockInsertionPoint {
                            inner: insertion_point,
                        },
                        PyBasicBlock { ptr: block },
                    ),
                );
            }

            fn create_block(
                &mut self,
                _ctx: &mut Context,
                insertion_point: BlockInsertionPoint,
                label: Option<Identifier>,
                arg_types: Vec<TypeHandle>,
            ) -> Ptr<BasicBlock> {
                let arg_types: Vec<PyType> =
                    arg_types.into_iter().map(|ptr| PyType { ptr }).collect();
                let ret = call_required(
                    &self.obj,
                    "create_block",
                    (
                        PyBlockInsertionPoint {
                            inner: insertion_point,
                        },
                        arg_types,
                        label.map(|l| l.to_string()),
                    ),
                );
                Python::with_gil(|py| {
                    ret.extract::<PyRef<'_, PyBasicBlock>>(py)
                        .expect("Python create_block() must return a BasicBlock")
                        .ptr
                })
            }

            fn get_insertion_point(&self) -> OpInsertionPoint {
                let ret = call_required(&self.obj, "get_insertion_point", ());
                Python::with_gil(|py| {
                    ret.extract::<PyRef<'_, PyOpInsertionPoint>>(py)
                        .expect("Python get_insertion_point() must return an OpInsertionPoint")
                        .inner
                })
            }

            fn set_insertion_point(&mut self, point: OpInsertionPoint) {
                call_required(
                    &self.obj,
                    "set_insertion_point",
                    (PyOpInsertionPoint { inner: point },),
                );
            }
        }
    };
}
pub(crate) use impl_python_inserter;

impl_python_inserter!(PyInserter);

// ---------------------------------------------------------------------------
// PyIRInserter — `pliron.irbuild.IRInserter`
// ---------------------------------------------------------------------------

/// `pliron.irbuild.IRInserter`: the native [`IRInserter`] whose (optional)
/// listener is a duck-typed Python object.
#[pyclass(unsendable, name = "IRInserter")]
pub struct PyIRInserter {
    pub(crate) inner: IRInserter<PyInsertionListener>,
    /// The Python listener object, retained so it can be read back. The *live*
    /// listener (a [`PyInsertionListener`] wrapping this same object) lives
    /// inside `inner` — this is a shared handle, not a copy.
    pub(crate) listener_obj: Option<Py<PyAny>>,
}

/// The shared `Inserter`-surface `#[pymethods]` for a `#[pyclass]` whose `inner`
/// implements [`Inserter`] (plus `is_modified`/`mark_modified` and the listener
/// getter/setter). Written once; used by [`PyIRInserter`] here and
/// `PyIRRewriter` in the sibling `rewriter` module.
///
/// `$listener` is the Rust dispatch struct wrapping the Python listener object
/// (see [`super::listener`]).
macro_rules! delegating_inserter_pymethods {
    ($py_ty:ty, $listener:ty) => {
        #[pymethods]
        impl $py_ty {
            /// Append `op` at the insertion point and advance the point past it.
            fn append_operation(&mut self, op: &PyOperation) -> PyResult<()> {
                let ctx = crate::get_ctx()?;
                self.inner.append_operation(ctx, op.ptr);
                Ok(())
            }

            /// Insert `op` at the insertion point *without* advancing it.
            fn insert_operation(&mut self, op: &PyOperation) -> PyResult<()> {
                let ctx = crate::get_ctx()?;
                self.inner.insert_operation(ctx, op.ptr);
                Ok(())
            }

            /// Insert (an unlinked) `block` at `insertion_point`.
            fn insert_block(
                &mut self,
                insertion_point: &PyBlockInsertionPoint,
                block: &PyBasicBlock,
            ) -> PyResult<()> {
                let ctx = crate::get_ctx()?;
                self.inner
                    .insert_block(ctx, insertion_point.inner, block.ptr);
                Ok(())
            }

            /// Create a new block at `insertion_point`; the op insertion point
            /// moves to the end of the new block. `arg_types` elements may be
            /// generic `Type`s or concrete type wrappers.
            #[pyo3(signature = (insertion_point, arg_types, label=None))]
            fn create_block(
                &mut self,
                insertion_point: &PyBlockInsertionPoint,
                arg_types: Vec<Bound<'_, PyAny>>,
                label: Option<&str>,
            ) -> PyResult<PyBasicBlock> {
                let arg_types = crate::types::type_handles_from_any(&arg_types)?;
                let ctx = crate::get_ctx_mut()?;
                let label = label
                    .map(|s| Identifier::try_from(s.to_string()))
                    .transpose()
                    .map_err(crate::to_py_err)?;
                let ptr = self
                    .inner
                    .create_block(ctx, insertion_point.inner, label, arg_types);
                Ok(PyBasicBlock { ptr })
            }

            /// The current op insertion point.
            fn get_insertion_point(&self) -> PyOpInsertionPoint {
                PyOpInsertionPoint {
                    inner: self.inner.get_insertion_point(),
                }
            }

            /// Is the op insertion point set?
            fn is_insertion_point_set(&self) -> bool {
                self.inner.is_insertion_point_set()
            }

            /// The block the next insertion will occur in, if known.
            fn get_insertion_block(&self) -> PyResult<Option<PyBasicBlock>> {
                let ctx = crate::get_ctx()?;
                Ok(self
                    .inner
                    .get_insertion_block(ctx)
                    .map(|ptr| PyBasicBlock { ptr }))
            }

            /// Set the op insertion point.
            fn set_insertion_point(&mut self, point: &PyOpInsertionPoint) {
                self.inner.set_insertion_point(point.inner);
            }

            /// Has the IR been modified through this object?
            fn is_modified(&self) -> bool {
                self.inner.is_modified()
            }

            /// Mark the IR as modified.
            fn mark_modified(&mut self) {
                self.inner.mark_modified();
            }

            /// The attached Python listener object, or `None`.
            #[getter]
            fn listener(&self, py: Python<'_>) -> Option<Py<PyAny>> {
                self.listener_obj.as_ref().map(|obj| obj.clone_ref(py))
            }

            /// Attach `listener` (any object implementing the listener
            /// protocol's `notify_*` hooks). Replaces any existing listener.
            fn set_listener(&mut self, py: Python<'_>, listener: Py<PyAny>) {
                self.inner
                    .set_listener(<$listener>::new(listener.clone_ref(py)));
                self.listener_obj = Some(listener);
            }
        }
    };
}
pub(crate) use delegating_inserter_pymethods;

delegating_inserter_pymethods!(PyIRInserter, PyInsertionListener);

#[pymethods]
impl PyIRInserter {
    /// Create an inserter, optionally at `insertion_point` and with a duck-typed
    /// Python `listener` (any object with `notify_*` hooks).
    #[new]
    #[pyo3(signature = (insertion_point=None, listener=None))]
    fn new(
        py: Python<'_>,
        insertion_point: Option<PyRef<'_, PyOpInsertionPoint>>,
        listener: Option<Py<PyAny>>,
    ) -> Self {
        let point = insertion_point.map(|p| p.inner).unwrap_or_default();
        let mut inner = IRInserter::<PyInsertionListener>::new(point);
        if let Some(obj) = &listener {
            inner.set_listener(PyInsertionListener::new(obj.clone_ref(py)));
        }
        PyIRInserter {
            inner,
            listener_obj: listener,
        }
    }
}
