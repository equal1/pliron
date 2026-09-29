//! `#[pymethods]` generation mirroring an attribute's, type's or op's `impl`
//! block. The three macros differ only in the handful of settings captured by
//! [`ImplKind`]; everything else is shared here.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    FnArg, ImplItem, ItemImpl, Pat, ReceiverKind, ReturnType, Signature, Type, Visibility,
    parse_quote, parse2,
};

use crate::py_type_mapper::{ParamKind, classify, pymap_path, substitute_self};

/// The per-kind parts of `#[pymethods]` generation.
pub(crate) struct ImplKind {
    /// Macro name used as the prefix of error messages, e.g. `"py_attr_impl"`.
    pub macro_name: &'static str,
    /// Instance methods always need `ctx` (e.g. types deref their `Ptr` through it).
    pub instance_needs_ctx: bool,
    /// How an instance method's wrapper reaches the Rust value, given the Rust
    /// self type and whether the method takes `&mut self`. `None` means methods
    /// with that receiver can't be wrapped for this kind.
    pub instance_access: fn(&syn::Ident, bool) -> Option<InstanceAccess>,
}

/// How an instance method's wrapper reaches the Rust value it calls the method on.
pub(crate) struct InstanceAccess {
    /// The wrapper's receiver: `&self` or `&mut self`.
    pub receiver: TokenStream,
    /// The statement binding `__inner` (the Rust value whose method is called)
    /// from the wrapper's `self`. `ctx` is in scope if it was needed.
    pub bind_inner: TokenStream,
}

/// Generate a `#[pyo3::pymethods] impl Py<Name> { ... }` block containing Python
/// wrappers for every `pub` function of the given `impl` block.
///
/// `emit_original` controls whether the original `impl` block is re-emitted in
/// front of the generated code: true for the attribute form (e.g. `#[py_attr_impl]`
/// on a local item), false for the reflect-export form (the `impl` lives in a
/// foreign crate and must not be duplicated).
///
/// A method whose signature can't be wrapped is reported as a `compile_error!`
/// on that method, while the remaining methods (and the original `impl`) are
/// still emitted.
pub(crate) fn gen_impl(
    item: impl Into<TokenStream>,
    emit_original: bool,
    kind: &ImplKind,
) -> syn::Result<TokenStream> {
    let input = item.into();
    let item: ItemImpl = parse2(input.clone())?;

    let rust_ty = extract_self_type(&item.self_ty, kind)?;
    let py_ty_name = format_ident!("Py{}", rust_ty);

    let PyMethods { methods, errors } = gen_py_methods(&item, &rust_ty, kind);

    let original = emit_original.then_some(input);
    let py_block = (!methods.is_empty()).then(|| {
        quote! {
            #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
            impl #py_ty_name {
                #(#methods)*
            }
        }
    });

    Ok(quote! {
        #original
        #(#errors)*
        #py_block
    })
}

/// The Python wrappers generated for an `impl` block's `pub` methods.
///
/// Errors are kept apart from the wrappers because they must be emitted outside
/// the `#[pymethods]` block: pyo3 rejects macro invocations (such as
/// `compile_error!`) as items inside it.
struct PyMethods {
    /// One wrapper `fn` per method that could be wrapped.
    methods: Vec<TokenStream>,
    /// One `compile_error!` per method that couldn't.
    errors: Vec<TokenStream>,
}

/// Generate a Python wrapper for every `pub` fn of `item`, collecting the
/// wrappers and the per-method errors separately (see [`PyMethods`]). Non-`pub`
/// fns and other impl items (consts, types, macros) are skipped.
fn gen_py_methods(item: &ItemImpl, rust_ty: &syn::Ident, kind: &ImplKind) -> PyMethods {
    let mut py_methods = PyMethods {
        methods: Vec::new(),
        errors: Vec::new(),
    };
    for method in item.items.iter().filter_map(|impl_item| match impl_item {
        ImplItem::Fn(method) if matches!(method.vis, Visibility::Public(_)) => Some(method),
        _ => None,
    }) {
        match gen_py_method(&method.sig, rust_ty, kind) {
            Ok(ts) => py_methods.methods.push(ts),
            Err(e) => py_methods.errors.push(e.into_compile_error()),
        }
    }
    py_methods
}

