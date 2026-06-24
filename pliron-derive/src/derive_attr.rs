use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{DeriveInput, LitStr, Result};

use crate::py_type_mapper::{ParamKind, classify, pymap_path};

const PROC_MACRO_NAME: &str = "def_attribute";

pub(crate) fn def_attribute(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let name = syn::parse2::<LitStr>(args.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DefAttribute::derive(name, input)?;
    Ok(p.into_token_stream())
}

/// The derived macro body for the `#[def_attribute]` proc macro.
struct DefAttribute {
    input: DeriveInput,
    impl_attr: ImplAttribute,
}

impl DefAttribute {
    fn derive(name: LitStr, input: DeriveInput) -> Result<Self> {
        let name_str = name.value();
        let Some((dialect_name, attr_name)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "attribute name must be in the form of `dialect.attr_name`",
            ));
        };

        match input.data {
            syn::Data::Struct(_) | syn::Data::Enum(_) => {}
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "Attribute can only be derived for structs or enums",
                ));
            }
        }

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "Attribute cannot be derived for generic structs or enums",
            ));
        }

        let attrs = input
            .attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident(PROC_MACRO_NAME))
            .collect();

        let input = DeriveInput { attrs, ..input };

        let impl_attr = ImplAttribute {
            ident: input.ident.clone(),
            dialect_name: dialect_name.to_string(),
            attr_name: attr_name.to_string(),
            data: input.data.clone(),
        };

        Ok(Self { input, impl_attr })
    }
}

impl ToTokens for DefAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let def_struct = &self.input;
        let impl_attribute_trait = &self.impl_attr;

        tokens.extend(quote! {
            #def_struct

            #impl_attribute_trait
        });
    }
}

struct ImplAttribute {
    ident: syn::Ident,
    attr_name: String,
    dialect_name: String,
    data: syn::Data,
}

impl ToTokens for ImplAttribute {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.ident;
        let attr_name = &self.attr_name;
        let dialect = &self.dialect_name;
        tokens.extend(quote! {
            impl ::pliron::attribute::Attribute for #name {
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other
                        .downcast_ref::<Self>()
                        .map_or(false, |other| other == self)
                }

                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }

                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::attribute::AttrName::new(#attr_name),
                        dialect: ::pliron::dialect::DialectName::new(#dialect),
                    }
                }

                fn verify_interfaces(&self, ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) =
                        ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP.get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }

            ::pliron::context_registration!(<#name as ::pliron::attribute::Attribute>::register);
        });

        // Generate Python class (behind #[cfg(feature = "python")])
        tokens.extend(gen_py_attr_class(name, dialect, attr_name, &self.data));
    }
}

