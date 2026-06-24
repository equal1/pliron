//! Assembly crate that exposes pliron as the `pliron` Python extension module.
//!
//! Build with maturin:
//!
//! ```shell
//! cd pliron-python
//! maturin develop        # install into current virtualenv (debug)
//! maturin build --release  # produce a wheel
//! ```

use pyo3::prelude::*;

/// The native extension module, installed as `pliron._pliron`.
///
/// The public API is the `pliron` package: its `__init__.py` shim re-exports
/// everything registered here, so users never import `_pliron` directly.
#[pymodule]
fn _pliron(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register all core IR types (Context, Operation, BasicBlock, Region, Value,
    // Type, Attribute, IRBuilder) and the PlironError exception.
    ::pliron::python::register_core_types(m)?;

    // Register any dialect-specific Python classes that used py_class_registration!.
    ::pliron::python::register_all_classes(m)?;


    Ok(())
}
