//! `#[pyclass]` wrapper generation for IR types.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::py_type_mapper::{ParamKind, classify, pymap_path};
use crate::registration::gen_registration;

/// Generate a `#[pyclass]` wrapper for a type, auto-registered via
/// `PY_CLASS_REGISTRATIONS`.
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
pub(crate) fn gen_py_type_class(
    struct_name: &syn::Ident,
    dialect_name: &str,
    type_name: &str,
    data: &syn::Data,
) -> TokenStream {
    let py_class_name = struct_name.to_string();
    let full_name = format!("{}.{}", dialect_name, type_name);
    let py_struct = format_ident!("Py{}", struct_name);
    let field_accessors = gen_type_field_accessors(data);
    let registration = gen_registration(&py_struct, &full_name);

    quote! {
        // The wrapper holds a statically-typed `TypedHandle<#struct_name>` and
        // behaves like one: every method derefs through the active context to the
        // concrete type.
        #[::pliron_python::pyo3::pyclass(unsendable, name = #py_class_name, crate = "::pliron_python::pyo3")]
        pub struct #py_struct {
            pub(crate) ptr: ::pliron::r#type::TypedHandle<#struct_name>,
        }

        #[::pliron_python::pyo3::pymethods(crate = "::pliron_python::pyo3")]
        impl #py_struct {
            /// Downcast a generic `Type` to this concrete type.
            /// Returns `None` if the handle does not refer to this type.
            #[staticmethod]
            fn from_type(
                ty: &::pliron_python::types::PyType,
            ) -> ::pliron_python::pyo3::PyResult<::core::option::Option<Self>> {
                let ctx = ::pliron_python::get_ctx()?;
                if ty.ptr.deref(ctx).downcast_ref::<#struct_name>().is_some() {
                    Ok(Some(Self {
                        ptr: ::pliron::r#type::TypedHandle::from_handle_unchecked(ty.ptr),
                    }))
                } else {
                    Ok(None)
                }
            }

            /// Project to the generic `Type` handle.
            fn to_type(&self) -> ::pliron_python::types::PyType {
                ::pliron_python::types::PyType { ptr: self.ptr.to_handle() }
            }

            fn __str__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
                let ctx = ::pliron_python::get_ctx()?;
                Ok(::pliron::alloc::format!(
                    "{}",
                    ::pliron::printable::Printable::disp(&self.ptr.to_handle(), ctx)
                ))
            }

            fn __repr__(&self) -> ::pliron_python::pyo3::PyResult<::pliron::alloc::string::String> {
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

        // Teach pliron-python about the Python wrapper for this type. The blanket
        // impl `impl<T: PyMapTarget> PyMap for TypedHandle<T>` in pliron-python then
        // routes `TypedHandle<MyType>` returns to `PyMyType` automatically. This
        // indirection is needed because a direct `impl PyMap for TypedHandle<MyType>`
        // here would violate the orphan rule when `MyType` is declared outside
        // pliron-python.
        impl ::pliron_python::PyTypeWrapper for #py_struct {
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

        impl ::pliron_python::PyMapTarget for #struct_name {
            type PyClass = #py_struct;
        }

        #registration
    }
}

/// Generate Python getter methods for each struct field of a *type*.
///
/// Named fields get `get_<field_name>(&self)`, tuple fields get `get_0`, `get_1`, etc.
/// **Always returns `PyResult<T>`** because accessing fields requires dereferencing
/// the `Ptr<TypeObj>` via `&Context`, which is obtained from the thread-local context.
///
/// Trivial-typed fields return the type directly; other fields go through `PyMap`.
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
                fn #method_name(&self) -> ::pliron_python::pyo3::PyResult<#field_ty> {
                    let ctx = ::pliron_python::get_ctx()?;
                    let __inner = self.ptr.deref(ctx);
                    Ok(__inner.#field_access.clone())
                }
            },
            _ => quote! {
                fn #method_name(&self) -> ::pliron_python::pyo3::PyResult<<#field_ty as #pymap>::Owned> {
                    let ctx = ::pliron_python::get_ctx()?;
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
