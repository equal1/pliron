//! `#[pymethods]` generation mirroring an op's `impl` block.

use proc_macro2::TokenStream;
use quote::quote;

use crate::impl_common::{CtxAccess, ImplKind, InstanceAccess, InstanceReceiver, gen_impl};

const KIND: ImplKind = ImplKind {
    macro_name: "py_op_impl",
    instance_access: |rust_ty, receiver| {
        // `MyOp::from_operation(ptr)` reconstructs the Rust-side op handle. It is a
        // fresh copy and the op's data lives in the context, so a `&mut self`
        // method only needs a mutable local, not a mutable wrapper.
        let mutability = matches!(receiver, InstanceReceiver::RefMut).then(|| quote! { mut });
        Some(InstanceAccess {
            receiver: quote! { &self },
            bind_inner: quote! {
                let #mutability __inner = <#rust_ty as ::pliron::op::Op>::from_operation(self.ptr);
            },
            ctx_access: CtxAccess::None,
        })
    },
};

/// Generate `#[pymethods]` for an op `impl` block; see [`gen_impl`].
///
/// Ops are stored as `Ptr<Operation>`; `Self` returns go through `PyMap`.
pub(crate) fn gen_op_impl(
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
            impl ModuleOp {
                pub fn new(ctx: &mut Context, name: String) -> Self {
                    todo!()
                }
                pub fn get_name(&self, ctx: &Context) -> String {
                    todo!()
                }
                fn private_ignored(&self) -> u32 {
                    0
                }
            }
        };
        let ts = gen_op_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyModuleOp {
                #[staticmethod]
                fn new(
                    name: String,
                ) -> ::pliron_python::pyo3::PyResult<<ModuleOp as ::pliron_python::PyMap>::Owned> {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let __result = ModuleOp::new(ctx, name);
                    let __val = __result;
                    Ok(<ModuleOp as ::pliron_python::PyMap>::into_py(__val))
                }
                fn get_name(&self) -> ::pliron_python::pyo3::PyResult<String> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = <ModuleOp as ::pliron::op::Op>::from_operation(self.ptr);
                    let __result = __inner.get_name(ctx);
                    let __val = __result;
                    Ok(__val)
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn explicit_unit_return_with_ctx() {
        let item = quote! {
            impl ModuleOp {
                pub fn touch(&self, ctx: &Context) -> () {
                    todo!()
                }
                pub fn check(&self, ctx: &Context) -> Result<()> {
                    todo!()
                }
                pub fn try_new(ctx: &mut Context) -> Result<Self, Error> {
                    todo!()
                }
            }
        };
        let ts = gen_op_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyModuleOp {
                fn touch(&self) -> ::pliron_python::pyo3::PyResult<()> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = <ModuleOp as ::pliron::op::Op>::from_operation(self.ptr);
                    let __result = __inner.touch(ctx);
                    Ok(())
                }
                fn check(&self) -> ::pliron_python::pyo3::PyResult<()> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = <ModuleOp as ::pliron::op::Op>::from_operation(self.ptr);
                    let __result = __inner.check(ctx);
                    __result.map(|__val| {}).map_err(::pliron_python::to_py_err)
                }
                #[staticmethod]
                fn try_new() -> ::pliron_python::pyo3::PyResult<
                    <ModuleOp as ::pliron_python::PyMap>::Owned,
                > {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let __result = ModuleOp::try_new(ctx);
                    __result
                        .map(|__val| { <ModuleOp as ::pliron_python::PyMap>::into_py(__val) })
                        .map_err(::pliron_python::to_py_err)
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn mut_self_method() {
        let item = quote! {
            impl ModuleOp {
                pub fn set_name(&mut self, ctx: &mut Context, name: String) {
                    todo!()
                }
            }
        };
        let ts = gen_op_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyModuleOp {
                fn set_name(&self, name: String) -> ::pliron_python::pyo3::PyResult<()> {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let mut __inner = <ModuleOp as ::pliron::op::Op>::from_operation(self.ptr);
                    let __result = __inner.set_name(ctx, name);
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn mut_self_method_with_shared_ctx_param() {
        // `&mut self` doesn't need `ctx`, so a `&Context` parameter fetches it shared.
        let item = quote! {
            impl ModuleOp {
                pub fn rename(&mut self, ctx: &Context, name: String) {
                    todo!()
                }
            }
        };
        let ts = gen_op_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyModuleOp {
                fn rename(&self, name: String) -> ::pliron_python::pyo3::PyResult<()> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let mut __inner = <ModuleOp as ::pliron::op::Op>::from_operation(self.ptr);
                    let __result = __inner.rename(ctx, name);
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }
}
