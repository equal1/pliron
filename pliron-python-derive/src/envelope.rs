//! Parser for the `pliron_reflect_v1` token envelope.
//!
//! `pliron-derive`'s `def_op` / `def_attribute` / `def_type` and
//! `pliron_op_impl` / `pliron_attr_impl` / `pliron_type_impl` macros emit, for
//! every annotated item, a `#[doc(hidden)] #[macro_export]` macro of the form
//! `__pliron_reflect_<kind>_<Ident>`. Invoking that macro with a macro path as
//! its argument forwards the item's tokens to that macro, wrapped in this
//! envelope:
//!
//! ```text
//! pliron_reflect_v1,
//! kind: op,                  // op | attr | ty | op_impl | attr_impl | ty_impl
//! ident: ModuleOp,
//! name: "builtin.module",    // absent for *_impl kinds
//! item: { ...item tokens... }
//! ```
//!
//! This is how python wrappers are generated for items that live in a crate
//! that must stay python-free (pliron's own dialects, or any downstream
//! dialect crate that keeps its bindings in a sibling crate).

use proc_macro2::TokenStream;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Token, braced};

pub(crate) struct ReflectEnvelope {
    pub kind: Ident,
    pub ident: Ident,
    /// The `"dialect.entity"` name. Present for `op`/`attr`/`ty`, absent for `*_impl`.
    pub name: Option<LitStr>,
    pub item: TokenStream,
}

fn expect_key(input: ParseStream, key: &str) -> syn::Result<()> {
    let ident: Ident = input.parse().map_err(|_| {
        syn::Error::new(
            input.span(),
            format!("expected `{key}:` in reflect envelope"),
        )
    })?;
    if ident != key {
        return Err(syn::Error::new_spanned(
            ident,
            format!("expected `{key}:` in reflect envelope"),
        ));
    }
    input.parse::<Token![:]>()?;
    Ok(())
}

impl Parse for ReflectEnvelope {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let version: Ident = input.parse().map_err(|_| {
            syn::Error::new(
                input.span(),
                "expected a `pliron_reflect_v1` envelope; this macro is meant to be invoked \
                 through a `__pliron_reflect_*` macro exported by a pliron-derive-annotated crate",
            )
        })?;
        if version != "pliron_reflect_v1" {
            return Err(syn::Error::new_spanned(
                version,
                "unsupported reflect envelope version (expected `pliron_reflect_v1`); \
                 the pliron and pliron-python-derive versions are likely incompatible",
            ));
        }
        input.parse::<Token![,]>()?;

        expect_key(input, "kind")?;
        let kind: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        expect_key(input, "ident")?;
        let ident: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        let name = if input.peek(Ident) && input.fork().parse::<Ident>()? == "name" {
            expect_key(input, "name")?;
            let name: LitStr = input.parse()?;
            input.parse::<Token![,]>()?;
            Some(name)
        } else {
            None
        };

        expect_key(input, "item")?;
        let content;
        braced!(content in input);
        let item: TokenStream = content.parse()?;

        // Optional trailing comma.
        let _ = input.parse::<Token![,]>();

        Ok(Self {
            kind,
            ident,
            name,
            item,
        })
    }
}

impl ReflectEnvelope {
    /// Validate the envelope kind and split the `"dialect.entity"` name.
    /// `expected_kind` is one of `op`/`attr`/`ty`.
    pub(crate) fn named_parts(&self, expected_kind: &str) -> syn::Result<(String, String)> {
        if self.kind != expected_kind {
            return Err(syn::Error::new_spanned(
                &self.kind,
                format!(
                    "this macro handles `{expected_kind}` reflect exports, \
                     but the envelope is of kind `{}`",
                    self.kind
                ),
            ));
        }
        let name = self.name.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(&self.ident, "reflect envelope is missing the `name:` field")
        })?;
        let name_str = name.value();
        let Some((dialect, entity)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "entity name must be in the form `dialect.name`",
            ));
        };
        Ok((dialect.to_string(), entity.to_string()))
    }

    /// Validate the envelope kind for an `*_impl` export.
    pub(crate) fn check_impl_kind(&self, expected_kind: &str) -> syn::Result<()> {
        if self.kind != expected_kind {
            return Err(syn::Error::new_spanned(
                &self.kind,
                format!(
                    "this macro handles `{expected_kind}` reflect exports, \
                     but the envelope is of kind `{}`",
                    self.kind
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn parses_full_envelope() {
        let env: ReflectEnvelope = syn::parse2(quote! {
            pliron_reflect_v1,
            kind: op,
            ident: ModuleOp,
            name: "builtin.module",
            item: { pub struct ModuleOp; }
        })
        .unwrap();
        assert_eq!(env.kind, "op");
        assert_eq!(env.ident, "ModuleOp");
        let (d, e) = env.named_parts("op").unwrap();
        assert_eq!((d.as_str(), e.as_str()), ("builtin", "module"));
    }

    #[test]
    fn parses_impl_envelope_without_name() {
        let env: ReflectEnvelope = syn::parse2(quote! {
            pliron_reflect_v1,
            kind: op_impl,
            ident: ModuleOp,
            item: { impl ModuleOp { pub fn new(name: String) -> Self {} } }
        })
        .unwrap();
        assert!(env.name.is_none());
        env.check_impl_kind("op_impl").unwrap();
        assert!(env.check_impl_kind("ty_impl").is_err());
    }

    #[test]
    fn rejects_wrong_version() {
        let res: syn::Result<ReflectEnvelope> = syn::parse2(quote! {
            pliron_reflect_v2, kind: op, ident: X, item: {}
        });
        assert!(res.is_err());
    }
}
