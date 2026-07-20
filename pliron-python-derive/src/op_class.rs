//! `#[pyclass]` wrapper generation for IR operations.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::registration::gen_registration;

/// Generate a `#[pyclass]` wrapper for an operation, auto-registered via
/// `PY_CLASS_REGISTRATIONS`.
///
/// The Python class name matches the Rust struct name (e.g. `ModuleOp`).
/// The generated Rust struct is `Py<StructName>` (e.g. `PyModuleOp`) and is placed
/// at module scope so that users can add extra `#[pymethods]` blocks (e.g. constructors).
///
/// Provides:
/// - `from_operation(op)` — check the operation's id and wrap
/// - `operation()` — unwrap to a generic `Operation`
/// - `__str__`, `__repr__`
pub(crate) fn gen_py_op_class(
    struct_name: &syn::Ident,
    dialect_name: &str,
    op_name: &str,
) -> TokenStream {
    let py_class_name = struct_name.to_string();
    let full_name = format!("{}.{}", dialect_name, op_name);
    let py_struct = format_ident!("Py{}", struct_name);
    let registration = gen_registration(&py_struct, &full_name);

    quote! {
        #[::pliron_python::pyo3::pyclass(unsendable, name = #py_class_name, crate = "::pliron_python::pyo3")]
        pub struct #py_struct {
            pub(crate) ptr: ::pliron::context::Ptr<::pliron::operation::Operation>,
        }

        #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
        impl #py_struct {
            /// Check the operation's kind and wrap it in a typed handle.
            #[staticmethod]
            fn from_operation(
                op: &::pliron_python::operation::PyOperation,
            ) -> ::pliron_python::pyo3::PyResult<Self> {
                let ctx = ::pliron_python::get_ctx()?;
                let actual = ::pliron::operation::Operation::get_opid(op.ptr, ctx);
                let expected = <#struct_name as ::pliron::op::Op>::get_opid_static();
                if actual != expected {
                    return Err(::pliron_python::PlironError::new_err(
                        ::pliron::alloc::format!("Expected op {}, got {}", expected, actual),
                    ));
                }
                Ok(Self { ptr: op.ptr })
            }

            /// Get the underlying generic Operation handle.
            fn operation(&self) -> ::pliron_python::operation::PyOperation {
                ::pliron_python::operation::PyOperation { ptr: self.ptr }
            }

            fn __str__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
                let ctx = ::pliron_python::get_ctx()?;
                Ok(::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.ptr, ctx)
                ))
            }

            fn __repr__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
                self.__str__()
            }
        }

        impl ::pliron_python::PyMap for #struct_name {
            type Owned = #py_struct;
            type Borrowed<'py> = ::pliron_python::pyo3::PyRef<'py, #py_struct>;

            fn into_py(self) -> #py_struct {
                #py_struct {
                    ptr: <#struct_name as ::pliron::op::Op>::get_operation(&self),
                }
            }

            fn from_py(py: ::pliron_python::pyo3::PyRef<'_, #py_struct>) -> Self {
                <#struct_name as ::pliron::op::Op>::from_operation(py.ptr)
            }
        }

        #registration
    }
}
