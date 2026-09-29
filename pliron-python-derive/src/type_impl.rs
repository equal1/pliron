//! `#[pymethods]` generation mirroring a type's `impl` block.

use proc_macro2::TokenStream;
use quote::quote;

use crate::impl_common::{ImplKind, InstanceAccess, gen_impl};

const KIND: ImplKind = ImplKind {
    macro_name: "py_type_impl",
    instance_needs_ctx: true,
    instance_access: |_rust_ty, mutable| {
        // Types are uniqued and immutable once created, so `&mut self` methods
        // can't be wrapped.
        (!mutable).then(|| InstanceAccess {
            receiver: quote! { &self },
            // The wrapper holds a `TypedHandle<T>`, so `deref(ctx)` yields a
            // `Ref<T>` directly — no downcast needed.
            bind_inner: quote! { let __inner = self.ptr.deref(ctx); },
        })
    },
};

/// Generate `#[pymethods]` for a type `impl` block; see [`gen_impl`].
///
/// Types are stored as `TypedHandle<T>`, so instance methods always need `ctx`
/// to deref, and their wrappers therefore always return `PyResult`.
pub(crate) fn gen_type_impl(
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
    fn instance_and_static_methods() {
        let item = quote! {
            impl IntegerType {
                pub fn width(&self) -> u32 {
                    self.width
                }
                pub fn get(ctx: &mut Context, width: u32) -> TypedHandle<Self> {
                    IntegerType::get_impl(ctx, width)
                }
                fn private_ignored(&self) -> u32 {
                    0
                }
            }
        };
        let ts = gen_type_impl(item, true).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            impl IntegerType {
                pub fn width(&self) -> u32 {
                    self.width
                }
                pub fn get(ctx: &mut Context, width: u32) -> TypedHandle<Self> {
                    IntegerType::get_impl(ctx, width)
                }
                fn private_ignored(&self) -> u32 {
                    0
                }
            }
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyIntegerType {
                fn width(&self) -> ::pliron_python::pyo3::PyResult<u32> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    let __result = __inner.width();
                    let __val = __result;
                    Ok(__val)
                }
                #[staticmethod]
                fn get(
                    width: u32,
                ) -> ::pliron_python::pyo3::PyResult<
                    <TypedHandle<IntegerType> as ::pliron_python::PyMap>::Owned,
                > {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let __result = IntegerType::get(ctx, width);
                    let __val = __result;
                    Ok(<TypedHandle<IntegerType> as ::pliron_python::PyMap>::into_py(__val))
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn explicit_unit_return() {
        // Regression: an explicit `-> ()` in a fallible wrapper used to end in `Ok()`.
        let item = quote! {
            impl IntegerType {
                pub fn f(&self) -> () {}
            }
        };
        let ts = gen_type_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyIntegerType {
                fn f(&self) -> ::pliron_python::pyo3::PyResult<()> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    let __result = __inner.f();
                    Ok(())
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn self_in_result_and_vec() {
        let item = quote! {
            impl IntegerType {
                pub fn try_get(ctx: &mut Context, width: u32) -> Result<TypedHandle<Self>> {
                    todo!()
                }
                pub fn all(ctx: &Context, others: Vec<TypedHandle<Self>>) -> Vec<TypedHandle<Self>> {
                    todo!()
                }
            }
        };
        let ts = gen_type_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyIntegerType {
                #[staticmethod]
                fn try_get(
                    width: u32,
                ) -> ::pliron_python::pyo3::PyResult<
                    <TypedHandle<IntegerType> as ::pliron_python::PyMap>::Owned,
                > {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let __result = IntegerType::try_get(ctx, width);
                    __result
                        .map(|__val| {
                            <TypedHandle<IntegerType> as ::pliron_python::PyMap>::into_py(__val)
                        })
                        .map_err(::pliron_python::to_py_err)
                }
                #[staticmethod]
                fn all(
                    others: <Vec<TypedHandle<IntegerType>> as ::pliron_python::PyMap>::Borrowed<'_>,
                ) -> ::pliron_python::pyo3::PyResult<
                    <Vec<TypedHandle<IntegerType>> as ::pliron_python::PyMap>::Owned,
                > {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __result = IntegerType::all(
                        ctx,
                        <Vec<TypedHandle<IntegerType>> as ::pliron_python::PyMap>::from_py(others),
                    );
                    let __val = __result;
                    Ok(<Vec<TypedHandle<IntegerType>> as ::pliron_python::PyMap>::into_py(__val))
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn instance_method_with_mut_ctx_param() {
        // The receiver needs a shared `ctx`, the parameter a mutable one: mutable wins.
        let item = quote! {
            impl IntegerType {
                pub fn bump(&self, ctx: &mut Context) -> u32 {
                    todo!()
                }
            }
        };
        let ts = gen_type_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl PyIntegerType {
                fn bump(&self) -> ::pliron_python::pyo3::PyResult<u32> {
                    let ctx = ::pliron_python::get_ctx_mut()?;
                    let __inner = self.ptr.deref(ctx);
                    let __result = __inner.bump(ctx);
                    let __val = __result;
                    Ok(__val)
                }
            }
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn mut_self_method() {
        let item = quote! {
            impl IntegerType {
                pub fn set_width(&mut self, width: u32) {
                    self.width = width;
                }
            }
        };
        let ts = gen_type_impl(item, false).unwrap();
        let f = syn::parse2::<syn::File>(ts).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            ::core::compile_error! {
                "py_type_impl: `&mut self` methods are not supported"
            }
        "#]]
        .assert_eq(&got);
    }
}
