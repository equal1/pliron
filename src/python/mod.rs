//! Python bindings for pliron, enabled by the `python` cargo feature.
//!
//! Exposes the core IR types — [Context](crate::context::Context),
//! [Operation](crate::operation::Operation), [BasicBlock](crate::basic_block::BasicBlock),
//! [Region](crate::region::Region), [Value](crate::value::Value),
//! [Type](crate::r#type::Type) and [Attribute](crate::attribute::Attribute) —
//! as Python classes backed by a single thread-local active context.
//!
//! # Lifetime model
//! Use `with pliron.Context() as ctx:` in Python to activate the compiler context.
//! Only one context may be active per OS thread at a time.
//! All Python IR objects resolve their context implicitly through this global.

pub mod attributes;
pub mod basic_block;
pub mod context;
pub mod irbuild;
pub mod operation;
pub mod py_map;
pub mod region;
pub mod types;
pub mod value;

pub use py_map::{PyMap, PyMapTarget, PyTypeWrapper};

use std::cell::Cell;

use alloc::format;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;

// ---------------------------------------------------------------------------
// Global active-context store (GIL-protected thread-local)
// ---------------------------------------------------------------------------

std::thread_local! {
    static ACTIVE_CTX_PTR: Cell<*mut crate::context::Context> =
        Cell::new(std::ptr::null_mut());
}

/// Get a shared reference to the active context.
///
/// # Errors
/// Returns `PlironError` if no context is currently active.
pub fn get_ctx() -> PyResult<&'static crate::context::Context> {
    ACTIVE_CTX_PTR.with(|c| {
        let ptr = c.get();
        if ptr.is_null() {
            Err(PlironError::new_err(
                "No active pliron context. Use `with pliron.Context() as ctx:`",
            ))
        } else {
            // Safety: pointer is valid while PyContext owns it and is active.
            // Python's GIL ensures single-threaded access.
            Ok(unsafe { &*ptr })
        }
    })
}

/// Get a mutable reference to the active context.
///
/// # Errors
/// Returns `PlironError` if no context is currently active.
pub fn get_ctx_mut() -> PyResult<&'static mut crate::context::Context> {
    ACTIVE_CTX_PTR.with(|c| {
        let ptr = c.get();
        if ptr.is_null() {
            Err(PlironError::new_err(
                "No active pliron context. Use `with pliron.Context() as ctx:`",
            ))
        } else {
            // Safety: same as get_ctx; GIL prevents concurrent access.
            Ok(unsafe { &mut *ptr })
        }
    })
}

/// Activate `ctx` as the thread-local active context.
///
/// # Errors
/// Returns `PlironError` if a context is already active.
pub fn set_active_ctx(ctx: *mut crate::context::Context) -> PyResult<()> {
    ACTIVE_CTX_PTR.with(|c| {
        if !c.get().is_null() {
            return Err(PlironError::new_err(
                "A pliron context is already active in this thread. \
                 Nested or parallel contexts are not supported.",
            ));
        }
        c.set(ctx);
        Ok(())
    })
}

/// Deactivate the current thread-local context.
pub fn clear_active_ctx() {
    ACTIVE_CTX_PTR.with(|c| c.set(std::ptr::null_mut()));
}

// ---------------------------------------------------------------------------
// PlironError — the single Python exception type for all pliron errors
// ---------------------------------------------------------------------------

pyo3::create_exception!(
    pliron,
    PlironError,
    PyException,
    "Base exception for all pliron compiler errors."
);

/// Convert a pliron `Result` error into a `PlironError` Python exception.
pub fn to_py_err(err: crate::result::Error) -> PyErr {
    PlironError::new_err(format!("{}", err))
}

// ---------------------------------------------------------------------------
// Registration trait — used by derive-generated Python classes to identify
// themselves so the pliron-python assembly crate can install them.
// ---------------------------------------------------------------------------