/// The name of the type an `impl` block is for: the last path segment, so
/// `impl foo::MyAttr` yields `MyAttr`. The Python wrapper is named `Py` + this.
/// Errors for anything other than a type path (e.g. `impl &MyAttr`).
fn extract_self_type(ty: &Type, kind: &ImplKind) -> syn::Result<syn::Ident> {
    if let Type::Path(tp) = ty
        && let Some(last) = tp.path.segments.last()
    {
        return Ok(last.ident.clone());
    }
    Err(syn::Error::new_spanned(
        ty,
        format!(
            "{} requires a concrete type path (e.g. `impl MyType`)",
            kind.macro_name
        ),
    ))
}

/// How a wrapper obtains the active pliron `Context`, ordered from weakest to
/// strongest so that the needs of a method's parts combine by taking the `max`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CtxAccess {
    /// The context isn't needed.
    None,
    /// Fetched with `get_ctx()`.
    Shared,
    /// Fetched with `get_ctx_mut()`.
    Mut,
}

impl CtxAccess {
    /// What a `&Context` or `&mut Context` parameter of type `ty` needs.
    fn of_context_param(ty: &Type) -> Self {
        if is_mut_ref(ty) {
            CtxAccess::Mut
        } else {
            CtxAccess::Shared
        }
    }

    /// What the receiver needs: `Shared` for instance methods of a kind whose
    /// instance access goes through `ctx`, `None` otherwise.
    fn of_receiver(self_kind: &SelfKind, kind: &ImplKind) -> Self {
        if kind.instance_needs_ctx && !matches!(self_kind, SelfKind::Static) {
            CtxAccess::Shared
        } else {
            CtxAccess::None
        }
    }

    /// The statement binding `ctx`, or nothing.
    fn fetch(self) -> TokenStream {
        match self {
            CtxAccess::None => quote! {},
            CtxAccess::Shared => quote! { let ctx = ::pliron_python::get_ctx()?; },
            CtxAccess::Mut => quote! { let ctx = ::pliron_python::get_ctx_mut()?; },
        }
    }

    /// Whether [`Self::fetch`] uses `?`, making the wrapper return `PyResult`.
    fn is_fallible(self) -> bool {
        self != CtxAccess::None
    }
}

enum SelfKind {
    Ref,
    RefMut,
    Static,
}

/// Generate the Python wrapper `fn` for one Rust method with signature `sig`.
///
/// The wrapper:
/// - takes `&self` or `&mut self` for instance methods (per [`ImplKind::instance_access`]),
///   and is a `#[staticmethod]` otherwise;
/// - drops `&Context` / `&mut Context` parameters from the Python signature and
///   fetches the active context instead (`get_ctx()` or `get_ctx_mut()`);
/// - takes other parameters as-is when pyo3 handles them natively, or through
///   `PyMap::Borrowed` otherwise;
/// - binds `__inner` as [`ImplKind::instance_access`] says and calls the method
///   on it; the receiver mutability is handled there too, or rejected;
/// - converts the result with [`map_return_type`], returning `PyResult` when the
///   method returns `Result` or the wrapper uses `?` to fetch the context.
///
/// The signature is first rewritten by [`normalise_signature`], so all later
/// steps see concrete types and a single unit form.
///
/// Errors for parameters that aren't simple identifiers.
fn gen_py_method(
    sig: &Signature,
    rust_ty: &syn::Ident,
    kind: &ImplKind,
) -> syn::Result<TokenStream> {
    let sig = &normalise_signature(sig, rust_ty);
    let method_name = &sig.ident;
    let self_kind = classify_self(sig);

    let ParamInfo {
        py_params,
        call_args,
        ctx_access: params_ctx_access,
    } = map_params(sig, kind)?;
    let ctx_access = params_ctx_access.max(CtxAccess::of_receiver(&self_kind, kind));
    let ctx_inject = ctx_access.fetch();

    let ReturnInfo {
        py_ret_ty,
        wrap_result,
    } = map_return_type(&sig.output, ctx_access.is_fallible(), kind)?;

    let ReceiverInfo {
        static_attr,
        self_param,
        bind_inner,
        call_expr,
    } = map_receiver(sig, self_kind, rust_ty, &call_args, kind)?;

    Ok(quote! {
        #static_attr
        fn #method_name(#self_param #(#py_params),*) -> #py_ret_ty {
            #ctx_inject
            #bind_inner
            let __result = #call_expr;
            #wrap_result
        }
    })
}

