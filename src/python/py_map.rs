//! [`PyMap`] — the compile-time bridge between Rust types and their Python-visible
//! representations.
//!
//! `PyMap` is **invisible to Python**. It is consumed only by `pliron-derive`'s code
//! generation to decide which Python type a Rust type maps to and how to convert
//! between the two.
//!
//! # Why two associated types
//!
//! PyO3 expects different shapes for parameters vs. return values:
//! - A `#[pyclass]` value can be returned by-value (`-> PyType`).
//! - A `#[pyclass]` value cannot be received by-value as a parameter; it must be
//!   borrowed (`x: PyRef<'_, PyType>`).
//! - Primitives behave the same in both positions.
//!
//! [`PyMap::Owned`] is what the generated `fn` returns; [`PyMap::Borrowed`] is what
//! it accepts. For primitive `T`, both are `T`. For pyclass-wrapped `T`, the latter
//! is a `PyRef`-style borrow.
//!
//! # Extending
//!
//! Dialect authors add support for a new Rust type by writing a single
//! `impl PyMap for ThatType { ... }` next to the type definition. No edits to
//! `pliron-derive` are required.

use pyo3::FromPyObject;

use alloc::{
    borrow::ToOwned,
    format,
    string::{String, ToString},
    vec::Vec,
};

#[cfg(feature = "python")]
use crate::builtin::types::Signedness;
use crate::{
    attribute::{AttrObj, AttributeDict},
    basic_block::BasicBlock,
    context::Ptr,
    identifier::Identifier,
    operation::Operation,
    region::Region,
    std_deps::hash::FxHashMap,
    r#type::TypeHandle,
    utils::{
        apfloat::{Double, Half, Single},
        apint::APInt,
    },
    value::Value,
};

use super::{
    attributes::PyAttribute, basic_block::PyBasicBlock, operation::PyOperation, region::PyRegion,
    types::PyType, value::PyValue,
};

/// Bridge a Rust type to its Python-visible representation.
///
/// Implement on the Rust value-type (e.g. `Identifier`, `Ptr<TypeObj>`, your dialect's
/// `MyType`). The generated `#[pymethods]` will use `Owned` in return position and
/// `Borrowed<'_>` in parameter position.
pub trait PyMap: Sized {
    /// The Python-visible type the generated code returns.
    /// MUST be one of: a PyO3-native scalar (`u32`, `String`, …), `Vec<T>` / `Option<T>`
    /// of such, or a `#[pyclass]` type.
    type Owned;

    /// The Python-visible type the generated code accepts as a parameter.
    /// MUST satisfy [`pyo3::FromPyObject`] so PyO3 can extract it from a Python call.
    type Borrowed<'py>: FromPyObject<'py>;

    /// Convert an owned Rust value to its Python representation.
    fn into_py(self) -> Self::Owned;

    /// Reconstruct the Rust value from a Python-side borrow.
    fn from_py(py: Self::Borrowed<'_>) -> Self;
}

// ---------------------------------------------------------------------------
// Core pliron types
// ---------------------------------------------------------------------------

impl PyMap for Identifier {
    type Owned = String;
    type Borrowed<'py> = String;

    fn into_py(self) -> String {
        self.to_string()
    }

    fn from_py(py: String) -> Self {
        Identifier::try_new(py).expect("invalid Identifier from Python")
    }
}

impl PyMap for TypeHandle {
    type Owned = PyType;
    type Borrowed<'py> = pyo3::PyRef<'py, PyType>;

    fn into_py(self) -> PyType {
        PyType { ptr: self }
    }

    fn from_py(py: pyo3::PyRef<'_, PyType>) -> Self {
        py.ptr
    }
}

impl PyMap for crate::r#type::TypeSig {
    // A function signature surfaces as a `(list[Type], list[Type])` tuple of
    // (argument types, result types).
    type Owned = (Vec<PyType>, Vec<PyType>);
    type Borrowed<'py> = (Vec<pyo3::PyRef<'py, PyType>>, Vec<pyo3::PyRef<'py, PyType>>);

    fn into_py(self) -> Self::Owned {
        (
            <Vec<TypeHandle> as PyMap>::into_py(self.arguments),
            <Vec<TypeHandle> as PyMap>::into_py(self.results),
        )
    }

    fn from_py(py: Self::Borrowed<'_>) -> Self {
        crate::r#type::TypeSig {
            arguments: <Vec<TypeHandle> as PyMap>::from_py(py.0),
            results: <Vec<TypeHandle> as PyMap>::from_py(py.1),
        }
    }
}

