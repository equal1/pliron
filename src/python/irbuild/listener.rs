//! Bridge Python listener objects to the Rust IR-build/rewrite listener traits.
//!
//! The Python side is **duck-typed**: a listener is any Python object
//! implementing (a subset of) the listener protocol — the `notify_*` methods
//! below, mirroring the Rust [`InsertionListener`] / [`RewriteListener`] traits.
//! There is no required base class; the protocol is a Python-side convention
//! (express it with `typing.Protocol` if you want static checking).
//!
//! ```python
//! class Counting:                        # no inheritance required
//!     def __init__(self):
//!         self.n = 0
//!     def notify_operation_inserted(self, op):
//!         self.n += 1
//!
//! rw = pliron.irbuild.IRRewriter(point, listener=Counting())
//! ```
//!
//! Two Rust-side dispatch structs wrap the Python object and implement the Rust
//! traits by calling the same-named method on it:
//!
//! - [`PyInsertionListener`] implements [`InsertionListener`] — the `L` in
//!   [`IRInserter<L>`](crate::irbuild::inserter::IRInserter).
//! - [`PyRewriteListener`] implements [`RewriteListener`] (and, per the
//!   `RewriteListener: InsertionListener` supertrait, [`InsertionListener`]) —
//!   the `L` in [`IRRewriter<L>`](crate::irbuild::rewriter::IRRewriter).
//!
//! The two are *sibling* structs (Rust has no struct inheritance); the shared
//! `InsertionListener` impl is generated once for both. Neither is a
//! `#[pyclass]` — Python never sees them; bindings wrap the user's listener
//! object at intake (e.g. `IRRewriter(listener=obj)`).
//!
//! Dispatch is **defensive**: a hook the object does not define is skipped
//! (implement only what you care about), and a hook that raises is printed and
//! swallowed (a notification must not abort an in-flight IR mutation). Because
//! the wrapped object is optional, both structs are `Default` (a no-op
//! listener), satisfying the `InsertionListener: Default` bound.
//!
//! The two Rust-native listeners are also exposed as plain `#[pyclass]`es —
//! [`PyDummyListener`] (`pliron.irbuild.DummyListener`, a no-op) and
//! [`PyRecorder`] (`pliron.irbuild.Recorder`, records events natively). Their
//! `notify_*` pymethods delegate to the wrapped native listener, so an instance
//! satisfies the protocol and is a valid `listener=` argument.

use pyo3::prelude::*;
use pyo3::types::PyTuple;

use alloc::{format, string::String, vec::Vec};

use crate::{
    basic_block::BasicBlock,
    context::{Context, Ptr},
    irbuild::listener::{DummyListener, InsertionListener, Recorder, RewriteListener},
    operation::Operation,
    region::Region,
    r#type::TypeHandle,
    value::Value,
};

use crate::python::{
    basic_block::PyBasicBlock, operation::PyOperation, region::PyRegion, types::PyType,
    value::PyValue,
};

/// Defensive dispatch: call `method(*args)` on the optional Python listener
/// object if it defines it.
///
/// Missing methods are skipped (the protocol may be partially implemented); a
/// raising callback is reported and swallowed (a notification must not abort an
/// in-flight IR mutation).
fn dispatch<A>(obj: &Option<Py<PyAny>>, method: &str, args: A)
where
    A: for<'py> IntoPyObject<'py, Target = PyTuple>,
{
    let Some(obj) = obj else { return };
    Python::with_gil(|py| {
        let bound = obj.bind(py);
        if !bound.hasattr(method).unwrap_or(false) {
            return;
        }
        if let Err(err) = bound.call_method1(method, args) {
            err.print(py);
        }
    });
}

/// A Python listener object as a Rust [`InsertionListener`]. `Default` is the
/// no-op listener (no wrapped object).
#[derive(Default)]
pub struct PyInsertionListener {
    obj: Option<Py<PyAny>>,
}

/// A Python listener object as a Rust [`RewriteListener`] (and hence also an
/// [`InsertionListener`], per the trait hierarchy). `Default` is the no-op
/// listener.
#[derive(Default)]
pub struct PyRewriteListener {
    obj: Option<Py<PyAny>>,
}

impl PyInsertionListener {
    /// Wrap a Python listener object.
    pub fn new(obj: Py<PyAny>) -> Self {
        Self { obj: Some(obj) }
    }
}

impl PyRewriteListener {
    /// Wrap a Python listener object.
    pub fn new(obj: Py<PyAny>) -> Self {
        Self { obj: Some(obj) }
    }
}

/// The `InsertionListener` impl is identical for both structs, so it is written
/// once here.
macro_rules! impl_py_insertion_listener {
    ($ty:ty) => {
        impl InsertionListener for $ty {
            fn notify_operation_inserted(&mut self, _ctx: &Context, op: Ptr<Operation>) {
                dispatch(&self.obj, "notify_operation_inserted", (PyOperation { ptr: op },));
            }
            fn notify_block_inserted(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
                dispatch(&self.obj, "notify_block_inserted", (PyBasicBlock { ptr: block },));
            }
        }
    };
}

impl_py_insertion_listener!(PyInsertionListener);
impl_py_insertion_listener!(PyRewriteListener);

impl RewriteListener for PyRewriteListener {
    fn notify_operation_erasure(&mut self, _ctx: &Context, op: Ptr<Operation>) {
        dispatch(&self.obj, "notify_operation_erasure", (PyOperation { ptr: op },));
    }