/// A copy of `sig` with `Self` replaced by `rust_ty` in every parameter and
/// return type, and an omitted return type made an explicit `-> ()`. The
/// receiver is left as written.
fn normalise_signature(sig: &Signature, rust_ty: &syn::Ident) -> Signature {
    let mut sig = sig.clone();
    for arg in &mut sig.inputs {
        if let FnArg::Typed(pat_ty) = arg {
            *pat_ty.ty = substitute_self(&pat_ty.ty, rust_ty);
        }
    }
    sig.output = match &sig.output {
        ReturnType::Default => parse_quote!(-> ()),
        ReturnType::Type(arrow, ty) => {
            ReturnType::Type(*arrow, Box::new(substitute_self(ty, rust_ty)))
        }
    };
    sig
}

/// The receiver-dependent parts of a wrapper `fn`.
struct ReceiverInfo {
    /// `#[staticmethod]` for methods without a receiver, empty otherwise.
    static_attr: TokenStream,
    /// The wrapper's receiver followed by a comma (`&self,` / `&mut self,`), or empty.
    self_param: TokenStream,
    /// The statement binding `__inner` for instance methods, or empty.
    bind_inner: TokenStream,
    /// The call of the Rust method, on `__inner` or on the type.
    call_expr: TokenStream,
}