impl PyMap for AttrObj {
    type Owned = PyAttribute;
    type Borrowed<'py> = pyo3::PyRef<'py, PyAttribute>;

    fn into_py(self) -> PyAttribute {
        PyAttribute {
            inner: dyn_clone::clone_box(&*self),
        }
    }

    fn from_py(py: pyo3::PyRef<'_, PyAttribute>) -> Self {
        dyn_clone::clone_box(&*py.inner)
    }
}

// ---------------------------------------------------------------------------
// IR-node handles — each maps to its hand-written `#[pyclass]` wrapper. These
// let derive-generated methods accept/return SSA values and arena handles (e.g.
// an op constructor taking `Vec<Value>`, or an accessor returning a block).
// ---------------------------------------------------------------------------

impl PyMap for Value {
    type Owned = PyValue;
    type Borrowed<'py> = pyo3::PyRef<'py, PyValue>;

    fn into_py(self) -> PyValue {
        PyValue { val: self }
    }

    fn from_py(py: pyo3::PyRef<'_, PyValue>) -> Self {
        py.val
    }
}

impl PyMap for Ptr<Operation> {
    type Owned = PyOperation;
    type Borrowed<'py> = pyo3::PyRef<'py, PyOperation>;

    fn into_py(self) -> PyOperation {
        PyOperation { ptr: self }
    }

    fn from_py(py: pyo3::PyRef<'_, PyOperation>) -> Self {
        py.ptr
    }
}

impl PyMap for Ptr<BasicBlock> {
    type Owned = PyBasicBlock;
    type Borrowed<'py> = pyo3::PyRef<'py, PyBasicBlock>;

    fn into_py(self) -> PyBasicBlock {
        PyBasicBlock { ptr: self }
    }

    fn from_py(py: pyo3::PyRef<'_, PyBasicBlock>) -> Self {
        py.ptr
    }
}

impl PyMap for Ptr<Region> {
    type Owned = PyRegion;
    type Borrowed<'py> = pyo3::PyRef<'py, PyRegion>;

    fn into_py(self) -> PyRegion {
        PyRegion { ptr: self }
    }

    fn from_py(py: pyo3::PyRef<'_, PyRegion>) -> Self {
        py.ptr
    }
}

// ---------------------------------------------------------------------------
// PyMapTarget — indirection so downstream crates can teach pliron how to map
// `TypedHandle<T>` for a local type `T` without violating the orphan rule.
//
// `impl PyMap for TypedHandle<T>` emitted directly in a downstream crate violates
// the orphan rule (both `PyMap` and `TypedHandle` are foreign). Instead, the derive
// emits `impl PyMapTarget for T` in the downstream crate (legal — the self type
// is local), and the blanket impl below lifts that into `impl PyMap for TypedHandle<T>`.
// ---------------------------------------------------------------------------

/// Shape contract for a derive-generated `Py<TypeName>` wrapper.
///
/// The wrapper holds a [`TypedHandle<Concrete>`](crate::r#type::TypedHandle) and
/// behaves like one — every method projects through `deref(ctx)` to the concrete
/// `Type`. `#[pliron_type]` emits this impl automatically; downstream code should
/// not implement it by hand.
pub trait PyTypeWrapper: pyo3::PyClass + Sized {
    /// The concrete pliron [`Type`](crate::r#type::Type) this wrapper is the
    /// Python face of.
    type Concrete: crate::r#type::Type;

    fn from_typed_handle(handle: crate::r#type::TypedHandle<Self::Concrete>) -> Self;
    fn to_typed_handle(&self) -> crate::r#type::TypedHandle<Self::Concrete>;
}

/// Declare the `#[pyclass]` Python wrapper for a `Type`. Implemented automatically
/// by `#[pliron_type]`.
pub trait PyMapTarget {
    type PyClass: PyTypeWrapper<Concrete = Self>;
}

impl<T> PyMap for crate::r#type::TypedHandle<T>
where
    T: crate::r#type::Type + PyMapTarget + 'static,
{
    type Owned = T::PyClass;
    type Borrowed<'py> = pyo3::PyRef<'py, T::PyClass>;

    fn into_py(self) -> T::PyClass {
        T::PyClass::from_typed_handle(self)
    }

    fn from_py(py: pyo3::PyRef<'_, T::PyClass>) -> Self {
        py.to_typed_handle()
    }
}

// ---------------------------------------------------------------------------
// Library-provided defaults for non-pyclass std-rust types that appear as fields
// of derived attributes.
//
// These are reasonable starting points; dialect authors are free to override the
// Python representation by replacing the impl (e.g. with a dedicated `#[pyclass]`
// wrapper) once they need richer access from Python.
// ---------------------------------------------------------------------------

impl PyMap for APInt {
    type Owned = String;
    type Borrowed<'py> = String;