/// Generate a `#[pyclass]` wrapper for this attribute, gated behind
/// `#[cfg(feature = "python")]` and auto-registered via `PY_CLASS_REGISTRATIONS`.
///
/// The Python class name matches the Rust struct name (e.g. `StringAttr`).
/// The generated Rust struct is `Py<StructName>` (e.g. `PyStringAttr`) and is placed
/// at module scope so that users can add extra `#[pymethods]` blocks (e.g. constructors).
///
/// The wrapper holds the concrete `StructName` by value (attributes are owned,
/// not arena-interned), so methods call straight into the Rust type.
///
/// Provides:
/// - `from_attr(attr)` — downcast a generic `Attribute`, returning `None` on mismatch
/// - `into_attr()` — box up into a generic `Attribute`
/// - the curated `Attribute`-trait surface: `attr_name`, `verify`, `__str__`/`__repr__`,
///   `__eq__`/`__ne__` (via `eq_attr`), `__hash__`, `clone_attr`
/// - One `get_<field>` getter per struct field, for all mappable types.
fn gen_py_attr_class(
    struct_name: &syn::Ident,
    dialect_name: &str,
    attr_name: &str,
    data: &syn::Data,
) -> TokenStream {
    let py_class_name = struct_name.to_string();
    let full_name = format!("{}.{}", dialect_name, attr_name);
    let py_struct = format_ident!("Py{}", struct_name);
    let field_accessors = gen_field_accessors(data);

    quote! {
        // Holds the concrete attribute struct *by value* (attributes are owned
        // values, not arena-interned handles), so every method calls straight
        // into the Rust type — no downcast.
        #[cfg(feature = "python")]
        #[::pliron::pyo3::pyclass(unsendable, name = #py_class_name, crate = "::pliron::pyo3")]
        pub struct #py_struct {
            pub(crate) inner: #struct_name,
        }

        #[cfg(feature = "python")]
        #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
        impl #py_struct {
            /// Downcast a generic `Attribute` to this concrete type.
            /// Returns `None` if the attribute isn't this type.
            #[staticmethod]
            fn from_attr(
                attr: &::pliron::python::attributes::PyAttribute,
            ) -> ::core::option::Option<Self> {
                attr.inner
                    .downcast_ref::<#struct_name>()
                    .map(|a| Self { inner: ::core::clone::Clone::clone(a) })
            }

            /// Project to the generic `Attribute`.
            fn into_attr(&self) -> ::pliron::python::attributes::PyAttribute {
                ::pliron::python::attributes::PyAttribute {
                    inner: ::pliron::alloc::boxed::Box::new(::core::clone::Clone::clone(&self.inner)),
                }
            }

            // --- `Attribute` trait surface (curated) ---

            fn attr_name(&self) -> ::pliron::alloc::string::String {
                ::pliron::alloc::format!(
                    "{}",
                    <#struct_name as ::pliron::attribute::Attribute>::get_attr_id(&self.inner)
                )
            }

            fn clone_attr(&self) -> Self {
                Self { inner: ::core::clone::Clone::clone(&self.inner) }
            }

            fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                let ctx = ::pliron::python::get_ctx()?;
                Ok(::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.inner, ctx)
                ))
            }

            fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                self.__str__()
            }

            fn verify(&self) -> ::pliron::pyo3::PyResult<()> {
                let ctx = ::pliron::python::get_ctx()?;
                ::pliron::attribute::verify_attr(&self.inner, ctx)
                    .map_err(::pliron::python::to_py_err)
            }

            fn __eq__(&self, other: &Self) -> bool {
                <#struct_name as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
            }

            fn __ne__(&self, other: &Self) -> bool {
                !<#struct_name as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
            }

            fn __hash__(&self) -> ::pliron::pyo3::PyResult<isize> {
                use ::core::hash::{Hash, Hasher};
                let ctx = ::pliron::python::get_ctx()?;
                let mut h = ::std::collections::hash_map::DefaultHasher::new();
                // Structurally-equal attributes print identically, so hashing the
                // canonical text keeps `a == b => hash(a) == hash(b)`.
                ::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.inner, ctx)
                ).hash(&mut h);
                Ok(h.finish() as isize)
            }

            #field_accessors
        }

        #[cfg(feature = "python")]
        impl ::pliron::python::PyMap for #struct_name {
            type Owned = #py_struct;
            type Borrowed<'py> = ::pliron::pyo3::PyRef<'py, #py_struct>;

            fn into_py(self) -> #py_struct {
                #py_struct { inner: self }
            }

            fn from_py(py: ::pliron::pyo3::PyRef<'_, #py_struct>) -> Self {
                ::core::clone::Clone::clone(&py.inner)
            }
        }

        #[cfg(feature = "python")]
        const _: () = {
            use ::pliron::pyo3::prelude::*;

            fn __register_py_attr(
                m: &::pliron::pyo3::Bound<'_, ::pliron::pyo3::types::PyModule>,
            ) -> ::pliron::pyo3::PyResult<()> {
                m.add_class::<#py_struct>()?;
                Ok(())
            }

            #[cfg_attr(
                not(target_family = "wasm"),
                ::pliron::linkme::distributed_slice(
                    ::pliron::python::statics::PY_CLASS_REGISTRATIONS
                ),
                linkme(crate = ::pliron::linkme),
            )]
            static __PY_REG: ::pliron::python::PyClassRegistration =
                ::pliron::python::PyClassRegistration {
                    name: #full_name,
                    register: __register_py_attr,
                };
        };
    }
}

