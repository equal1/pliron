//! `#[pymethods]` generation mirroring an op's `impl` block.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ItemImpl, Pat, ReturnType, Signature, Type, Visibility, parse2};

use crate::py_type_mapper::{
    ParamKind, classify, pymap_path, return_mentions_self, substitute_self,
};

/// Generate a `#[pyo3::pymethods] impl Py<Name> { ... }` block containing Python
/// wrappers for every `pub` function of the given `impl` block.
///
/// `emit_original` controls whether the original `impl` block is re-emitted in
/// front of the generated code: true for the attribute form (`#[py_op_impl]` on
/// a local item), false for the reflect-export form (the `impl` lives in a
/// foreign crate and must not be duplicated).
///
/// Ops are stored as `Ptr<Operation>` — analogous to the typed-pointer form of
/// types. Instance methods may or may not need `&Context` depending on what the
/// underlying Rust method calls; the macro detects use of `&Context`/`&mut Context`
/// parameters and injects `ctx` only when needed. `Self` returns are wrapped as
/// `PyMyOp { ptr: my_op.op }`.
pub(crate) fn gen_op_impl(
    item: impl Into<TokenStream>,
    emit_original: bool,
) -> syn::Result<TokenStream> {
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
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl #py_ty_name {
                #(#py_methods)*
            }
        }
    };

    let original = if emit_original {
        quote! { #item }
    } else {
        quote! {}
    };

    Ok(quote! {
        #original
        #(#method_errors)*
        #py_block
    })
}

fn extract_self_type(ty: &Type) -> syn::Result<syn::Ident> {
    if let Type::Path(tp) = ty
        && let Some(last) = tp.path.segments.last()
    {
        return Ok(last.ident.clone());
    }
    Err(syn::Error::new_spanned(
        ty,
        "py_op_impl requires a concrete type path (e.g. `impl MyOp`)",
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

    let mut needs_ctx = false;
    let mut needs_mut_ctx = false;
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
                if is_mut_ref(&param_ty) {
                    needs_mut_ctx = true;
                }
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
                    "py_op_impl: unsupported parameter shape",
                ));
            }
        }
    }

    let ctx_inject = if needs_mut_ctx {
        quote! { let ctx = ::pliron_python::get_ctx_mut()?; }
    } else if needs_ctx {
        quote! { let ctx = ::pliron_python::get_ctx()?; }
    } else {
        quote! {}
    };

    // For instance methods we need a Rust-side handle (`__inner: MyOp`) to call
    // the user's method. `MyOp::from_operation(ptr)` reconstructs it.
    let downcast_stmt = match &self_kind {
        SelfKind::Ref | SelfKind::RefMut => quote! {
            let __inner = <#rust_ty as ::pliron::op::Op>::from_operation(self.ptr);
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

/// True for a `&mut T` reference type. Used to decide whether a `Context`
/// parameter needs `get_ctx_mut()` rather than the shared `get_ctx()`.
fn is_mut_ref(ty: &Type) -> bool {
    matches!(ty, Type::Reference(r) if r.mutability.is_some())
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
        "py_op_impl: only simple identifier patterns are supported in function parameters",
    ))
}

struct ReturnInfo {
    py_ret_ty: TokenStream,
    wrap_result: TokenStream,
}

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
                py_ret_ty: quote!(::pliron_python::pyo3::PyResult<()>),
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
            py_ret_ty: quote!(::pliron_python::pyo3::PyResult<#py_inner>),
            wrap_result: quote! {
                __result.map(|__val| { #converter }).map_err(::pliron_python::to_py_err)
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
            py_ret_ty: quote!(::pliron_python::pyo3::PyResult<#py_ty_out>),
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
            "py_op_impl: `&Context` cannot be a return type",
        )),
        Some(ParamKind::Trivial) => Ok(InnerReturn {
            py_ty: quote!(#ty),
            converter: quote! { __val },
        }),
        Some(ParamKind::PyMapped) => {
            if let Type::Tuple(tt) = ty
                && tt.elems.is_empty()
            {
                return Ok(InnerReturn {
                    py_ty: quote!(()),
                    converter: quote! {},
                });
            }
            Ok(InnerReturn {
                py_ty: quote!(<#ty as #pymap>::Owned),
                converter: quote!(<#ty as #pymap>::into_py(__val)),
            })
        }
        None => Err(syn::Error::new_spanned(
            ty,
            "py_op_impl: unsupported return shape",
        )),
    }
}

fn extract_result_ok(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty {
        let last = tp.path.segments.last()?;
        if last.ident != "Result" {
            return None;
        }
        if let syn::PathArguments::AngleBracketed(ab) = &last.arguments
            && let Some(syn::GenericArgument::Type(ok_ty)) = ab.args.first()
        {
            return Some(ok_ty);
        }
    }
    None
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
}
