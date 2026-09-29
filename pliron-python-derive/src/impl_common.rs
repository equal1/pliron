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
#[derive(Debug)]
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
        let sig = normalise_signature(&method.sig, rust_ty);
        match analyse_method(&sig, rust_ty, kind) {
            Ok(desc) => py_methods.methods.push(emit_method(&desc, rust_ty)),
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

/// What the generator knows about one Rust method, gathered by [`analyse_method`]
/// and turned into a wrapper by [`emit_method`].
#[derive(Debug)]
struct MethodDesc {
    /// The Rust method's name, also used for the wrapper.
    name: syn::Ident,
    /// How the wrapper receives `self`.
    receiver: Receiver,
    /// The non-`self` parameters, in order.
    params: Vec<Param>,
    /// How the wrapper gets the context: the strongest need of the parameters
    /// and the receiver.
    ctx_access: CtxAccess,
    /// What the Rust method returns.
    ret: Return,
}

/// How a wrapper receives `self`.
#[derive(Debug)]
enum Receiver {
    /// No receiver: the wrapper is a `#[staticmethod]`.
    Static,
    /// `&self` (or `self`), reached as the [`ImplKind`] says.
    Shared(InstanceAccess),
    /// `&mut self`, reached as the [`ImplKind`] says.
    Mut(InstanceAccess),
}

/// One non-`self` parameter of the Rust method.
#[derive(Debug)]
struct Param {
    name: syn::Ident,
    /// The parameter type, with `Self` replaced.
    ty: Type,
    kind: ParamKind,
}

/// What a Rust method returns.
#[derive(Debug)]
struct Return {
    /// The returned value, or the `Ok` value when [`Self::in_result`].
    value: ReturnValue,
    /// Whether the method returns `Result<value, _>`.
    in_result: bool,
}

#[derive(Debug)]
enum ReturnValue {
    /// `()`.
    Unit,
    /// Any other type.
    Value(ValueKind, Box<Type>),
}

/// How a returned value is converted to Python.
#[derive(Debug)]
enum ValueKind {
    /// pyo3 handles it natively: returned as-is.
    Trivial,
    /// Converted with `PyMap::into_py`.
    PyMapped,
}

/// How a wrapper obtains the active pliron `Context`, ordered from weakest to
/// strongest so that the needs of a method's parts combine by taking the `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CtxAccess {
    /// The context isn't needed.
    None,
    /// Fetched with `get_ctx()`.
    Shared,
    /// Fetched with `get_ctx_mut()`.
    Mut,
}

impl CtxAccess {
    /// What a parameter needs: `Mut` for `&mut Context`, `Shared` for
    /// `&Context`, `None` for anything else.
    fn of_param(param: &Param) -> Self {
        match param.kind {
            ParamKind::ContextParam if is_mut_ref(&param.ty) => CtxAccess::Mut,
            ParamKind::ContextParam => CtxAccess::Shared,
            ParamKind::Trivial | ParamKind::PyMapped => CtxAccess::None,
        }
    }

    /// What the receiver needs: `Shared` for instance methods of a kind whose
    /// instance access goes through `ctx`, `None` otherwise.
    fn of_receiver(receiver: &Receiver, kind: &ImplKind) -> Self {
        if kind.instance_needs_ctx && !matches!(receiver, Receiver::Static) {
            CtxAccess::Shared
        } else {
            CtxAccess::None
        }
    }
}

