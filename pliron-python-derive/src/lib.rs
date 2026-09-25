//! Python-binding code generation for [pliron](https://crates.io/crates/pliron).
//!
//! This crate generates the `#[pyclass]` wrappers, `#[pymethods]` mirrors,
//! `PyMap` marshalling impls and `PY_CLASS_REGISTRATIONS` entries that expose
//! pliron dialects to Python. All generated code references runtime machinery
//! rooted at `::pliron_python` (the `pliron-python` crate), which re-exports
//! this crate's macros as `pliron_python::derive`.
//!
//! Every generator is available in two forms:
//!
//! 1. **Attribute form** — for items in the crate you are writing. Stack the
//!    attribute *above* the pliron-derive attribute so it can read the
//!    `name = "dialect.entity"` argument:
//!
//!    ```ignore
//!    #[cfg_attr(feature = "python", pliron_python::derive::py_op)]
//!    #[pliron_op(name = "mydialect.my_op")]
//!    pub struct MyOp;
//!
//!    #[cfg_attr(feature = "python", pliron_python::derive::py_op_impl)]
//!    #[pliron_op_impl]
//!    impl MyOp {
//!        pub fn new(ctx: &mut Context /* ... */) -> Self { /* ... */ }
//!    }
//!    ```
//!
//! 2. **Reflect-export form** (`*_from_export`) — for items defined in a
//!    *foreign* crate that must stay python-free (e.g. pliron's own builtin
//!    dialect). pliron-derive's macros export every annotated item's tokens as
//!    a `__pliron_reflect_*` macro; invoke that macro with one of the
//!    `*_from_export` macros as its argument, from a module that has the
//!    target types in scope:
//!
//!    ```ignore
//!    use pliron::builtin::ops::ModuleOp;
//!    pliron::__pliron_reflect_op_ModuleOp!(::pliron_python::derive::py_op_from_export);
//!    pliron::__pliron_reflect_op_impl_ModuleOp!(::pliron_python::derive::py_op_impl_from_export);
//!    ```

mod attr_class;
mod attr_impl;
mod envelope;
mod op_class;
mod op_impl;
mod py_type_mapper;
mod registration;
mod type_class;
mod type_impl;

use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;

use envelope::ReflectEnvelope;

