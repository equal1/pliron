//! Python-agnostic "reflection" token exports.
//!
//! `def_op` / `def_attribute` / `def_type` (and thus `pliron_op` /
//! `pliron_attr` / `pliron_type`) emit, for every annotated item, an inert
//! `#[doc(hidden)] #[macro_export]` macro named
//! `__pliron_reflect_<kind>_<Ident>`. The `pliron_op_impl` /
//! `pliron_attr_impl` / `pliron_type_impl` hook macros do the same for `impl`
//! blocks. Invoking such a macro with the path of another macro as its sole
//! argument forwards the item's tokens to that macro, wrapped in a versioned
//! envelope:
//!
//! ```text
//! some_crate::__pliron_reflect_op_MyOp!(::pliron_python::derive::py_op_from_export);
//! // expands to:
//! ::pliron_python::derive::py_op_from_export! {
//!     pliron_reflect_v1,
//!     kind: op,                  // op | attr | ty | op_impl | attr_impl | ty_impl
//!     ident: MyOp,
//!     name: "dialect.my_op",     // absent for *_impl kinds
//!     item: { ...item tokens... }
//! }
//! ```
//!
//! This lets a separate crate (e.g. `pliron-python-derive`, via
//! `pliron-python`) generate bindings for items defined here without this
//! crate — or the crate the items live in — depending on any binding
//! machinery. The exported macros carry no runtime or dependency cost; they
//! are plain token containers.
//!
//! Note: `#[macro_export]` macros live in a flat, crate-root namespace, so two
//! same-named entities in one crate would collide. Op/attr/type struct names
//! are unique per crate in practice; for `impl` blocks, annotate at most one
//! block per type with a `pliron_*_impl` hook.

use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{ImplItem, ItemImpl, parse2};

/// Emit the export macro for a class-like entity (`kind` is `op`/`attr`/`ty`).
/// `full_name` is `"dialect.entity"`; `item` is the (possibly empty) item body
/// forwarded to consumers.
pub(crate) fn class_export(
    kind: &str,
    ident: &syn::Ident,
    full_name: &str,
    item: TokenStream,
) -> TokenStream {
    export(kind, ident, Some(full_name), item)
}

/// Emit the export macro for a `DeriveInput`-shaped entity, forwarding the
/// item definition with its outer attributes stripped (consumers only need the
/// shape: ident, generics, fields).
pub(crate) fn derive_input_export(
    kind: &str,
    input: &syn::DeriveInput,
    dialect_name: &str,
    entity_name: &str,
) -> TokenStream {
    let stripped = syn::DeriveInput {
        attrs: vec![],
        ..input.clone()
    };
    class_export(
        kind,
        &input.ident,
        &format!("{}.{}", dialect_name, entity_name),
        stripped.into_token_stream(),
    )
}

/// The shared implementation of the `pliron_op_impl` / `pliron_attr_impl` /
/// `pliron_type_impl` hook macros: re-emit the `impl` block unchanged and
/// export its tokens (with function bodies stripped) under
/// `__pliron_reflect_<kind>_<SelfTy>`.
pub(crate) fn impl_hook(kind: &str, item: impl Into<TokenStream>) -> syn::Result<TokenStream> {
    let item: ItemImpl = parse2(item.into())?;
    let ident = extract_self_ident(&item)?;
    let export = impl_export(kind, &ident, &item);
    Ok(quote! {
        #item
        #export
    })
}

/// Emit the export macro for an `impl` block (`kind` is
/// `op_impl`/`attr_impl`/`ty_impl`). Function bodies are stripped: consumers
/// only need signatures, and bodies may contain tokens (e.g. `$`) that cannot
/// appear verbatim in a `macro_rules!` expansion.
fn impl_export(kind: &str, ident: &syn::Ident, item: &ItemImpl) -> TokenStream {
    let mut stripped = item.clone();
    for impl_item in &mut stripped.items {
        if let ImplItem::Fn(f) = impl_item {
            f.block = syn::parse_quote!({});
        }
    }
    export(kind, ident, None, stripped.into_token_stream())
}

fn extract_self_ident(item: &ItemImpl) -> syn::Result<syn::Ident> {
    if let syn::Type::Path(tp) = &*item.self_ty
        && let Some(last) = tp.path.segments.last()
    {
        return Ok(last.ident.clone());
    }
    Err(syn::Error::new_spanned(
        &item.self_ty,
        "this hook macro requires a concrete type path (e.g. `impl MyOp`)",
    ))
}

fn export(kind: &str, ident: &syn::Ident, name: Option<&str>, item: TokenStream) -> TokenStream {
    let macro_name = format_ident!("__pliron_reflect_{}_{}", kind, ident);
    let kind_ident = format_ident!("{}", kind);
    let name_field = match name {
        Some(n) => quote! { name: #n, },
        None => quote! {},
    };
    quote! {
        #[doc(hidden)]
        #[macro_export]
        macro_rules! #macro_name {
            ($($cb:tt)+) => {
                $($cb)+ ! {
                    pliron_reflect_v1,
                    kind: #kind_ident,
                    ident: #ident,
                    #name_field
                    item: { #item }
                }
            };
        }
    }
}
