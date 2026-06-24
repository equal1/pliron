//! Classify a Rust type for the `pliron_attr_impl` / `pliron_type_impl` /
//! `pliron_op_impl` Python-wrapper macros.
//!
//! Everything beyond a small set of syntactic specials is handled via the
//! [`PyMap`](../../pliron/python/trait.PyMap.html) trait at the call site of the
//! generated code — this module deliberately knows almost nothing about
//! domain types.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, PathArguments, Type};

/// How a Rust type is treated when generating a Python wrapper.
pub(crate) enum ParamKind {
    /// `&Context` or `&mut Context`. The macro drops this from the Python
    /// signature and supplies it from the thread-local active context.
    ContextParam,
    /// A type that pyo3 handles natively (primitive integer/float/bool, `String`,
    /// `&str`, or a `Vec` / `Option` whose innermost element is also trivial).
    /// The macro emits the type as-is in the Python signature and uses identity
    /// conversion both directions.
    Trivial,
    /// Anything else. The macro defers to `PyMap` at the call site:
    /// `<T as PyMap>::Owned` for returns and `<T as PyMap>::Borrowed<'_>` for params.
    PyMapped,
}

/// Classify a type. Returns `None` only for shapes the macro cannot route at all
/// (e.g. an unexpected `Type::ImplTrait`).
pub(crate) fn classify(ty: &Type) -> Option<ParamKind> {
    if is_context_ref(ty) {
        return Some(ParamKind::ContextParam);
    }
    if is_trivial(ty) {
        return Some(ParamKind::Trivial);
    }
    Some(ParamKind::PyMapped)
}

/// `&Context` or `&mut Context`.
fn is_context_ref(ty: &Type) -> bool {
    let Type::Reference(r) = ty else { return false };
    let Type::Path(tp) = &*r.elem else { return false };
    tp.path
        .segments
        .last()
        .is_some_and(|s| s.ident == "Context")
}

/// Recursively decide whether `ty` is PyO3-trivial: a primitive, `String`, `&str`,
/// or a `Vec` / `Option` whose element is itself trivial.
fn is_trivial(ty: &Type) -> bool {
    match ty {
        Type::Reference(r) => {
            // `&str` only.
            if let Type::Path(tp) = &*r.elem {
                if let Some(last) = tp.path.segments.last() {
                    return last.ident == "str";
                }
            }
            false
        }
        Type::Path(tp) => {
            let Some(last) = tp.path.segments.last() else {
                return false;
            };
            let ident = last.ident.to_string();
            match ident.as_str() {
                "bool" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16"
                | "i32" | "i64" | "i128" | "isize" | "f32" | "f64" | "String" => true,
                "Vec" | "Option" => single_generic_arg(&last.arguments)
                    .map(is_trivial)
                    .unwrap_or(false),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Extract the single generic argument from angle-bracket args (e.g. `Vec<T>` → `T`).
fn single_generic_arg(args: &PathArguments) -> Option<&Type> {
    let PathArguments::AngleBracketed(ab) = args else {
        return None;
    };
    if ab.args.len() != 1 {
        return None;
    }
    if let GenericArgument::Type(t) = &ab.args[0] {
        Some(t)
    } else {
        None
    }
}

/// Substitute `Self` → `concrete` in a type expression. Used by the impl-block
/// macros to convert `fn foo() -> Self` and `fn foo() -> TypePtr<Self>` into
/// concrete return types the trait machinery can resolve.
pub(crate) fn substitute_self(ty: &Type, concrete: &syn::Ident) -> Type {
    match ty {
        Type::Path(tp) => {
            let mut tp = tp.clone();
            if tp.path.is_ident("Self") {
                return syn::parse_quote!(#concrete);
            }
            for seg in &mut tp.path.segments {
                if let PathArguments::AngleBracketed(ab) = &mut seg.arguments {
                    for arg in &mut ab.args {
                        if let GenericArgument::Type(inner) = arg {
                            *inner = substitute_self(inner, concrete);
                        }
                    }
                }
            }
            Type::Path(tp)
        }
        Type::Reference(r) => {
            let mut r = r.clone();
            *r.elem = substitute_self(&r.elem, concrete);
            Type::Reference(r)
        }
        Type::Tuple(tt) => {
            let mut tt = tt.clone();
            for elem in &mut tt.elems {
                *elem = substitute_self(elem, concrete);
            }
            Type::Tuple(tt)
        }
        _ => ty.clone(),
    }
}

/// True if the return type is `Self` or `Result<Self, _>` / `Result<Self>`.
/// (Used by the macros to know when to emit `Self` substitutions.)
pub(crate) fn return_mentions_self(ty: &Type) -> bool {
    match ty {
        Type::Path(tp) => {
            if tp.path.is_ident("Self") {
                return true;
            }
            tp.path.segments.iter().any(|seg| {
                if let PathArguments::AngleBracketed(ab) = &seg.arguments {
                    ab.args.iter().any(|a| {
                        if let GenericArgument::Type(t) = a {
                            return_mentions_self(t)
                        } else {
                            false
                        }
                    })
                } else {
                    false
                }
            })
        }
        _ => false,
    }
}

/// The path `::pliron::python::PyMap` written out for `quote!` reuse.
pub(crate) fn pymap_path() -> TokenStream {
    quote!(::pliron::python::PyMap)
}
