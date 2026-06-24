use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, Pat, ReturnType, Signature, Type, Visibility, parse2};

use crate::py_type_mapper::{
    ParamKind, classify, pymap_path, return_mentions_self, substitute_self,
};

/// Entry point for `#[pliron_type_impl]`.
///
/// Emits the original `impl` block unchanged, then generates a
/// `#[cfg(feature = "python")] #[pyo3::pymethods] impl Py<Name> { ... }` block.
///
/// **Key difference from `pliron_attr_impl`**: types are stored as `Ptr<TypeObj>`,
/// so instance methods always require `&Context` to deref. The wrapper therefore
/// always injects ctx and returns `PyResult<T>` for instance methods.
pub(crate) fn pliron_type_impl(item: impl Into<TokenStream>) -> syn::Result<TokenStream> {
    let item: ItemImpl = parse2(item.into())?;

    let rust_ty = extract_self_type(&item.self_ty)?;
    let py_ty_name = format_ident!("Py{}", rust_ty);

    let mut py_methods: Vec<TokenStream> = Vec::new();
    let mut method_errors: Vec<TokenStream> = Vec::new();
    for impl_item in &item.items {
        if let ImplItem::Fn(method) = impl_item {
            if !matches!(method.vis, Visibility::Public(_)) {
                continue;
            }
            match gen_py_method(&method.sig, &rust_ty) {
                Ok(ts) => py_methods.push(ts),
                Err(e) => method_errors.push(e.to_compile_error()),
            }
        }
    }

    let py_block = if py_methods.is_empty() {
        quote! {}
    } else {
        quote! {
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl #py_ty_name {
                #(#py_methods)*
            }
        }
    };

    Ok(quote! {
        #item
        #(#method_errors)*
        #py_block
    })
}

fn extract_self_type(ty: &Type) -> syn::Result<syn::Ident> {
    if let Type::Path(tp) = ty {
        if let Some(last) = tp.path.segments.last() {
            return Ok(last.ident.clone());
        }
    }
    Err(syn::Error::new_spanned(
        ty,
        "#[pliron_type_impl] requires a concrete type path (e.g. `impl MyType`)",
    ))
}

enum SelfKind {
    Ref,
    RefMut,
    Static,
}

