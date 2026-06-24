use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::{DeriveInput, LitStr, Result};

use crate::py_type_mapper::{ParamKind, classify, pymap_path};

const PROC_MACRO_NAME: &str = "def_type";

pub(crate) fn def_type(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let name = syn::parse2::<LitStr>(args.into())?;
    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DefType::derive(name, input)?;
    Ok(p.into_token_stream())
}

/// Input for the `#[def_type]` proc macro.
struct DefType {
    input: DeriveInput,
    impl_type: ImplType,
}

impl DefType {
    fn derive(name: LitStr, input: DeriveInput) -> Result<Self> {
        let name_str = name.value();
        let Some((dialect_name, type_name)) = name_str.split_once('.') else {
            return Err(syn::Error::new_spanned(
                name,
                "type name must be in the form `dialect.type_name`",
            ));
        };

        let is_singleton = match input.data {
            syn::Data::Struct(ref data_struct) => data_struct.fields.is_empty(),
            syn::Data::Enum(_) => false,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "Type can only be derived for structs or enums",
                ));
            }
        };

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "Type cannot be derived for generic structs or enums",
            ));
        }

        let attrs = input
            .attrs
            .into_iter()
            .filter(|attr| !attr.path().is_ident(PROC_MACRO_NAME))
            .collect();

        let input = DeriveInput { attrs, ..input };

        let impl_type = ImplType {
            ident: input.ident.clone(),
            dialect_name: dialect_name.to_string(),
            type_name: type_name.to_string(),
            is_singleton,
            data: input.data.clone(),
        };
        Ok(Self { input, impl_type })
    }
}

impl ToTokens for DefType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let def_struct = &self.input;
        let impl_type = &self.impl_type;

        tokens.extend(quote! {
            #def_struct

            #impl_type
        });
    }
}

struct ImplType {
    ident: syn::Ident,
    dialect_name: String,
    type_name: String,
    is_singleton: bool,
    data: syn::Data,
}

impl ToTokens for ImplType {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let name = &self.ident;
        let dialect = &self.dialect_name;
        let type_name = &self.type_name;
        let register = if self.is_singleton {
            quote! {
                |ctx: &mut ::pliron::context::Context| {
                    <#name as ::pliron::r#type::Type>::register(ctx);
                    ::pliron::r#type::Type::register_instance(#name {}, ctx);
                }
            }
        } else {
            quote! {
                <#name as ::pliron::r#type::Type>::register
            }
        };

        tokens.extend(quote! {
            impl ::pliron::r#type::Type for #name {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }

                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other
                        .downcast_ref::<Self>()
                        .map_or(false, |other| other == self)
                }

                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }

                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::r#type::TypeName::new(#type_name),
                        dialect: ::pliron::dialect::DialectName::new(#dialect),
                    }
                }

                fn verify_interfaces(&self, ctx: &::pliron::context::Context) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) =
                        ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP.get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }

            ::pliron::context_registration!(#register);
        });

        // Generate Python class (behind #[cfg(feature = "python")])
        tokens.extend(gen_py_type_class(name, dialect, type_name, &self.data));
    }
}