/// Opaque registration entry produced by derive-generated code.
/// The assembly crate's `#[pymodule]` iterates these to register all classes.
pub struct PyClassRegistration {
    /// Human-readable class name (e.g. `"llvm.AddOp"`)
    pub name: &'static str,
    /// Registers the class into the given parent module.
    pub register: fn(&Bound<'_, PyModule>) -> PyResult<()>,
}

/// Get `parent.<name>` as a module, creating and attaching it if absent.
///
/// This only creates the *attribute* (`pliron.builtin` etc.); making the
/// submodule reachable via `import pliron.builtin` / `from pliron.builtin
/// import X` additionally requires a `sys.modules` entry, which the Python
/// shim (`python/pliron/__init__.py`) adds generically for every native
/// submodule at import time.
pub fn get_or_create_submodule<'py>(
    parent: &Bound<'py, PyModule>,
    name: &str,
) -> PyResult<Bound<'py, PyModule>> {
    if let Ok(existing) = parent.getattr(name) {
        if let Ok(module) = existing.downcast_into::<PyModule>() {
            return Ok(module);
        }
    }
    let module = PyModule::new(parent.py(), name)?;
    parent.add_submodule(&module)?;
    Ok(module)
}

/// Collect all registration entries into a module.
/// Called from the `pliron-python` assembly crate's `#[pymodule]`.
///
/// Each derive-generated class is routed into its dialect's submodule
/// (`pliron.builtin`, `pliron.llvm`, …), keyed by the `"dialect.name"` prefix
/// the registration already carries. Submodules are created on demand, so
/// linkme's arbitrary iteration order does not matter and dialect authors need
/// no extra declaration — linking the crate is enough.
pub fn register_all_classes(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let route = |reg: &PyClassRegistration| -> PyResult<()> {
        match reg.name.split_once('.') {
            Some((dialect, _)) => {
                let sub = get_or_create_submodule(m, dialect)?;
                (reg.register)(&sub)
            }
            // No dialect prefix: register at the top level.
            None => (reg.register)(m),
        }
    };
    #[cfg(not(target_family = "wasm"))]
    {
        for reg in crate::python::statics::PY_CLASS_REGISTRATIONS.iter() {
            route(reg)?;
        }
    }
    #[cfg(target_family = "wasm")]
    {
        for reg in pyo3::inventory::iter::<&'static PyClassRegistration>() {
            route(reg)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Distributed-slice for automatic class registration (mirrors context_registration!)
// ---------------------------------------------------------------------------

#[cfg(not(target_family = "wasm"))]
pub mod statics {
    use super::PyClassRegistration;

    #[::pliron::linkme::distributed_slice]
    #[linkme(crate = ::pliron::linkme)]
    pub static PY_CLASS_REGISTRATIONS: [PyClassRegistration] = [..];
}

#[cfg(target_family = "wasm")]
mod _wasm_statics {
    use super::PyClassRegistration;
    ::pliron::inventory::collect!(&'static PyClassRegistration);
}

/// Macro to register a derive-generated Python class.
/// Mirrors `context_registration!` in style.
///
/// Usage: `py_class_registration!(name = "llvm.AddOp", register = AddOpPy::register_python_class);`
#[macro_export]
macro_rules! py_class_registration {
    (name = $name:expr, register = $fn:expr) => {
        const _: () = {
            #[cfg_attr(
                not(target_family = "wasm"),
                ::pliron::linkme::distributed_slice(
                    ::pliron::python::statics::PY_CLASS_REGISTRATIONS
                ),
                linkme(crate = ::pliron::linkme)
            )]
            static _REG: ::pliron::python::PyClassRegistration =
                ::pliron::python::PyClassRegistration {
                    name: $name,
                    register: $fn,
                };

            #[cfg(target_family = "wasm")]
            ::pliron::inventory::submit! { &_REG }
        };
    };
}

// ---------------------------------------------------------------------------
// Add all pyclass types to a PyModule (called by the assembly crate)
// ---------------------------------------------------------------------------

/// Register all core IR Python types into `m`.
///
/// Layout rule: classes from files directly under `src/python/` go on the
/// top-level `pliron` module; each *folder* (Rust module directory) becomes one
/// Python submodule, populated flat by its files (e.g. `pliron.irbuild`).
pub fn register_core_types(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<context::PyContext>()?;
    m.add_class::<operation::PyOperation>()?;
    m.add_class::<basic_block::PyBasicBlock>()?;
    m.add_class::<basic_block::PyBlockOpsIter>()?;
    m.add_class::<region::PyRegion>()?;
    m.add_class::<region::PyRegionBlocksIter>()?;
    m.add_class::<value::PyValue>()?;
    m.add_class::<types::PyType>()?;
    m.add_class::<attributes::PyAttribute>()?;
    irbuild::register(m)?;
    m.add(
        "PlironError",
        m.py().get_type::<PlironError>(),
    )?;
    Ok(())
}
