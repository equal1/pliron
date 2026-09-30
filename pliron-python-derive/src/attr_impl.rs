//! `#[pymethods]` generation mirroring an attribute's `impl` block.

use proc_macro2::TokenStream;
use quote::quote;

use crate::impl_common::{CtxAccess, ImplKind, InstanceAccess, InstanceReceiver, gen_impl};

const KIND: ImplKind = ImplKind {
    macro_name: "py_attr_impl",
    instance_access: |_rust_ty, receiver| {
        // The wrapper holds its own copy of the attribute by value, so borrow it
        // directly. A `&mut self` method mutates that copy, which needs `&mut self`
        // on the wrapper too (pyo3 takes a mutable borrow of the Python object).
        Some(match receiver {
            InstanceReceiver::Ref => InstanceAccess {
                receiver: quote! { &self },
                bind_inner: quote! { let __inner = &self.inner; },
                ctx_access: CtxAccess::None,
            },
            InstanceReceiver::RefMut => InstanceAccess {
                receiver: quote! { &mut self },
                bind_inner: quote! { let __inner = &mut self.inner; },
                ctx_access: CtxAccess::None,
            },
        })
    },
};

/// Generate `#[pymethods]` for an attribute `impl` block; see [`gen_impl`].
pub(crate) fn gen_attr_impl(
    item: impl Into<TokenStream>,
    emit_original: bool,
) -> syn::Result<TokenStream> {
    gen_impl(item, emit_original, &KIND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;
    use quote::quote;

    #[test]
    fn static_and_instance_methods() {
        let item = quote! {
            impl StringAttr {
                pub fn new(value: String) -> Self {
                    StringAttr(value)
                }
                pub fn value(&self) -> String {
                    self.0.clone()
                }
                fn private_ignored(&self) -> String {
                    "ignored".to_string()
                }
            }
        };
        let ts = gen_attr_impl(item, true).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl StringAttr {
                pub fn new(value: String) -> Self {
                    StringAttr(value)
                }
                pub fn value(&self) -> String {
                    self.0.clone()
                }
                fn private_ignored(&self) -> String {
                    "ignored".to_string()
                }
            }
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyStringAttr {
                #[staticmethod]
                fn new(value: String) -> <StringAttr as ::pliron_python::PyMap>::Owned {
                    let __result = StringAttr::new(value);
                    let __val = __result;
                    <StringAttr as ::pliron_python::PyMap>::into_py(__val)
                }
                fn value(&self) -> String {
                    let __inner = &self.inner;
                    let __result = __inner.value();
                    let __val = __result;
                    __val
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn mut_self_method() {
        let item = quote! {
            impl StringAttr {
                pub fn set_value(&mut self, value: String) {
                    self.0 = value;
                }
            }
        };
        let ts = gen_attr_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyStringAttr {
                fn set_value(&mut self, value: String) -> () {
                    let __inner = &mut self.inner;
                    let __result = __inner.set_value(value);
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn result_returns_without_ctx() {
        // No `ctx` is fetched, so only the `Result` makes these wrappers fallible.
        let item = quote! {
            impl StringAttr {
                pub fn parse(value: String) -> Result<Self> {
                    todo!()
                }
                pub fn validate(&self) -> Result<()> {
                    todo!()
                }
            }
        };
        let ts = gen_attr_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyStringAttr {
                #[staticmethod]
                fn parse(
                    value: String,
                ) -> ::pliron_python::pyo3::PyResult<<StringAttr as ::pliron_python::PyMap>::Owned> {
                    let __result = StringAttr::parse(value);
                    __result
                        .map(|__val| { <StringAttr as ::pliron_python::PyMap>::into_py(__val) })
                        .map_err(::pliron_python::to_py_err)
                }
                fn validate(&self) -> ::pliron_python::pyo3::PyResult<()> {
                    let __inner = &self.inner;
                    let __result = __inner.validate();
                    __result.map(|__val| {}).map_err(::pliron_python::to_py_err)
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn one_method_fails() {
        // The error is emitted outside the `#[pymethods]` block and the valid
        // method is still wrapped.
        let item = quote! {
            impl StringAttr {
                pub fn pair(&self, (a, b): (u32, u32)) -> u32 {
                    a + b
                }
                pub fn len(&self) -> usize {
                    self.0.len()
                }
            }
        };
        let ts = gen_attr_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            ::core::compile_error! {
                "py_attr_impl: only simple identifier patterns are supported in function parameters"
            }
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyStringAttr {
                fn len(&self) -> usize {
                    let __inner = &self.inner;
                    let __result = __inner.len();
                    let __val = __result;
                    __val
                }
            }
        "#]]
        .assert_eq(&got);
    }
}