/// Generate a `#[pyclass]` wrapper for this type, gated behind
/// `#[cfg(feature = "python")]` and auto-registered via `PY_CLASS_REGISTRATIONS`.
///
/// The Python class name matches the Rust struct name (e.g. `IntegerType`).
/// The generated Rust struct is `Py<StructName>` (e.g. `PyIntegerType`) and is placed
/// at module scope so that users can add extra `#[pymethods]` blocks (e.g. constructors).
///
/// The wrapper holds a `TypedHandle<StructName>` and behaves like one.
///
/// Provides:
/// - `from_type(ty)` — downcast a generic `Type`, returning `None` on mismatch
/// - `to_type()` — project to a generic `Type`
/// - `__str__`, `__repr__`, `__eq__`, `__hash__`
/// - One `get_<field>` getter per struct field, for all mappable types.
fn gen_py_type_class(
    struct_name: &syn::Ident,
    dialect_name: &str,
    type_name: &str,
    data: &syn::Data,
) -> TokenStream {
    let py_class_name = struct_name.to_string();
    let full_name = format!("{}.{}", dialect_name, type_name);
    let py_struct = format_ident!("Py{}", struct_name);
    let field_accessors = gen_type_field_accessors(data);

    quote! {
        // The wrapper holds a statically-typed `TypedHandle<#struct_name>` and
        // behaves like one: every method derefs through the active context to the
        // concrete type.
        #[cfg(feature = "python")]
        #[::pliron::pyo3::pyclass(unsendable, name = #py_class_name, crate = "::pliron::pyo3")]
        pub struct #py_struct {
            pub(crate) ptr: ::pliron::r#type::TypedHandle<#struct_name>,
        }

        #[cfg(feature = "python")]
        #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
        impl #py_struct {
            /// Downcast a generic `Type` to this concrete type.
            /// Returns `None` if the handle does not refer to this type.
            #[staticmethod]
            fn from_type(
                ty: &::pliron::python::types::PyType,
            ) -> ::pliron::pyo3::PyResult<::core::option::Option<Self>> {
                let ctx = ::pliron::python::get_ctx()?;
                if ty.ptr.deref(ctx).downcast_ref::<#struct_name>().is_some() {
                    Ok(Some(Self {
                        ptr: ::pliron::r#type::TypedHandle::from_handle_unchecked(ty.ptr),
                    }))
                } else {
                    Ok(None)
                }
            }

            /// Project to the generic `Type` handle.
            fn to_type(&self) -> ::pliron::python::types::PyType {
                ::pliron::python::types::PyType { ptr: self.ptr.to_handle() }
            }

            fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                let ctx = ::pliron::python::get_ctx()?;
                Ok(::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.ptr.to_handle(), ctx)
                ))
            }

            fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                self.__str__()
            }

            fn __eq__(&self, other: &Self) -> bool {
                self.ptr.to_handle() == other.ptr.to_handle()
            }

            fn __hash__(&self) -> usize {
                use ::std::hash::{Hash, Hasher};
                let mut h = ::std::collections::hash_map::DefaultHasher::new();
                self.ptr.to_handle().hash(&mut h);
                h.finish() as usize
            }

            #field_accessors
        }

        // Teach pliron about the Python wrapper for this type. The blanket impl
        // `impl<T: PyMapTarget> PyMap for TypedHandle<T>` in pliron then routes
        // `TypedHandle<MyType>` returns to `PyMyType` automatically. This indirection
        // is needed because a direct `impl PyMap for TypedHandle<MyType>` here would
        // violate the orphan rule when `MyType` is declared outside `pliron`.
        #[cfg(feature = "python")]
        impl ::pliron::python::PyTypeWrapper for #py_struct {
            type Concrete = #struct_name;
            fn from_typed_handle(
                handle: ::pliron::r#type::TypedHandle<#struct_name>,
            ) -> Self {
                #py_struct { ptr: handle }
            }
            fn to_typed_handle(&self) -> ::pliron::r#type::TypedHandle<#struct_name> {
                ::pliron::r#type::TypedHandle::from_handle_unchecked(self.ptr.to_handle())
            }
        }

        #[cfg(feature = "python")]
        impl ::pliron::python::PyMapTarget for #struct_name {
            type PyClass = #py_struct;
        }

        #[cfg(feature = "python")]
        const _: () = {
            use ::pliron::pyo3::prelude::*;

            fn __register_py_type(
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
                    register: __register_py_type,
                };
        };
    }
}