/// Generate Python getter methods for each struct field.
///
/// Named fields get `get_<field_name>(&self)`, tuple fields get `get_0`, `get_1`, etc.
/// Trivial-typed fields return the type directly; other fields go through
/// [`PyMap`]. Fields whose type lacks a `PyMap` impl will surface as a compile-time
/// error at the trait call site — preferred over silently skipping.
fn gen_field_accessors(data: &syn::Data) -> TokenStream {
    let pymap = pymap_path();
    let make_accessor = |method_name: syn::Ident,
                         field_access: TokenStream,
                         field_ty: &syn::Type|
     -> TokenStream {
        match classify(field_ty) {
            Some(ParamKind::Trivial) => quote! {
                fn #method_name(&self) -> #field_ty {
                    self.inner.#field_access.clone()
                }
            },
            _ => quote! {
                fn #method_name(&self) -> <#field_ty as #pymap>::Owned {
                    <#field_ty as #pymap>::into_py(self.inner.#field_access.clone())
                }
            },
        }
    };

    let accessors: Vec<TokenStream> = match data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Named(named) => named
                .named
                .iter()
                .filter_map(|field| {
                    let field_ident = field.ident.as_ref()?;
                    let method_name = format_ident!("get_{}", field_ident);
                    Some(make_accessor(
                        method_name,
                        quote!(#field_ident),
                        &field.ty,
                    ))
                })
                .collect(),
            syn::Fields::Unnamed(unnamed) => unnamed
                .unnamed
                .iter()
                .enumerate()
                .map(|(i, field)| {
                    let method_name = format_ident!("get_{}", i);
                    let idx = syn::Index::from(i);
                    make_accessor(method_name, quote!(#idx), &field.ty)
                })
                .collect(),
            syn::Fields::Unit => vec![],
        },
        _ => vec![],
    };
    quote! { #(#accessors)* }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn simple() {
        let args = quote! { "testing.unit" };
        let input = quote! {
            #[derive(PartialEq, Eq, Debug, Clone)]
            pub struct UnitAttr;
        };
        let attr = def_attribute(args, input).unwrap();
        let f = syn::parse2::<syn::File>(attr).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(PartialEq, Eq, Debug, Clone)]
            pub struct UnitAttr;
            impl ::pliron::attribute::Attribute for UnitAttr {
                fn eq_attr(&self, other: &dyn ::pliron::attribute::Attribute) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_attr_id(&self) -> ::pliron::attribute::AttrId {
                    Self::get_attr_id_static()
                }
                fn get_attr_id_static() -> ::pliron::attribute::AttrId {
                    ::pliron::attribute::AttrId {
                        name: ::pliron::attribute::AttrName::new("unit"),
                        dialect: ::pliron::dialect::DialectName::new("testing"),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::attribute::ATTR_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(
                < UnitAttr as ::pliron::attribute::Attribute > ::register
            );
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pyclass(unsendable, name = "UnitAttr", crate = "::pliron::pyo3")]
            pub struct PyUnitAttr {
                pub(crate) inner: UnitAttr,
            }
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl PyUnitAttr {
                /// Downcast a generic `Attribute` to this concrete type.
                /// Returns `None` if the attribute isn't this type.
                #[staticmethod]
                fn from_attr(
                    attr: &::pliron::python::attributes::PyAttribute,
                ) -> ::core::option::Option<Self> {
                    attr.inner
                        .downcast_ref::<UnitAttr>()
                        .map(|a| Self {
                            inner: ::core::clone::Clone::clone(a),
                        })
                }
                /// Project to the generic `Attribute`.
                fn into_attr(&self) -> ::pliron::python::attributes::PyAttribute {
                    ::pliron::python::attributes::PyAttribute {
                        inner: ::pliron::alloc::boxed::Box::new(
                            ::core::clone::Clone::clone(&self.inner),
                        ),
                    }
                }
                fn attr_name(&self) -> ::pliron::alloc::string::String {
                    ::pliron::alloc::format!(
                        "{}", < UnitAttr as ::pliron::attribute::Attribute > ::get_attr_id(& self
                        .inner)
                    )
                }
                fn clone_attr(&self) -> Self {
                    Self {
                        inner: ::core::clone::Clone::clone(&self.inner),
                    }
                }
                fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    let ctx = ::pliron::python::get_ctx()?;
                    Ok(
                        ::pliron::alloc::format!(
                            "{}", ::pliron::printable::Printable::disp(& self.inner, ctx)
                        ),
                    )
                }
                fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    self.__str__()
                }
                fn verify(&self) -> ::pliron::pyo3::PyResult<()> {
                    let ctx = ::pliron::python::get_ctx()?;
                    ::pliron::attribute::verify_attr(&self.inner, ctx)
                        .map_err(::pliron::python::to_py_err)
                }
                fn __eq__(&self, other: &Self) -> bool {
                    <UnitAttr as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
                }
                fn __ne__(&self, other: &Self) -> bool {
                    !<UnitAttr as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
                }
                fn __hash__(&self) -> ::pliron::pyo3::PyResult<isize> {
                    use ::core::hash::{Hash, Hasher};
                    let ctx = ::pliron::python::get_ctx()?;
                    let mut h = ::std::collections::hash_map::DefaultHasher::new();
                    ::pliron::alloc::format!(
                        "{}", ::pliron::printable::Printable::disp(& self.inner, ctx)
                    )
                        .hash(&mut h);
                    Ok(h.finish() as isize)
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyMap for UnitAttr {
                type Owned = PyUnitAttr;
                type Borrowed<'py> = ::pliron::pyo3::PyRef<'py, PyUnitAttr>;
                fn into_py(self) -> PyUnitAttr {
                    PyUnitAttr { inner: self }
                }
                fn from_py(py: ::pliron::pyo3::PyRef<'_, PyUnitAttr>) -> Self {
                    ::core::clone::Clone::clone(&py.inner)
                }
            }
            #[cfg(feature = "python")]
            const _: () = {
                use ::pliron::pyo3::prelude::*;
                fn __register_py_attr(
                    m: &::pliron::pyo3::Bound<'_, ::pliron::pyo3::types::PyModule>,
                ) -> ::pliron::pyo3::PyResult<()> {
                    m.add_class::<PyUnitAttr>()?;
                    Ok(())
                }
                #[cfg_attr(
                    not(target_family = "wasm"),
                    ::pliron::linkme::distributed_slice(
                        ::pliron::python::statics::PY_CLASS_REGISTRATIONS
                    ),
                    linkme(crate = ::pliron::linkme),
                )]
                static __PY_REG: ::pliron::python::PyClassRegistration = ::pliron::python::PyClassRegistration {
                    name: "testing.unit",
                    register: __register_py_attr,
                };
            };
        "##]]
        .assert_eq(&got);
    }
}