    fn notify_value_use_replacement(
        &mut self,
        _ctx: &Context,
        old_value: Value,
        new_value: Value,
    ) {
        dispatch(
            &self.obj,
            "notify_value_use_replacement",
            (PyValue { val: old_value }, PyValue { val: new_value }),
        );
    }

    fn notify_value_type_change(
        &mut self,
        _ctx: &Context,
        value: Value,
        old_type: TypeHandle,
        new_type: TypeHandle,
    ) {
        dispatch(
            &self.obj,
            "notify_value_type_change",
            (
                PyValue { val: value },
                PyType { ptr: old_type },
                PyType { ptr: new_type },
            ),
        );
    }

    fn notify_block_erasure(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
        dispatch(&self.obj, "notify_block_erasure", (PyBasicBlock { ptr: block },));
    }

    fn notify_region_erasure(&mut self, _ctx: &Context, region: Ptr<Region>) {
        dispatch(&self.obj, "notify_region_erasure", (PyRegion { ptr: region },));
    }

    fn notify_operation_unlinking(&mut self, _ctx: &Context, op: Ptr<Operation>) {
        dispatch(&self.obj, "notify_operation_unlinking", (PyOperation { ptr: op },));
    }

    fn notify_block_unlinking(&mut self, _ctx: &Context, block: Ptr<BasicBlock>) {
        dispatch(&self.obj, "notify_block_unlinking", (PyBasicBlock { ptr: block },));
    }
}

// ---------------------------------------------------------------------------
// Rust-native listeners, exposed as plain Python classes.
//
// Each is a thin `#[pyclass]` wrapper whose `notify_*` pymethods delegate to the
// wrapped native listener — so an instance satisfies the Python listener
// protocol and works as a `listener=` argument, while the recording work
// happens natively in Rust.
// ---------------------------------------------------------------------------

/// A no-op listener (`pliron.irbuild.DummyListener`). Equivalent to passing no
/// listener at all; provided for parity with the Rust `DummyListener`.
#[pyclass(unsendable, name = "DummyListener")]
pub struct PyDummyListener {
    inner: DummyListener,
}

/// A listener that records IR build/rewrite events in order
/// (`pliron.irbuild.Recorder`). Pass an instance as `listener=` and read the log
/// back with `events()` / `len()`.
#[pyclass(unsendable, name = "Recorder")]
pub struct PyRecorder {
    inner: Recorder,
}

/// Generate the `#[pymethods]` block of listener protocol methods for a
/// `#[pyclass]` wrapping a native listener in its `inner` field. Written once,
/// used by both wrappers — each hook just forwards to the native trait method.
macro_rules! delegating_listener_pymethods {
    ($py_ty:ty, $native:ty) => {
        #[pymethods]
        impl $py_ty {
            #[new]
            fn new() -> Self {
                Self { inner: <$native>::default() }
            }

            fn notify_operation_inserted(&mut self, op: &PyOperation) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as InsertionListener>::notify_operation_inserted(
                    &mut self.inner, ctx, op.ptr,
                );
                Ok(())
            }

            fn notify_block_inserted(&mut self, block: &PyBasicBlock) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as InsertionListener>::notify_block_inserted(
                    &mut self.inner, ctx, block.ptr,
                );
                Ok(())
            }

            fn notify_operation_erasure(&mut self, op: &PyOperation) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_operation_erasure(
                    &mut self.inner, ctx, op.ptr,
                );
                Ok(())
            }

            fn notify_value_use_replacement(
                &mut self,
                old_value: &PyValue,
                new_value: &PyValue,
            ) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_value_use_replacement(
                    &mut self.inner, ctx, old_value.val, new_value.val,
                );
                Ok(())
            }

            fn notify_value_type_change(
                &mut self,
                value: &PyValue,
                old_type: &PyType,
                new_type: &PyType,
            ) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_value_type_change(
                    &mut self.inner, ctx, value.val, old_type.ptr, new_type.ptr,
                );
                Ok(())
            }

            fn notify_block_erasure(&mut self, block: &PyBasicBlock) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_block_erasure(
                    &mut self.inner, ctx, block.ptr,
                );
                Ok(())
            }

            fn notify_region_erasure(&mut self, region: &PyRegion) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_region_erasure(
                    &mut self.inner, ctx, region.ptr,
                );
                Ok(())
            }

            fn notify_operation_unlinking(&mut self, op: &PyOperation) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_operation_unlinking(
                    &mut self.inner, ctx, op.ptr,
                );
                Ok(())
            }

            fn notify_block_unlinking(&mut self, block: &PyBasicBlock) -> PyResult<()> {
                let ctx = crate::python::get_ctx()?;
                <$native as RewriteListener>::notify_block_unlinking(
                    &mut self.inner, ctx, block.ptr,
                );
                Ok(())
            }
        }
    };
}

delegating_listener_pymethods!(PyDummyListener, DummyListener);
delegating_listener_pymethods!(PyRecorder, Recorder);

#[pymethods]
impl PyRecorder {
    /// Number of recorded events.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// `True` if no events have been recorded.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Drop all recorded events.
    fn clear(&mut self) {
        self.inner.clear();
    }

    /// The recorded events, in order, each as its debug string.
    fn events(&self) -> Vec<String> {
        self.inner.events.iter().map(|e| format!("{:?}", e)).collect()
    }
}