fn to_token_stream(res: syn::Result<proc_macro2::TokenStream>) -> TokenStream {
    match res {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ---------------------------------------------------------------------------
// Reflect-export (callback) forms
// ---------------------------------------------------------------------------

fn class_from_export(
    input: TokenStream,
    expected_kind: &str,
    generate: impl FnOnce(&syn::Ident, &str, &str, &syn::Data) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    let res = (|| {
        let env: ReflectEnvelope = syn::parse(input)?;
        let (dialect, entity) = env.named_parts(expected_kind)?;
        let data = if env.item.is_empty() {
            // Ops export no item tokens; synthesize an empty struct.
            syn::Data::Struct(syn::DataStruct {
                struct_token: Default::default(),
                fields: syn::Fields::Unit,
                semi_token: None,
            })
        } else {
            let item: DeriveInput = syn::parse2(env.item.clone())?;
            item.data
        };
        generate(&env.ident, &dialect, &entity, &data)
    })();
    to_token_stream(res)
}

/// Generate the `#[pyclass]` wrapper for an op from a `__pliron_reflect_op_*` export.
#[proc_macro]
pub fn py_op_from_export(input: TokenStream) -> TokenStream {
    class_from_export(input, "op", |ident, dialect, entity, _data| {
        Ok(op_class::gen_py_op_class(ident, dialect, entity))
    })
}

/// Generate the `#[pyclass]` wrapper for an attribute from a `__pliron_reflect_attr_*` export.
#[proc_macro]
pub fn py_attr_from_export(input: TokenStream) -> TokenStream {
    class_from_export(input, "attr", |ident, dialect, entity, data| {
        Ok(attr_class::gen_py_attr_class(ident, dialect, entity, data))
    })
}

/// Generate the `#[pyclass]` wrapper for a type from a `__pliron_reflect_ty_*` export.
#[proc_macro]
pub fn py_type_from_export(input: TokenStream) -> TokenStream {
    class_from_export(input, "ty", |ident, dialect, entity, data| {
        Ok(type_class::gen_py_type_class(ident, dialect, entity, data))
    })
}

fn impl_from_export(
    input: TokenStream,
    expected_kind: &str,
    generate: impl FnOnce(proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream>,
) -> TokenStream {
    let res = (|| {
        let env: ReflectEnvelope = syn::parse(input)?;
        env.check_impl_kind(expected_kind)?;
        generate(env.item)
    })();
    to_token_stream(res)
}

/// Generate `#[pymethods]` mirroring an op `impl` block from a
/// `__pliron_reflect_op_impl_*` export.
#[proc_macro]
pub fn py_op_impl_from_export(input: TokenStream) -> TokenStream {
    impl_from_export(input, "op_impl", |item| op_impl::gen_op_impl(item, false))
}

/// Generate `#[pymethods]` mirroring an attribute `impl` block from a
/// `__pliron_reflect_attr_impl_*` export.
#[proc_macro]
pub fn py_attr_impl_from_export(input: TokenStream) -> TokenStream {
    impl_from_export(input, "attr_impl", |item| {
        attr_impl::gen_attr_impl(item, false)
    })
}

/// Generate `#[pymethods]` mirroring a type `impl` block from a
/// `__pliron_reflect_ty_impl_*` export.
#[proc_macro]
pub fn py_type_impl_from_export(input: TokenStream) -> TokenStream {
    impl_from_export(input, "ty_impl", |item| {
        type_impl::gen_type_impl(item, false)
    })
}

// ---------------------------------------------------------------------------
// Attribute forms (same-crate use)
// ---------------------------------------------------------------------------

/// Scan an attribute-argument token stream for `name = "..."` at top level.
fn scan_name_value(tokens: proc_macro2::TokenStream) -> Option<String> {
    let mut iter = tokens.into_iter().peekable();
    while let Some(tt) = iter.next() {
        if let proc_macro2::TokenTree::Ident(id) = &tt
            && id == "name"
            && let Some(proc_macro2::TokenTree::Punct(p)) = iter.peek()
            && p.as_char() == '='
        {
            iter.next();
            if let Some(proc_macro2::TokenTree::Literal(lit)) = iter.next()
                && let Ok(syn::Lit::Str(s)) = syn::parse_str::<syn::Lit>(&lit.to_string())
            {
                return Some(s.value());
            }
        }
    }
    None
}

/// Find the `"dialect.entity"` name for an item: from this macro's own
/// arguments (`name = "..."`), or from a sibling `#[<unified>(name = "...")]` /
/// `#[<def>("...")]` attribute on the item.
fn find_entity_name(
    args: proc_macro2::TokenStream,
    input: &DeriveInput,
    unified: &str,
    def: &str,
) -> syn::Result<String> {
    if let Some(name) = scan_name_value(args) {
        return Ok(name);
    }
    for attr in &input.attrs {
        if attr.path().is_ident(def) {
            let lit: syn::LitStr = attr.parse_args()?;
            return Ok(lit.value());
        }
        if attr.path().is_ident(unified)
            && let syn::Meta::List(l) = &attr.meta
            && let Some(name) = scan_name_value(l.tokens.clone())
        {
            return Ok(name);
        }
    }
    Err(syn::Error::new_spanned(
        &input.ident,
        format!(
            "could not determine the entity name: place this attribute above \
             `#[{unified}(name = \"dialect.name\", ...)]` / `#[{def}(\"dialect.name\")]`, \
             or pass `name = \"dialect.name\"` explicitly"
        ),
    ))
}

fn class_attribute(
    args: TokenStream,
    item: TokenStream,
    unified: &str,
    def: &str,
    generate: impl FnOnce(&syn::Ident, &str, &str, &syn::Data) -> proc_macro2::TokenStream,
) -> TokenStream {
    let original: proc_macro2::TokenStream = item.clone().into();
    let res = (|| {
        let input: DeriveInput = syn::parse(item)?;
        let name = find_entity_name(args.into(), &input, unified, def)?;
        let Some((dialect, entity)) = name
            .split_once('.')
            .map(|(d, e)| (d.to_string(), e.to_string()))
        else {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "entity name must be in the form `dialect.name`",
            ));
        };
        let generated = generate(&input.ident, &dialect, &entity, &input.data);
        Ok(quote! {
            #original
            #generated
        })
    })();
    to_token_stream(res)
}

/// Attribute form of [`py_op_from_export!`]: generate the Python wrapper class
/// for an op defined in the current crate. Stack it *above*
/// `#[pliron_op(...)]` / `#[def_op(...)]` (or pass `name = "dialect.op"`).
#[proc_macro_attribute]
pub fn py_op(args: TokenStream, item: TokenStream) -> TokenStream {
    class_attribute(
        args,
        item,
        "pliron_op",
        "def_op",
        |ident, dialect, entity, _| op_class::gen_py_op_class(ident, dialect, entity),
    )
}

/// Attribute form of [`py_attr_from_export!`]: generate the Python wrapper class
/// for an attribute defined in the current crate. Stack it *above*
/// `#[pliron_attr(...)]` / `#[def_attribute(...)]` (or pass `name = "dialect.attr"`).
#[proc_macro_attribute]
pub fn py_attr(args: TokenStream, item: TokenStream) -> TokenStream {
    class_attribute(
        args,
        item,
        "pliron_attr",
        "def_attribute",
        attr_class::gen_py_attr_class,
    )
}

/// Attribute form of [`py_type_from_export!`]: generate the Python wrapper class
/// for a type defined in the current crate. Stack it *above*
/// `#[pliron_type(...)]` / `#[def_type(...)]` (or pass `name = "dialect.type"`).
#[proc_macro_attribute]
pub fn py_type(args: TokenStream, item: TokenStream) -> TokenStream {
    class_attribute(
        args,
        item,
        "pliron_type",
        "def_type",
        type_class::gen_py_type_class,
    )
}

/// Attribute form of [`py_op_impl_from_export!`]: mirror all `pub` methods of an
/// op `impl` block into `#[pymethods]`. Emits the original `impl` unchanged.
#[proc_macro_attribute]
pub fn py_op_impl(_args: TokenStream, item: TokenStream) -> TokenStream {
    to_token_stream(op_impl::gen_op_impl(
        proc_macro2::TokenStream::from(item),
        true,
    ))
}

/// Attribute form of [`py_attr_impl_from_export!`]: mirror all `pub` methods of an
/// attribute `impl` block into `#[pymethods]`. Emits the original `impl` unchanged.
#[proc_macro_attribute]
pub fn py_attr_impl(_args: TokenStream, item: TokenStream) -> TokenStream {
    to_token_stream(attr_impl::gen_attr_impl(
        proc_macro2::TokenStream::from(item),
        true,
    ))
}

/// Attribute form of [`py_type_impl_from_export!`]: mirror all `pub` methods of a
/// type `impl` block into `#[pymethods]`. Emits the original `impl` unchanged.
#[proc_macro_attribute]
pub fn py_type_impl(_args: TokenStream, item: TokenStream) -> TokenStream {
    to_token_stream(type_impl::gen_type_impl(
        proc_macro2::TokenStream::from(item),
        true,
    ))
}