fn gen_py_method(sig: &Signature, rust_ty: &syn::Ident) -> syn::Result<TokenStream> {
    let method_name = &sig.ident;
    let self_kind = classify_self(sig);

    // Instance methods on types ALWAYS need ctx for `ptr.deref(ctx)`.
    let mut needs_ctx = !matches!(self_kind, SelfKind::Static);

    let mut py_params: Vec<TokenStream> = Vec::new();
    let mut call_args: Vec<TokenStream> = Vec::new();
    let pymap = pymap_path();

    let non_self_args: Vec<&FnArg> = sig
        .inputs
        .iter()
        .filter(|a| !matches!(a, FnArg::Receiver(_)))
        .collect();

    for arg in non_self_args {
        let FnArg::Typed(pat_ty) = arg else { continue };
        let param_name = extract_pat_ident(&pat_ty.pat)?;
        let param_ty = substitute_self(&pat_ty.ty, rust_ty);

        match classify(&param_ty) {
            Some(ParamKind::ContextParam) => {
                needs_ctx = true;
                call_args.push(quote! { ctx });
            }
            Some(ParamKind::Trivial) => {
                py_params.push(quote! { #param_name: #param_ty });
                call_args.push(quote! { #param_name });
            }
            Some(ParamKind::PyMapped) => {
                py_params.push(quote! {
                    #param_name: <#param_ty as #pymap>::Borrowed<'_>
                });
                call_args.push(quote! {
                    <#param_ty as #pymap>::from_py(#param_name)
                });
            }
            None => {
                return Err(syn::Error::new_spanned(
                    &pat_ty.ty,
                    "#[pliron_type_impl]: unsupported parameter shape",
                ));
            }
        }
    }

    let ctx_inject = if needs_ctx {
        quote! { let ctx = ::pliron::python::get_ctx()?; }
    } else {
        quote! {}
    };

    // The wrapper holds a `TypedHandle<#rust_ty>`, so `deref(ctx)` yields a
    // `Ref<#rust_ty>` directly — no downcast needed.
    let downcast_stmt = match &self_kind {
        SelfKind::Ref | SelfKind::RefMut => quote! {
            let __inner = self.ptr.deref(ctx);
        },
        SelfKind::Static => quote! {},
    };

    let ret_info = map_return_type(&sig.output, rust_ty, needs_ctx)?;
    let py_ret_ty = &ret_info.py_ret_ty;
    let wrap_result = &ret_info.wrap_result;

    let call_expr = match &self_kind {
        SelfKind::Static => quote! { #rust_ty::#method_name(#(#call_args),*) },
        _ => quote! { __inner.#method_name(#(#call_args),*) },
    };

    let self_param = match &self_kind {
        SelfKind::Ref | SelfKind::RefMut => quote! { &self, },
        SelfKind::Static => quote! {},
    };

    let static_attr = if matches!(&self_kind, SelfKind::Static) {
        quote! { #[staticmethod] }
    } else {
        quote! {}
    };

    let body = quote! {
        #ctx_inject
        #downcast_stmt
        let __result = #call_expr;
        #wrap_result
    };

    Ok(quote! {
        #static_attr
        fn #method_name(#self_param #(#py_params),*) -> #py_ret_ty {
            #body
        }
    })
}

fn classify_self(sig: &Signature) -> SelfKind {
    for arg in &sig.inputs {
        if let FnArg::Receiver(r) = arg {
            return if r.mutability.is_some() {
                SelfKind::RefMut
            } else {
                SelfKind::Ref
            };
        }
    }
    SelfKind::Static
}

fn extract_pat_ident(pat: &Pat) -> syn::Result<&syn::Ident> {
    if let Pat::Ident(pi) = pat {
        return Ok(&pi.ident);
    }
    Err(syn::Error::new_spanned(
        pat,
        "#[pliron_type_impl]: only simple identifier patterns are supported in function parameters",
    ))
}

struct ReturnInfo {
    py_ret_ty: TokenStream,
    wrap_result: TokenStream,
}

/// Wrap the return in `PyResult<>` when ctx is being injected (i.e. the body uses `?`),
/// so that `let ctx = get_ctx()?;` can early-return.
fn map_return_type(
    ret: &ReturnType,
    rust_ty: &syn::Ident,
    always_pyresult: bool,
) -> syn::Result<ReturnInfo> {
    let inner_ty: Option<&Type> = match ret {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(ty),
    };

    let Some(ty) = inner_ty else {
        return Ok(if always_pyresult {
            ReturnInfo {
                py_ret_ty: quote!(::pliron::pyo3::PyResult<()>),
                wrap_result: quote! { Ok(()) },
            }
        } else {
            ReturnInfo {
                py_ret_ty: quote!(()),
                wrap_result: quote! {},
            }
        });
    };

    if let Some(ok_ty) = extract_result_ok(ty) {
        let ok_ty = substitute_self(ok_ty, rust_ty);
        let inner = map_inner_return(&ok_ty)?;
        let py_inner = &inner.py_ty;
        let converter = &inner.converter;
        return Ok(ReturnInfo {
            py_ret_ty: quote!(::pliron::pyo3::PyResult<#py_inner>),
            wrap_result: quote! {
                __result.map(|__val| { #converter }).map_err(::pliron::python::to_py_err)
            },
        });
    }

    let ty = if return_mentions_self(ty) {
        substitute_self(ty, rust_ty)
    } else {
        ty.clone()
    };
    let inner = map_inner_return(&ty)?;
    let py_ty_out = &inner.py_ty;
    let converter = &inner.converter;

    if always_pyresult {
        Ok(ReturnInfo {
            py_ret_ty: quote!(::pliron::pyo3::PyResult<#py_ty_out>),
            wrap_result: quote! { let __val = __result; Ok(#converter) },
        })
    } else {
        Ok(ReturnInfo {
            py_ret_ty: quote!(#py_ty_out),
            wrap_result: quote! { let __val = __result; #converter },
        })
    }
}

struct InnerReturn {
    py_ty: TokenStream,
    converter: TokenStream,
}

fn map_inner_return(ty: &Type) -> syn::Result<InnerReturn> {
    let pymap = pymap_path();
    match classify(ty) {
        Some(ParamKind::ContextParam) => Err(syn::Error::new_spanned(
            ty,
            "#[pliron_type_impl]: `&Context` cannot be a return type",
        )),
        Some(ParamKind::Trivial) => Ok(InnerReturn {
            py_ty: quote!(#ty),
            converter: quote! { __val },
        }),
        Some(ParamKind::PyMapped) => {
            if let Type::Tuple(tt) = ty {
                if tt.elems.is_empty() {
                    return Ok(InnerReturn {
                        py_ty: quote!(()),
                        converter: quote! {},
                    });
                }
            }
            Ok(InnerReturn {
                py_ty: quote!(<#ty as #pymap>::Owned),
                converter: quote!(<#ty as #pymap>::into_py(__val)),
            })
        }
        None => Err(syn::Error::new_spanned(
            ty,
            "#[pliron_type_impl]: unsupported return shape",
        )),
    }
}

fn extract_result_ok(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty {
        let last = tp.path.segments.last()?;
        if last.ident != "Result" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(ab) = &last.arguments {
            if let Some(syn::GenericArgument::Type(ok_ty)) = ab.args.first() {
                return Some(ok_ty);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

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
        let ts = pliron_type_impl(item).unwrap();
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
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl PyIntegerType {
                fn width(&self) -> ::pliron::pyo3::PyResult<u32> {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    let __result = __inner.width();
                    let __val = __result;
                    Ok(__val)
                }
                #[staticmethod]
                fn get(
                    width: u32,
                ) -> ::pliron::pyo3::PyResult<
                    <TypedHandle<IntegerType> as ::pliron::python::PyMap>::Owned,
                > {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __result = IntegerType::get(ctx, width);
                    let __val = __result;
                    Ok(<TypedHandle<IntegerType> as ::pliron::python::PyMap>::into_py(__val))
                }
            }
        "#]]
        .assert_eq(&got);
    }
}
