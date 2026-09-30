//! `#[pyclass]` wrapper generation for IR attributes.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::py_type_mapper::{ParamKind, classify, pymap_path};
use crate::registration::gen_registration;

/// Generate a `#[pyclass]` wrapper for an attribute, auto-registered via
/// `PY_CLASS_REGISTRATIONS`.
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
pub(crate) fn gen_py_attr_class(
    struct_name: &syn::Ident,
    dialect_name: &str,
    attr_name: &str,
    data: &syn::Data,
) -> TokenStream {
    let py_class_name = struct_name.to_string();
    let full_name = format!("{}.{}", dialect_name, attr_name);
    let py_struct = format_ident!("Py{}", struct_name);
    let field_accessors = gen_field_accessors(data);
    let registration = gen_registration(&py_struct, &full_name);

    quote! {
        // Holds the concrete attribute struct *by value* (attributes are owned
        // values, not arena-interned handles), so every method calls straight
        // into the Rust type — no downcast.
        #[::pliron_python::pyo3::pyclass(unsendable, name = #py_class_name, crate = "::pliron_python::pyo3")]
        pub struct #py_struct {
            pub(crate) inner: #struct_name,
        }

        #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
        impl #py_struct {
            /// Downcast a generic `Attribute` to this concrete type.
            /// Returns `None` if the attribute isn't this type.
            #[staticmethod]
            fn from_attr(
                attr: &::pliron_python::attributes::PyAttribute,
            ) -> ::core::option::Option<Self> {
                attr.inner
                    .downcast_ref::<#struct_name>()
                    .map(|a| Self { inner: ::core::clone::Clone::clone(a) })
            }

            /// Project to the generic `Attribute`.
            fn into_attr(&self) -> ::pliron_python::attributes::PyAttribute {
                ::pliron_python::attributes::PyAttribute {
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

            fn __str__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
                let ctx = ::pliron_python::get_ctx()?;
                Ok(::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.inner, ctx)
                ))
            }

            fn __repr__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
                self.__str__()
            }

            fn verify(&self) -> ::pliron_python::pyo3::PyResult<()> {
                let ctx = ::pliron_python::get_ctx()?;
                ::pliron::attribute::verify_attr(&self.inner, ctx)
                    .map_err(::pliron_python::to_py_err)
            }

            fn __eq__(&self, other: &Self) -> bool {
                <#struct_name as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
            }

            fn __ne__(&self, other: &Self) -> bool {
                !<#struct_name as ::pliron::attribute::Attribute>::eq_attr(&self.inner, &other.inner)
            }

            fn __hash__(&self) -> ::pliron_python::pyo3::PyResult<isize> {
                use ::core::hash::{Hash, Hasher};
                let ctx = ::pliron_python::get_ctx()?;
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

        impl ::pliron_python::PyMap for #struct_name {
            type Owned = #py_struct;
            type Borrowed<'py> = ::pliron_python::pyo3::PyRef<'py, #py_struct>;

            fn into_py(self) -> #py_struct {
                #py_struct { inner: self }
            }

            fn from_py(py: ::pliron_python::pyo3::PyRef<'_, #py_struct>) -> Self {
                ::core::clone::Clone::clone(&py.inner)
            }
        }

        #registration
    }
}

/// Generate Python getter methods for each struct field.
///
/// Named fields get `get_<field_name>(&self)`, tuple fields get `get_0`, `get_1`, etc.
/// Trivial-typed fields return the type directly; other fields go through
/// `PyMap`. Fields whose type lacks a `PyMap` impl will surface as a compile-time
/// error at the trait call site — preferred over silently skipping.
fn gen_field_accessors(data: &syn::Data) -> TokenStream {
    let pymap = pymap_path();
    let make_accessor =
        |method_name: syn::Ident, field_access: TokenStream, field_ty: &syn::Type| -> TokenStream {
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
                    Some(make_accessor(method_name, quote!(#field_ident), &field.ty))
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