/// Generate Python getter methods for each struct field of a *type*.
///
/// Named fields get `get_<field_name>(&self)`, tuple fields get `get_0`, `get_1`, etc.
/// **Always returns `PyResult<T>`** because accessing fields requires dereferencing
/// the `Ptr<TypeObj>` via `&Context`, which is obtained from the thread-local context.
///
/// Trivial-typed fields return the type directly; other fields go through [`PyMap`].
/// Fields whose type lacks a `PyMap` impl surface as a compile-time error at the
/// trait call site (rather than being silently skipped).
fn gen_type_field_accessors(data: &syn::Data) -> TokenStream {
    let pymap = pymap_path();
    let make_accessor = |method_name: syn::Ident,
                         field_access: TokenStream,
                         field_ty: &syn::Type|
     -> TokenStream {
        match classify(field_ty) {
            Some(ParamKind::Trivial) => quote! {
                fn #method_name(&self) -> ::pliron::pyo3::PyResult<#field_ty> {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    Ok(__inner.#field_access.clone())
                }
            },
            _ => quote! {
                fn #method_name(&self) -> ::pliron::pyo3::PyResult<<#field_ty as #pymap>::Owned> {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    Ok(<#field_ty as #pymap>::into_py(__inner.#field_access.clone()))
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

pub(crate) fn derive_type_get(
    args: impl Into<TokenStream>,
    input: impl Into<TokenStream>,
) -> syn::Result<TokenStream> {
    let args_stream = args.into();
    if !args_stream.is_empty() {
        return Err(syn::Error::new_spanned(
            args_stream,
            "`#[derive_type_get]` does not take any arguments",
        ));
    }

    let input = syn::parse2::<DeriveInput>(input.into())?;
    let p = DeriveTypeGet::derive(input)?;
    Ok(p.into_token_stream())
}

/// Input for the `#[derive(DeriveTypeGet)]` proc macro.
struct DeriveTypeGet {
    input: DeriveInput,
    ident: syn::Ident,
    fields: syn::Fields,
}

impl DeriveTypeGet {
    fn derive(input: DeriveInput) -> Result<Self> {
        let fields = match input.data {
            syn::Data::Struct(ref data_struct) => data_struct.fields.clone(),
            _ => {
                return Err(syn::Error::new_spanned(
                    &input,
                    "DeriveTypeGet can only be derived for structs",
                ));
            }
        };

        if !input.generics.params.is_empty() {
            return Err(syn::Error::new_spanned(
                &input,
                "DeriveTypeGet cannot be derived for generic structs",
            ));
        }

        Ok(Self {
            ident: input.ident.clone(),
            input,
            fields,
        })
    }
}

impl ToTokens for DeriveTypeGet {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let input = &self.input;
        let impl_get = derive_type_get_impl(&self.ident, &self.fields);
        tokens.extend(quote! {
            #input

            #impl_get
        });
    }
}

/// Generate get methods for types based on their fields
fn derive_type_get_impl(ident: &syn::Ident, fields: &syn::Fields) -> TokenStream {
    let field_params = generate_field_params(fields);
    let struct_construction = generate_struct_construction(ident, fields);

    quote! {
        impl #ident {
            /// Get or create a new instance.
            pub fn get(
                ctx: &::pliron::context::Context,
                #field_params
            ) -> ::pliron::r#type::TypedHandle<Self> {
                ::pliron::r#type::Type::register_instance(
                    #struct_construction,
                    ctx,
                )
            }
        }
    }
}