    fn into_py(self) -> String {
        format!("{:?}", self)
    }

    fn from_py(_py: String) -> Self {
        unimplemented!("PyMap::from_py for APInt is not yet supported")
    }
}

impl PyMap for Single {
    type Owned = f64;
    type Borrowed<'py> = f64;

    fn into_py(self) -> f64 {
        crate::utils::apfloat::single_to_f32(self) as f64
    }

    fn from_py(py: f64) -> Self {
        crate::utils::apfloat::f32_to_single(py as f32)
    }
}

impl PyMap for Double {
    type Owned = f64;
    type Borrowed<'py> = f64;

    fn into_py(self) -> f64 {
        crate::utils::apfloat::double_to_f64(self)
    }

    fn from_py(py: f64) -> Self {
        crate::utils::apfloat::f64_to_double(py)
    }
}

impl PyMap for Half {
    type Owned = f64;
    type Borrowed<'py> = f64;

    fn into_py(self) -> f64 {
        crate::utils::apfloat::half_to_f64(self)
    }

    fn from_py(py: f64) -> Self {
        crate::utils::apfloat::f64_to_half(py)
    }
}

impl PyMap for AttributeDict {
    type Owned = std::collections::HashMap<String, PyAttribute>;
    type Borrowed<'py> = std::collections::HashMap<String, pyo3::PyRef<'py, PyAttribute>>;

    fn into_py(self) -> Self::Owned {
        self.0
            .into_iter()
            .map(|(k, v)| (k.to_string(), <AttrObj as PyMap>::into_py(v)))
            .collect()
    }

    fn from_py(py: Self::Borrowed<'_>) -> Self {
        let map: FxHashMap<Identifier, AttrObj> = py
            .into_iter()
            .map(|(k, v)| {
                let id = Identifier::try_new(k).expect("invalid Identifier key from Python");
                let attr = dyn_clone::clone_box(&*v.inner);
                (id, attr)
            })
            .collect();
        AttributeDict(map)
    }
}

impl PyMap for Signedness {
    type Owned = ::std::string::String;
    type Borrowed<'py> = ::std::string::String;

    fn into_py(self) -> ::std::string::String {
        match self {
            Signedness::Signed => "signed".to_owned(),
            Signedness::Unsigned => "unsigned".to_owned(),
            Signedness::Signless => "signless".to_owned(),
        }
    }

    fn from_py(py: ::std::string::String) -> Self {
        match py.as_str() {
            "signed" => Signedness::Signed,
            "unsigned" => Signedness::Unsigned,
            "signless" => Signedness::Signless,
            other => panic!(
                "invalid Signedness value `{}`; expected one of: signed, unsigned, signless",
                other
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Generic wrappers — Vec, Option
//
// These blanket impls are reached only when the macro decides the inner type
// is non-trivial. Trivial `Vec<u32>` / `Option<u32>` are hardcoded by the macro
// and never use `PyMap`.
// ---------------------------------------------------------------------------

impl<T: PyMap> PyMap for Vec<T>
where
    for<'py> T::Borrowed<'py>: FromPyObject<'py>,
{
    type Owned = Vec<T::Owned>;
    type Borrowed<'py> = Vec<T::Borrowed<'py>>;

    fn into_py(self) -> Self::Owned {
        self.into_iter().map(T::into_py).collect()
    }

    fn from_py(py: Self::Borrowed<'_>) -> Self {
        py.into_iter().map(T::from_py).collect()
    }
}

impl<T: PyMap> PyMap for Option<T>
where
    for<'py> T::Borrowed<'py>: FromPyObject<'py>,
{
    type Owned = Option<T::Owned>;
    type Borrowed<'py> = Option<T::Borrowed<'py>>;

    fn into_py(self) -> Self::Owned {
        self.map(T::into_py)
    }

    fn from_py(py: Self::Borrowed<'_>) -> Self {
        py.map(T::from_py)
    }
}
