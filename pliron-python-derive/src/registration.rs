//! Shared emission of the `PY_CLASS_REGISTRATIONS` entry for a generated class.

use proc_macro2::TokenStream;
use quote::quote;

/// Emit a `PyClassRegistration` for `py_struct` under `full_name`
/// (`"dialect.entity"`), so `pliron_python::register_all_classes` picks the
/// class up at import time and routes it into the right submodule.
pub(crate) fn gen_registration(py_struct: &syn::Ident, full_name: &str) -> TokenStream {
    quote! {
        const _: () = {
            use ::pliron_python::pyo3::prelude::*;

            fn __register_py_class(
                m: &::pliron_python::pyo3::Bound<'_, ::pliron_python::pyo3::types::PyModule>,
            ) -> ::pliron_python::pyo3::PyResult<()> {
                m.add_class::<#py_struct>()?;
                Ok(())
            }

            #[cfg_attr(
                not(target_family = "wasm"),
                ::pliron_python::linkme::distributed_slice(
                    ::pliron_python::statics::PY_CLASS_REGISTRATIONS
                ),
                linkme(crate = ::pliron_python::linkme),
            )]
            static __PY_REG: ::pliron_python::PyClassRegistration =
                ::pliron_python::PyClassRegistration {
                    name: #full_name,
                    register: __register_py_class,
                };
        };
    }
}