/// Generate field parameters for function signatures
fn generate_field_params(fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(fields) => {
            let params = fields.named.iter().map(|field| {
                let name = &field.ident;
                let ty = &field.ty;
                quote! { #name: #ty }
            });
            quote! { #(#params),* }
        }
        syn::Fields::Unnamed(fields) => {
            let params = fields.unnamed.iter().enumerate().map(|(i, field)| {
                let name = syn::Ident::new(&format!("field_{}", i), proc_macro2::Span::call_site());
                let ty = &field.ty;
                quote! { #name: #ty }
            });
            quote! { #(#params),* }
        }
        syn::Fields::Unit => quote! {},
    }
}

/// Generate field assignments for struct construction
fn generate_field_assignments(fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(fields) => {
            let assignments = fields.named.iter().map(|field| {
                let name = &field.ident;
                quote! { #name }
            });
            quote! { #(#assignments),* }
        }
        syn::Fields::Unnamed(fields) => {
            let assignments = fields.unnamed.iter().enumerate().map(|(i, _)| {
                let name = syn::Ident::new(&format!("field_{}", i), proc_macro2::Span::call_site());
                quote! { #name }
            });
            quote! { #(#assignments),* }
        }
        syn::Fields::Unit => quote! {},
    }
}

/// Generate struct construction syntax based on field type
fn generate_struct_construction(ident: &syn::Ident, fields: &syn::Fields) -> TokenStream {
    match fields {
        syn::Fields::Named(_) => {
            let field_assignments = generate_field_assignments(fields);
            quote! { #ident { #field_assignments } }
        }
        syn::Fields::Unnamed(_) => {
            let field_assignments = generate_field_assignments(fields);
            quote! { #ident(#field_assignments) }
        }
        syn::Fields::Unit => {
            quote! { #ident {} }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn simple() {
        let args = quote! { "testing.simple_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct SimpleType;
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct SimpleType;
            impl ::pliron::r#type::Type for SimpleType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::r#type::TypeName::new("simple_type"),
                        dialect: ::pliron::dialect::DialectName::new("testing"),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
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
                | ctx : & mut ::pliron::context::Context | { < SimpleType as ::pliron::r#type::Type >
                ::register(ctx); ::pliron::r#type::Type::register_instance(SimpleType {}, ctx); }
            );
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pyclass(unsendable, name = "SimpleType", crate = "::pliron::pyo3")]
            pub struct PySimpleType {
                pub(crate) ptr: ::pliron::r#type::TypedHandle<SimpleType>,
            }
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl PySimpleType {
                /// Downcast a generic `Type` to this concrete type.
                /// Returns `None` if the handle does not refer to this type.
                #[staticmethod]
                fn from_type(
                    ty: &::pliron::python::types::PyType,
                ) -> ::pliron::pyo3::PyResult<::core::option::Option<Self>> {
                    let ctx = ::pliron::python::get_ctx()?;
                    if ty.ptr.deref(ctx).downcast_ref::<SimpleType>().is_some() {
                        Ok(
                            Some(Self {
                                ptr: ::pliron::r#type::TypedHandle::from_handle_unchecked(ty.ptr),
                            }),
                        )
                    } else {
                        Ok(None)
                    }
                }
                /// Project to the generic `Type` handle.
                fn to_type(&self) -> ::pliron::python::types::PyType {
                    ::pliron::python::types::PyType {
                        ptr: self.ptr.to_handle(),
                    }
                }
                fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    let ctx = ::pliron::python::get_ctx()?;
                    Ok(
                        ::pliron::alloc::format!(
                            "{}", ::pliron::printable::Printable::disp(& self.ptr.to_handle(), ctx)
                        ),
                    )
                }
                fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    self.__str__()
                }
                fn __eq__(&self, other: &Self) -> bool {
                    self.ptr.to_handle() == other.ptr.to_handle()
                }
                fn __hash__(&self) -> usize {
                    use ::std::hash::{Hash, Hasher};
                    let mut h = ::std::collections::hash_map::DefaultHasher::new();
                    self.ptr.to_handle().hash(&mut h);
                    h.finish() as usize
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyTypeWrapper for PySimpleType {
                type Concrete = SimpleType;
                fn from_typed_handle(handle: ::pliron::r#type::TypedHandle<SimpleType>) -> Self {
                    PySimpleType { ptr: handle }
                }
                fn to_typed_handle(&self) -> ::pliron::r#type::TypedHandle<SimpleType> {
                    ::pliron::r#type::TypedHandle::from_handle_unchecked(self.ptr.to_handle())
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyMapTarget for SimpleType {
                type PyClass = PySimpleType;
            }
            #[cfg(feature = "python")]
            const _: () = {
                use ::pliron::pyo3::prelude::*;
                fn __register_py_type(
                    m: &::pliron::pyo3::Bound<'_, ::pliron::pyo3::types::PyModule>,
                ) -> ::pliron::pyo3::PyResult<()> {
                    m.add_class::<PySimpleType>()?;
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
                    name: "testing.simple_type",
                    register: __register_py_type,
                };
            };
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn compound() {
        let args = quote! { "testing.compound_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct CompoundType {
                x1: u32,
                x2: String,
            }
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub struct CompoundType {
                x1: u32,
                x2: String,
            }
            impl ::pliron::r#type::Type for CompoundType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::r#type::TypeName::new("compound_type"),
                        dialect: ::pliron::dialect::DialectName::new("testing"),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< CompoundType as ::pliron::r#type::Type > ::register);
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pyclass(unsendable, name = "CompoundType", crate = "::pliron::pyo3")]
            pub struct PyCompoundType {
                pub(crate) ptr: ::pliron::r#type::TypedHandle<CompoundType>,
            }
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl PyCompoundType {
                /// Downcast a generic `Type` to this concrete type.
                /// Returns `None` if the handle does not refer to this type.
                #[staticmethod]
                fn from_type(
                    ty: &::pliron::python::types::PyType,
                ) -> ::pliron::pyo3::PyResult<::core::option::Option<Self>> {
                    let ctx = ::pliron::python::get_ctx()?;
                    if ty.ptr.deref(ctx).downcast_ref::<CompoundType>().is_some() {
                        Ok(
                            Some(Self {
                                ptr: ::pliron::r#type::TypedHandle::from_handle_unchecked(ty.ptr),
                            }),
                        )
                    } else {
                        Ok(None)
                    }
                }
                /// Project to the generic `Type` handle.
                fn to_type(&self) -> ::pliron::python::types::PyType {
                    ::pliron::python::types::PyType {
                        ptr: self.ptr.to_handle(),
                    }
                }
                fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    let ctx = ::pliron::python::get_ctx()?;
                    Ok(
                        ::pliron::alloc::format!(
                            "{}", ::pliron::printable::Printable::disp(& self.ptr.to_handle(), ctx)
                        ),
                    )
                }
                fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    self.__str__()
                }
                fn __eq__(&self, other: &Self) -> bool {
                    self.ptr.to_handle() == other.ptr.to_handle()
                }
                fn __hash__(&self) -> usize {
                    use ::std::hash::{Hash, Hasher};
                    let mut h = ::std::collections::hash_map::DefaultHasher::new();
                    self.ptr.to_handle().hash(&mut h);
                    h.finish() as usize
                }
                fn get_x1(&self) -> ::pliron::pyo3::PyResult<u32> {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    Ok(__inner.x1.clone())
                }
                fn get_x2(&self) -> ::pliron::pyo3::PyResult<String> {
                    let ctx = ::pliron::python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    Ok(__inner.x2.clone())
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyTypeWrapper for PyCompoundType {
                type Concrete = CompoundType;
                fn from_typed_handle(handle: ::pliron::r#type::TypedHandle<CompoundType>) -> Self {
                    PyCompoundType { ptr: handle }
                }
                fn to_typed_handle(&self) -> ::pliron::r#type::TypedHandle<CompoundType> {
                    ::pliron::r#type::TypedHandle::from_handle_unchecked(self.ptr.to_handle())
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyMapTarget for CompoundType {
                type PyClass = PyCompoundType;
            }
            #[cfg(feature = "python")]
            const _: () = {
                use ::pliron::pyo3::prelude::*;
                fn __register_py_type(
                    m: &::pliron::pyo3::Bound<'_, ::pliron::pyo3::types::PyModule>,
                ) -> ::pliron::pyo3::PyResult<()> {
                    m.add_class::<PyCompoundType>()?;
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
                    name: "testing.compound_type",
                    register: __register_py_type,
                };
            };
        "##]].assert_eq(&got);
    }

    #[test]
    fn enum_type() {
        let args = quote! { "testing.enum_type" };
        let input = quote! {
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub enum EnumType {
                None,
                One(u32),
            }
        };
        let t = def_type(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r##"
            #[derive(Hash, PartialEq, Eq, Debug)]
            pub enum EnumType {
                None,
                One(u32),
            }
            impl ::pliron::r#type::Type for EnumType {
                fn hash_type(&self) -> ::pliron::storage_uniquer::TypeValueHash {
                    ::pliron::storage_uniquer::TypeValueHash::new(self)
                }
                fn eq_type(&self, other: &dyn ::pliron::r#type::Type) -> bool {
                    other.downcast_ref::<Self>().map_or(false, |other| other == self)
                }
                fn get_type_id(&self) -> ::pliron::r#type::TypeId {
                    Self::get_type_id_static()
                }
                fn get_type_id_static() -> ::pliron::r#type::TypeId {
                    ::pliron::r#type::TypeId {
                        name: ::pliron::r#type::TypeName::new("enum_type"),
                        dialect: ::pliron::dialect::DialectName::new("testing"),
                    }
                }
                fn verify_interfaces(
                    &self,
                    ctx: &::pliron::context::Context,
                ) -> ::pliron::result::Result<()> {
                    if let Some(interface_verifiers) = ::pliron::r#type::TYPE_INTERFACE_VERIFIERS_MAP
                        .get(&::core::any::TypeId::of::<Self>())
                    {
                        for verifier in interface_verifiers {
                            verifier(self, ctx)?;
                        }
                    }
                    Ok(())
                }
            }
            ::pliron::context_registration!(< EnumType as ::pliron::r#type::Type > ::register);
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pyclass(unsendable, name = "EnumType", crate = "::pliron::pyo3")]
            pub struct PyEnumType {
                pub(crate) ptr: ::pliron::r#type::TypedHandle<EnumType>,
            }
            #[cfg(feature = "python")]
            #[::pliron::pyo3::pymethods(crate = "::pliron::pyo3")]
            impl PyEnumType {
                /// Downcast a generic `Type` to this concrete type.
                /// Returns `None` if the handle does not refer to this type.
                #[staticmethod]
                fn from_type(
                    ty: &::pliron::python::types::PyType,
                ) -> ::pliron::pyo3::PyResult<::core::option::Option<Self>> {
                    let ctx = ::pliron::python::get_ctx()?;
                    if ty.ptr.deref(ctx).downcast_ref::<EnumType>().is_some() {
                        Ok(
                            Some(Self {
                                ptr: ::pliron::r#type::TypedHandle::from_handle_unchecked(ty.ptr),
                            }),
                        )
                    } else {
                        Ok(None)
                    }
                }
                /// Project to the generic `Type` handle.
                fn to_type(&self) -> ::pliron::python::types::PyType {
                    ::pliron::python::types::PyType {
                        ptr: self.ptr.to_handle(),
                    }
                }
                fn __str__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    let ctx = ::pliron::python::get_ctx()?;
                    Ok(
                        ::pliron::alloc::format!(
                            "{}", ::pliron::printable::Printable::disp(& self.ptr.to_handle(), ctx)
                        ),
                    )
                }
                fn __repr__(&self) -> ::pliron::pyo3::PyResult<::pliron::alloc::string::String> {
                    self.__str__()
                }
                fn __eq__(&self, other: &Self) -> bool {
                    self.ptr.to_handle() == other.ptr.to_handle()
                }
                fn __hash__(&self) -> usize {
                    use ::std::hash::{Hash, Hasher};
                    let mut h = ::std::collections::hash_map::DefaultHasher::new();
                    self.ptr.to_handle().hash(&mut h);
                    h.finish() as usize
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyTypeWrapper for PyEnumType {
                type Concrete = EnumType;
                fn from_typed_handle(handle: ::pliron::r#type::TypedHandle<EnumType>) -> Self {
                    PyEnumType { ptr: handle }
                }
                fn to_typed_handle(&self) -> ::pliron::r#type::TypedHandle<EnumType> {
                    ::pliron::r#type::TypedHandle::from_handle_unchecked(self.ptr.to_handle())
                }
            }
            #[cfg(feature = "python")]
            impl ::pliron::python::PyMapTarget for EnumType {
                type PyClass = PyEnumType;
            }
            #[cfg(feature = "python")]
            const _: () = {
                use ::pliron::pyo3::prelude::*;
                fn __register_py_type(
                    m: &::pliron::pyo3::Bound<'_, ::pliron::pyo3::types::PyModule>,
                ) -> ::pliron::pyo3::PyResult<()> {
                    m.add_class::<PyEnumType>()?;
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
                    name: "testing.enum_type",
                    register: __register_py_type,
                };
            };
        "##]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_unit_struct() {
        let args = quote! {};
        let input = quote! {
            pub struct UnitType;
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct UnitType;
            impl UnitType {
                /// Get or create a new instance.
                pub fn get(ctx: &::pliron::context::Context) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(UnitType {}, ctx)
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_named_fields() {
        let args = quote! {};
        let input = quote! {
            pub struct VectorType {
                elem_ty: TypeHandle,
                num_elems: u32,
                kind: VectorTypeKind,
            }
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct VectorType {
                elem_ty: TypeHandle,
                num_elems: u32,
                kind: VectorTypeKind,
            }
            impl VectorType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    elem_ty: TypeHandle,
                    num_elems: u32,
                    kind: VectorTypeKind,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(
                        VectorType {
                            elem_ty,
                            num_elems,
                            kind,
                        },
                        ctx,
                    )
                }
            }
        "#]]
        .assert_eq(&got);
    }

    #[test]
    fn derive_type_get_unnamed_fields() {
        let args = quote! {};
        let input = quote! {
            pub struct TupleType(u32, String, bool);
        };
        let t = derive_type_get(args, input).unwrap();
        let f = syn::parse2::<syn::File>(t).unwrap();
        let got = prettyplease::unparse(&f);

        expect![[r#"
            pub struct TupleType(u32, String, bool);
            impl TupleType {
                /// Get or create a new instance.
                pub fn get(
                    ctx: &::pliron::context::Context,
                    field_0: u32,
                    field_1: String,
                    field_2: bool,
                ) -> ::pliron::r#type::TypedHandle<Self> {
                    ::pliron::r#type::Type::register_instance(
                        TupleType(field_0, field_1, field_2),
                        ctx,
                    )
                }
            }
        "#]]
        .assert_eq(&got);
    }
}
