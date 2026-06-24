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

/// Top-level `pliron` Python module.
#[pymodule]
fn pliron(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Register all core IR types (Context, Operation, BasicBlock, Region, Value,
    // Type, Attribute, IRBuilder, Rewriter) and the PlironError exception.
    ::pliron::python::register_core_types(m)?;

    // Register any dialect-specific Python classes that used py_class_registration!.
    ::pliron::python::register_all_classes(m)?;

    // Expose the top-level apply_match_rewrite function.
    m.add_function(wrap_pyfunction!(
        ::pliron::python::irbuild::rewriter::py_apply_match_rewrite,
        m
    )?)?;

    Ok(())
}