/// Decide how the wrapper receives `self` and calls the Rust method with
/// `call_args`: as a `#[staticmethod]` calling `rust_ty::method(..)`, or as an
/// instance method calling `__inner.method(..)` bound per
/// [`ImplKind::instance_access`]. Errors when the kind rejects the receiver.
fn map_receiver(
    sig: &Signature,
    self_kind: SelfKind,
    rust_ty: &syn::Ident,
    call_args: &[TokenStream],
    kind: &ImplKind,
) -> syn::Result<ReceiverInfo> {
    let method_name = &sig.ident;
    let unsupported = |receiver: &str| {
        syn::Error::new_spanned(
            sig.receiver(),
            format!(
                "{}: `{receiver}` methods are not supported",
                kind.macro_name
            ),
        )
    };
    let access = match self_kind {
        SelfKind::Static => None,
        SelfKind::Ref => {
            Some((kind.instance_access)(rust_ty, false).ok_or_else(|| unsupported("&self"))?)
        }
        SelfKind::RefMut => {
            Some((kind.instance_access)(rust_ty, true).ok_or_else(|| unsupported("&mut self"))?)
        }
    };

    Ok(match access {
        None => ReceiverInfo {
            static_attr: quote! { #[staticmethod] },
            self_param: quote! {},
            bind_inner: quote! {},
            call_expr: quote! { #rust_ty::#method_name(#(#call_args),*) },
        },
        Some(InstanceAccess {
            receiver,
            bind_inner,
        }) => ReceiverInfo {
            static_attr: quote! {},
            self_param: quote! { #receiver, },
            bind_inner,
            call_expr: quote! { __inner.#method_name(#(#call_args),*) },
        },
    })
}

/// The wrapper-side view of a Rust method's non-`self` parameters.
struct ParamInfo {
    /// The Python-visible parameters of the wrapper (`name: Ty`).
    py_params: Vec<TokenStream>,
    /// The arguments passed to the Rust method, one per parameter.
    call_args: Vec<TokenStream>,
    /// What the `Context` parameters need: the strongest over all of them.
    ctx_access: CtxAccess,
}

/// Map each non-`self` parameter of `sig` to its wrapper parameter (if any) and
/// the argument passed to the Rust method. `Context` parameters are dropped from
/// the Python signature and receive `ctx` instead.
fn map_params(sig: &Signature, kind: &ImplKind) -> syn::Result<ParamInfo> {
    let mut params = ParamInfo {
        py_params: Vec::new(),
        call_args: Vec::new(),
        ctx_access: CtxAccess::None,
    };
    let pymap = pymap_path();

    for arg in &sig.inputs {
        let FnArg::Typed(pat_ty) = arg else { continue };
        let param_name = extract_pat_ident(&pat_ty.pat, kind)?;
        let param_ty = &*pat_ty.ty;

        match classify(param_ty) {
            Some(ParamKind::ContextParam) => {
                params.ctx_access = params.ctx_access.max(CtxAccess::of_context_param(param_ty));
                params.call_args.push(quote! { ctx });
            }
            Some(ParamKind::Trivial) => {
                params.py_params.push(quote! { #param_name: #param_ty });
                params.call_args.push(quote! { #param_name });
            }
            Some(ParamKind::PyMapped) => {
                params.py_params.push(quote! {
                    #param_name: <#param_ty as #pymap>::Borrowed<'_>
                });
                params.call_args.push(quote! {
                    <#param_ty as #pymap>::from_py(#param_name)
                });
            }
            None => {
                return Err(syn::Error::new_spanned(
                    &pat_ty.ty,
                    format!("{}: unsupported parameter shape", kind.macro_name),
                ));
            }
        }
    }
    Ok(params)
}

/// True for a `&mut T` reference type.
fn is_mut_ref(ty: &Type) -> bool {
    matches!(ty, Type::Reference(r) if r.mutability.is_some())
}

fn classify_self(sig: &Signature) -> SelfKind {
    match sig.receiver() {
        // `Receiver::mutability` is the `mut` of `mut self`; the `mut` of
        // `&mut self` lives in the reference kind.
        Some(r) if matches!(r.kind, ReceiverKind::Reference(_, _, Some(_))) => SelfKind::RefMut,
        Some(_) => SelfKind::Ref,
        None => SelfKind::Static,
    }
}

fn extract_pat_ident<'a>(pat: &'a Pat, kind: &ImplKind) -> syn::Result<&'a syn::Ident> {
    if let Pat::Ident(pi) = pat {
        return Ok(&pi.ident);
    }
    Err(syn::Error::new_spanned(
        pat,
        format!(
            "{}: only simple identifier patterns are supported in function parameters",
            kind.macro_name
        ),
    ))
}

struct ReturnInfo {
    py_ret_ty: TokenStream,
    wrap_result: TokenStream,
}

/// Map the (normalised) Rust return type to the wrapper's return type and the
/// statements converting `__result` into it. When the wrapper is `fallible`
/// (its body uses `?`, e.g. to fetch `ctx`), non-`Result` returns are wrapped
/// in `PyResult` too.
fn map_return_type(ret: &ReturnType, fallible: bool, kind: &ImplKind) -> syn::Result<ReturnInfo> {
    let ReturnType::Type(_, ty) = ret else {
        unreachable!("normalise_signature makes every return type explicit")
    };

    if is_unit(ty) {
        return Ok(if fallible {
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

    // Result<T, E> → PyResult<<T as PyMap>::Owned> with map_err.
    if let Some(ok_ty) = extract_result_ok(ty) {
        let InnerReturn { py_ty, converter } = map_inner_return(ok_ty, kind)?;
        return Ok(ReturnInfo {
            py_ret_ty: quote!(::pliron_python::pyo3::PyResult<#py_ty>),
            wrap_result: quote! {
                __result.map(|__val| { #converter }).map_err(::pliron_python::to_py_err)
            },
        });
    }

    let InnerReturn { py_ty, converter } = map_inner_return(ty, kind)?;

    Ok(if fallible {
        ReturnInfo {
            py_ret_ty: quote!(::pliron_python::pyo3::PyResult<#py_ty>),
            wrap_result: quote! { let __val = __result; Ok(#converter) },
        }
    } else {
        ReturnInfo {
            py_ret_ty: py_ty,
            wrap_result: quote! { let __val = __result; #converter },
        }
    })
}

struct InnerReturn {
    py_ty: TokenStream,
    converter: TokenStream,
}

fn map_inner_return(ty: &Type, kind: &ImplKind) -> syn::Result<InnerReturn> {
    let pymap = pymap_path();
    match classify(ty) {
        Some(ParamKind::ContextParam) => Err(syn::Error::new_spanned(
            ty,
            format!("{}: `&Context` cannot be a return type", kind.macro_name),
        )),
        Some(ParamKind::Trivial) => Ok(InnerReturn {
            py_ty: quote!(#ty),
            converter: quote! { __val },
        }),
        Some(ParamKind::PyMapped) => {
            // `()` is classified as PyMapped (it isn't recognized as primitive),
            // so handle the unit case explicitly here (reached for `Result<()>`).
            if is_unit(ty) {
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
            format!("{}: unsupported return shape", kind.macro_name),
        )),
    }
}

fn is_unit(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tt) if tt.elems.is_empty())
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