/// Describe the Rust method with the normalised signature `sig` (see
/// [`normalise_signature`]).
///
/// Errors for parameters that aren't simple identifiers, `&Context` returns,
/// and receivers that `kind` can't wrap.
fn analyse_method(
    sig: &Signature,
    rust_ty: &syn::Ident,
    kind: &ImplKind,
) -> syn::Result<MethodDesc> {
    let params = analyse_params(sig, kind)?;
    let ret = analyse_return(&sig.output, kind)?;
    let receiver = analyse_receiver(sig, rust_ty, kind)?;
    let ctx_access = params
        .iter()
        .map(CtxAccess::of_param)
        .fold(CtxAccess::of_receiver(&receiver, kind), CtxAccess::max);
    Ok(MethodDesc {
        name: sig.ident.clone(),
        receiver,
        params,
        ctx_access,
        ret,
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

/// Describe the receiver, reached per [`ImplKind::instance_access`]. Errors
/// when the kind rejects it.
fn analyse_receiver(
    sig: &Signature,
    rust_ty: &syn::Ident,
    kind: &ImplKind,
) -> syn::Result<Receiver> {
    let Some(receiver) = sig.receiver() else {
        return Ok(Receiver::Static);
    };
    // `Receiver::mutability` is the `mut` of `mut self`; the `mut` of
    // `&mut self` lives in the reference kind.
    let mutable = matches!(receiver.kind, ReceiverKind::Reference(_, _, Some(_)));
    let access = (kind.instance_access)(rust_ty, mutable).ok_or_else(|| {
        syn::Error::new_spanned(
            receiver,
            format!(
                "{}: `{}` methods are not supported",
                kind.macro_name,
                if mutable { "&mut self" } else { "&self" }
            ),
        )
    })?;
    Ok(if mutable {
        Receiver::Mut(access)
    } else {
        Receiver::Shared(access)
    })
}

/// Describe each non-`self` parameter of `sig`. Errors for parameters that
/// aren't simple identifiers.
fn analyse_params(sig: &Signature, kind: &ImplKind) -> syn::Result<Vec<Param>> {
    let mut params = Vec::new();
    for arg in &sig.inputs {
        let FnArg::Typed(pat_ty) = arg else { continue };
        let name = extract_pat_ident(&pat_ty.pat, kind)?;
        let ty = &*pat_ty.ty;
        let Some(param_kind) = classify(ty) else {
            return Err(syn::Error::new_spanned(
                ty,
                format!("{}: unsupported parameter shape", kind.macro_name),
            ));
        };
        params.push(Param {
            name: name.clone(),
            ty: ty.clone(),
            kind: param_kind,
        });
    }
    Ok(params)
}

/// True for a `&mut T` reference type.
fn is_mut_ref(ty: &Type) -> bool {
    matches!(ty, Type::Reference(r) if r.mutability.is_some())
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

/// Describe the (normalised) Rust return type. Errors for `&Context` returns.
fn analyse_return(ret: &ReturnType, kind: &ImplKind) -> syn::Result<Return> {
    let ReturnType::Type(_, ty) = ret else {
        unreachable!("normalise_signature makes every return type explicit")
    };
    Ok(match extract_result_ok(ty) {
        Some(ok_ty) => Return {
            value: analyse_return_value(ok_ty, kind)?,
            in_result: true,
        },
        None => Return {
            value: analyse_return_value(ty, kind)?,
            in_result: false,
        },
    })
}

fn analyse_return_value(ty: &Type, kind: &ImplKind) -> syn::Result<ReturnValue> {
    // `()` would classify as PyMapped (it isn't recognized as primitive).
    if is_unit(ty) {
        return Ok(ReturnValue::Unit);
    }
    let value_kind = match classify(ty) {
        Some(ParamKind::ContextParam) => {
            return Err(syn::Error::new_spanned(
                ty,
                format!("{}: `&Context` cannot be a return type", kind.macro_name),
            ));
        }
        Some(ParamKind::Trivial) => ValueKind::Trivial,
        Some(ParamKind::PyMapped) => ValueKind::PyMapped,
        None => {
            return Err(syn::Error::new_spanned(
                ty,
                format!("{}: unsupported return shape", kind.macro_name),
            ));
        }
    };
    Ok(ReturnValue::Value(value_kind, Box::new(ty.clone())))
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

/// Generate the Python wrapper `fn` described by `desc`. `rust_ty` is the Rust
/// self type, on which static methods are called.
///
/// The wrapper:
/// - takes `&self` or `&mut self` for instance methods (per [`ImplKind::instance_access`]),
///   and is a `#[staticmethod]` otherwise;
/// - drops `&Context` / `&mut Context` parameters from the Python signature and
///   fetches the active context instead (`get_ctx()` or `get_ctx_mut()`);
/// - takes other parameters as-is when pyo3 handles them natively, or through
///   `PyMap::Borrowed` otherwise;
/// - binds `__inner` as [`ImplKind::instance_access`] says and calls the method
///   on it, or calls it on `rust_ty`;
/// - converts the result, as-is or through `PyMap::into_py`, returning `PyResult`
///   when the method returns `Result` or the wrapper uses `?` to fetch the context.
///
/// The body's locals are `ctx` (the context), `__inner` (the Rust value, bound
/// by the kind), `__result` (what the method returned) and `__val` (the value
/// being converted).
fn emit_method(desc: &MethodDesc, rust_ty: &syn::Ident) -> TokenStream {
    let MethodDesc {
        name,
        receiver,
        params,
        ctx_access,
        ret,
    } = desc;
    let pymap = pymap_path();

    let py_params = params
        .iter()
        .filter_map(|Param { name, ty, kind }| match kind {
            ParamKind::ContextParam => None,
            ParamKind::Trivial => Some(quote! { #name: #ty }),
            ParamKind::PyMapped => Some(quote! { #name: <#ty as #pymap>::Borrowed<'_> }),
        });
    let call_args = params.iter().map(|Param { name, ty, kind }| match kind {
        ParamKind::ContextParam => quote! { ctx },
        ParamKind::Trivial => quote! { #name },
        ParamKind::PyMapped => quote! { <#ty as #pymap>::from_py(#name) },
    });

    let (static_attr, self_param, bind_inner, call_expr) = match receiver {
        Receiver::Static => (
            quote! { #[staticmethod] },
            quote! {},
            quote! {},
            quote! { #rust_ty::#name(#(#call_args),*) },
        ),
        Receiver::Shared(access) | Receiver::Mut(access) => {
            let InstanceAccess {
                receiver,
                bind_inner,
            } = access;
            (
                quote! {},
                quote! { #receiver, },
                bind_inner.clone(),
                quote! { __inner.#name(#(#call_args),*) },
            )
        }
    };

    let fetch_ctx = match ctx_access {
        CtxAccess::None => quote! {},
        CtxAccess::Shared => quote! { let ctx = ::pliron_python::get_ctx()?; },
        CtxAccess::Mut => quote! { let ctx = ::pliron_python::get_ctx_mut()?; },
    };
    // Fetching the context uses `?`, so the wrapper must return `PyResult`.
    let fallible = *ctx_access != CtxAccess::None;

    let py_result = |ty: TokenStream| quote!(::pliron_python::pyo3::PyResult<#ty>);
    // For a value: its Python type, and the expression converting `__val` to it.
    let value = match &ret.value {
        ReturnValue::Unit => None,
        ReturnValue::Value(ValueKind::Trivial, ty) => Some((quote!(#ty), quote! { __val })),
        ReturnValue::Value(ValueKind::PyMapped, ty) => Some((
            quote!(<#ty as #pymap>::Owned),
            quote!(<#ty as #pymap>::into_py(__val)),
        )),
    };
    // A `Result` is returned as `PyResult` whether or not the wrapper is fallible.
    let (py_ret_ty, wrap_result) = match (value, ret.in_result, fallible) {
        (None, false, false) => (quote!(()), quote! {}),
        (None, false, true) => (py_result(quote!(())), quote! { Ok(()) }),
        (None, true, false | true) => (
            py_result(quote!(())),
            quote! { __result.map(|__val| {}).map_err(::pliron_python::to_py_err) },
        ),
        (Some((py_ty, convert)), false, false) => {
            (py_ty, quote! { let __val = __result; #convert })
        }
        (Some((py_ty, convert)), false, true) => (
            py_result(py_ty),
            quote! { let __val = __result; Ok(#convert) },
        ),
        (Some((py_ty, convert)), true, false | true) => (
            py_result(py_ty),
            quote! {
                __result.map(|__val| { #convert }).map_err(::pliron_python::to_py_err)
            },
        ),
    };

    quote! {
        #static_attr
        fn #name(#self_param #(#py_params),*) -> #py_ret_ty {
            #fetch_ctx
            #bind_inner
            let __result = #call_expr;
            #wrap_result
        }
    }
}
