# 02 — The `PyMap` bridge and derive codegen

This is the core of the design. Dialect authors write **only** their normal
`#[pliron_type]` / `#[pliron_attr]` / `#[pliron_op]` definitions; the
`pliron-python-derive` macros generate the entire Python `#[pyclass]` surface
from them — either from the *reflect token exports* the pliron derives leave
behind, or stacked directly as attributes on the definitions. The one extension
point dialect authors may touch is the `PyMap` trait, to teach the codegen how
a new Rust field type crosses the language boundary.

## The reflect-export mechanism

The core `pliron` crate is Python-free, so the Python codegen cannot run inside
`def_op`/`def_attribute`/`def_type`. Instead those macros (and thus
`pliron_op`/`pliron_attr`/`pliron_type`) emit, for every annotated item, an
inert Python-agnostic token export
([`pliron-derive/src/reflect.rs`](../../pliron-derive/src/reflect.rs)):

```rust
#[doc(hidden)]
#[macro_export]
macro_rules! __pliron_reflect_<kind>_<Ident> { /* kind: op | attr | ty */ }
```

The `#[pliron_op_impl]`/`#[pliron_attr_impl]`/`#[pliron_type_impl]` macros
still exist but are now thin hooks: they re-emit the `impl` block unchanged and
additionally export its tokens (with function bodies stripped — consumers only
need signatures) as `__pliron_reflect_<op_impl|attr_impl|ty_impl>_<SelfTy>`.

Invoking such a macro with another macro's path as its sole argument forwards
the item's tokens to that macro, wrapped in a **versioned envelope**:

```rust
pliron::__pliron_reflect_op_ModuleOp!(::pliron_python::derive::py_op_from_export);
// expands to:
::pliron_python::derive::py_op_from_export! {
    pliron_reflect_v1,
    kind: op,                  // op | attr | ty | op_impl | attr_impl | ty_impl
    ident: ModuleOp,
    name: "builtin.module",    // absent for *_impl kinds
    item: { /* item tokens */ }
}
```

The exported macros carry no runtime or dependency cost — they are plain token
containers, and the envelope version (`pliron_reflect_v1`) is the only contract
between `pliron` and the bindings. One caveat: `#[macro_export]` macros live in
a flat crate-root namespace, so annotate at most one `impl` block per type with
a `pliron_*_impl` hook.

`pliron-python-derive` provides every generator in two forms
([`pliron-python-derive/src/lib.rs`](../../pliron-python-derive/src/lib.rs)):

- **Callback macros** `py_op_from_export!`, `py_attr_from_export!`,
  `py_type_from_export!`, `py_op_impl_from_export!`, `py_attr_impl_from_export!`,
  `py_type_impl_from_export!` — consume a reflect envelope, for items defined in
  a *foreign* crate. This is how `pliron-python/src/dialects/builtin.rs`
  instantiates the builtin dialect's wrappers.
- **Attribute macros** `#[py_op]`, `#[py_attr]`, `#[py_type]`, `#[py_op_impl]`,
  `#[py_attr_impl]`, `#[py_type_impl]` — for items in the crate you are writing.
  The class ones read the `"dialect.name"` from the sibling
  `#[pliron_op(name = …)]` / `#[def_op("…")]` attribute (stack them *above* it),
  or accept `name = "…"` directly.

Generated code references runtime machinery by absolute `::pliron_python::*`
paths and is **not** cfg-gated — downstream crates gate at the call site with
`#[cfg_attr(feature = "python", pliron_python::derive::py_op)]` (see
[05-extending.md](05-extending.md)).

## The `PyMap` trait

`PyMap` ([`pliron-python/src/py_map.rs`](../src/py_map.rs)) is **invisible to
Python**. It exists purely so that derive-generated code can ask, at compile
time, "what Python type does this Rust field map to, and how do I convert?"

```rust
pub trait PyMap: Sized {
    type Owned;                                  // returned by generated getters
    type Borrowed<'py>: FromPyObject<'py>;       // accepted by generated params
    fn into_py(self) -> Self::Owned;             // Rust value  -> Python
    fn from_py(py: Self::Borrowed<'_>) -> Self;  // Python      -> Rust value
}
```

### Why two associated types

PyO3 is asymmetric about `#[pyclass]` values:

- A `#[pyclass]` can be **returned** by value (`-> PyType`).
- A `#[pyclass]` **cannot be received** by value as a parameter; it must be
  borrowed (`x: PyRef<'_, PyType>`).
- Primitives are symmetric in both positions.

So `Owned` is what a getter returns and `Borrowed<'py>` is what a setter/
constructor accepts. For a primitive `T`, both are `T`; for a pyclass-wrapped
type they differ (value vs `PyRef`).

### Core impls shipped with the library

| Rust type | `Owned` | Notes |
|---|---|---|
| `Identifier` | `String` | round-trips via `Identifier::try_new` |
| `TypeHandle` | `PyType` | the generic type handle |
| `TypedHandle<T>` | `T::PyClass` | blanket impl, via `PyMapTarget` (below) |
| `AttrObj` | `PyAttribute` | clones the boxed `dyn Attribute` |
| `Value` | `PyValue` | SSA value |
| `Ptr<Operation>` / `Ptr<BasicBlock>` / `Ptr<Region>` | `PyOperation` / `PyBasicBlock` / `PyRegion` | IR-node handles |
| `Vec<T: PyMap>` | `Vec<T::Owned>` | blanket |
| `Option<T: PyMap>` | `Option<T::Owned>` | blanket |

Builtin-specific impls also live here: `Signedness → String`
("signed"/"unsigned"/"signless"), `APInt → String` (one-way: `from_py` is
`unimplemented!`), `Single`/`Double`/`Half → f64` (`Half` via pliron's
`apfloat::half_to_f64`/`f64_to_half` helpers),
`AttributeDict → HashMap<String, PyAttribute>`. These are "reasonable starting
points" — a dialect can replace any of them with a richer `#[pyclass]` wrapper.

### The orphan-rule workaround for `TypedHandle<T>`

A downstream crate cannot write `impl PyMap for TypedHandle<MyType>` — both
`PyMap` and `TypedHandle` are foreign there. So the codegen splits it:

```rust
pub trait PyTypeWrapper: pyo3::PyClass + Sized {   // shape of a Py<Name> type wrapper
    type Concrete: Type;
    fn from_typed_handle(h: TypedHandle<Self::Concrete>) -> Self;
    fn to_typed_handle(&self) -> TypedHandle<Self::Concrete>;
}
pub trait PyMapTarget { type PyClass: PyTypeWrapper<Concrete = Self>; }   // links a Type to its Py wrapper

// blanket impl in pliron-python (legal — PyMap defined here):
impl<T: Type + PyMapTarget + 'static> PyMap for TypedHandle<T> {
    type Owned = T::PyClass;
    type Borrowed<'py> = PyRef<'py, T::PyClass>;
    fn into_py(self)  -> T::PyClass { T::PyClass::from_typed_handle(self) }
    fn from_py(py)     -> Self      { py.to_typed_handle() }
}
```

The generated type class emits `impl PyMapTarget for MyType` (legal wherever
the wrapper is generated) and `impl PyTypeWrapper for PyMyType`. The blanket
impl then lifts those into a full `PyMap for TypedHandle<MyType>`
automatically. **This is why a dialect needs zero hand-written PyO3 code to
expose its types.** Full details in [06-type-exposure.md](06-type-exposure.md).

## The type-mapping decision tree

A small module,
[`pliron-python-derive/src/py_type_mapper.rs`](../../pliron-python-derive/src/py_type_mapper.rs),
classifies each field/parameter type into one of three kinds. It deliberately
knows almost nothing about domain types:

```
classify(ty):
  &Context / &mut Context           -> ContextParam   // dropped from the Python signature;
                                                       //   supplied from the thread-local
  pyo3-native trivial type          -> Trivial        // emitted as-is, identity conversion
  everything else                   -> PyMapped        // routed through the PyMap trait
```

`Trivial` (recursive) = `bool`, the integer/float primitives, `String`, `&str`,
and `Vec<T>`/`Option<T>` **iff** their innermost element is itself trivial. So
`Vec<u32>` and `Option<String>` are trivial and hardcoded; `Vec<TypePtr<T>>`,
`Ptr<TypeObj>`, attribute types, etc. all fall through to `PyMapped` and are
resolved by `PyMap` in the generated code. A `PyMapped` type with no `PyMap` impl
is a **compile error at the generated call site** — intentional, never a silent
drop.

`Owned` vs `Borrowed` is then chosen purely by position:

- **Getter / return position** → `<Ty as PyMap>::Owned`, value via `into_py`.
- **Parameter position** → `<Ty as PyMap>::Borrowed<'_>`, value via `from_py`.

`Self` / `TypePtr<Self>` in a return type is substituted to the concrete type
before classification (`substitute_self`), so `fn get(...) -> TypePtr<Self>`
becomes a getter returning the concrete `Py<Name>`.

## What the type generator emits

`#[pliron_type]` is a facade that expands to `#[def_type]` (+ optional
`format_type`/`verify_succ`/`derive_type_get`) and leaves behind the
`__pliron_reflect_ty_<Name>` export. **All Python codegen lives in
`gen_py_type_class`** in
[`pliron-python-derive/src/type_class.rs`](../../pliron-python-derive/src/type_class.rs),
reached via `py_type_from_export!` / `#[py_type]`. Nothing is cfg-gated in the
output; gating (if wanted) is applied by the caller via `cfg_attr`.

In short: it emits `Py<Name> { ptr: TypedHandle<Name> }` (exposed under the bare
struct name) with `to_type`/`from_type` projections, `get_<field>` getters,
`__str__`/`__repr__`/`__eq__`/`__hash__`, the `PyTypeWrapper`/`PyMapTarget`
orphan-rule bridge, and a `pliron_python::statics::PY_CLASS_REGISTRATIONS`
entry. The class generator emits **no constructor** — a type's uniqued `get` is
exposed via the impl mirror (`#[py_type_impl]` /
`py_type_impl_from_export!`) or hand-written (the builtins hand-write theirs in
[`pliron-python/src/dialects/builtin.rs`](../src/dialects/builtin.rs)).

The full type-layer design — the generic `Type` face vs. the concrete `Py<Name>`
face, the `TypedHandle<T>` representation, and `to_type`/`from_type` — lives in
**[06-type-exposure.md](06-type-exposure.md)**. Attributes and ops are covered
below.

## What the attribute generator emits

`gen_py_attr_class` in
[`attr_class.rs`](../../pliron-python-derive/src/attr_class.rs)
(via `py_attr_from_export!` / `#[py_attr]`) mirrors the type path. Attributes
are **owned values** (not arena-interned), so the wrapper holds the **concrete
struct by value** — the analogue of the type wrapper holding a
`TypedHandle<T>`:

```rust
#[pyclass(unsendable, name = "StringAttr", crate = "::pliron_python::pyo3")]
pub struct PyStringAttr { pub(crate) inner: StringAttr }   // concrete, not Box<dyn Attribute>
```

Because it holds the concrete type, methods call straight into it with **no
runtime downcast** (field getters are `self.inner.<field>.clone()`).

Generated methods:

- `from_attr(attr: &PyAttribute) -> Option<Self>` (`#[staticmethod]`) —
  downcasts `attr.inner`, returning `None` on type mismatch.
- `into_attr(&self) -> PyAttribute` — boxes the concrete struct into a generic
  `Attribute`. This is **the typed→generic coercion convention**:
  `PyOperation.set_attribute` first tries to extract a `PyAttribute`, and on
  failure calls `into_attr()` on the passed object.
- the curated `Attribute`-trait surface: `attr_name()`, `verify()`,
  `__str__`/`__repr__`, `__eq__`/`__ne__` (via `eq_attr`), `__hash__` (hashes the
  canonical text, so `a == b ⇒ hash(a) == hash(b)`), `clone_attr()`.
- field getters (`get_<field>` / `get_N`), same `Owned`/Trivial rule.
- `impl PyMap for Name` with `Owned = PyName`, `Borrowed = PyRef<'_, PyName>` —
  `into_py` moves the struct in, `from_py` clones it out.
- registration entry keyed by `"dialect.attr"`.

No `#[new]` is generated; constructors come from the attr-impl mirror
(`#[py_attr_impl]` / `py_attr_impl_from_export!`) or are hand-written.
Mirrored methods borrow `&self.inner` (the concrete struct) directly and call
the native method.

## What the op generator emits

`gen_py_op_class` in [`op_class.rs`](../../pliron-python-derive/src/op_class.rs)
(via `py_op_from_export!` / `#[py_op]`) produces a thin pointer wrapper:

```rust
#[pyclass(unsendable, name = "ModuleOp", crate = "::pliron_python::pyo3")]
pub struct PyModuleOp { pub(crate) ptr: Ptr<Operation> }
```

Generated methods are intentionally minimal:

- `from_operation(op: &PyOperation) -> PyResult<Self>` (`#[staticmethod]`) —
  compares opids, errors on mismatch.
- `operation(&self) -> PyOperation` — typed→generic projection (a pointer copy;
  both are `Ptr<Operation>` newtypes).
- `__str__`/`__repr__`.
- `impl PyMap for Op` and a registration entry keyed by `"dialect.op"`.

Structural inspection (operands, results, regions, attributes, navigation) is
**not** duplicated onto the typed wrapper — it lives on the core `PyOperation`
(see [03-core-classes.md](03-core-classes.md)). Construction is also not
generated by the class generator: ops are built by dialect-authored factory
methods (`ModuleOp.new(...)`) — mirrored from the Rust `impl` block via the
op-impl mirror below — that build the operands/results/regions Rust-side, then
placed with `IRInserter`/`insert_*`.

## Exposing `impl`-block methods: the impl mirrors

A second family of generators — `type_impl.rs`, `attr_impl.rs`, `op_impl.rs` in
`pliron-python-derive` (reached via `#[py_type_impl]`/`#[py_attr_impl]`/
`#[py_op_impl]` or the `py_*_impl_from_export!` macros consuming a
`#[pliron_*_impl]` hook's export) — mirrors the **public methods** of an `impl`
block into the generated `Py<Name>`'s `#[pymethods]`. For each `pub fn`:

- `&Context`/`&mut Context` params are **dropped** and injected from the
  thread-local.
- `PyMapped` params become `<Ty as PyMap>::Borrowed<'_>` (converted via
  `from_py`); trivial params pass through.
- `PyMapped` returns become `<Ty as PyMap>::Owned` (via `into_py`); `Result<T,E>`
  becomes `PyResult<…>` with `to_py_err`; `Self` is substituted to the concrete
  type.
- instance methods on a `Type` always need the context (the handle must be
  deref'd), so they always return `PyResult<…>`; op methods inject the context
  (and become `PyResult`) only when the Rust signature takes
  `&Context`/`&mut Context`.

This is the clean path for exposing a uniqued constructor (e.g. `Type::get`) or a
typed accessor without writing PyO3 by hand.

## Naming conventions (cheat-sheet)

| Concept | Rule | Example |
|---|---|---|
| Generated wrapper struct | `Py` + struct name | `IntegerType` → `PyIntegerType` |
| Python class name | bare struct name | `"IntegerType"` |
| Registration `name` | full `"dialect.name"` | `"builtin.integer"` |
| Named-field getter | `get_<field>` | `get_width` |
| Tuple-field getter | `get_<index>` | `get_0` |
| Type ↔ generic | `from_type` / `to_type` | |
| Attr ↔ generic | `from_attr` / `into_attr` | |
| Op ↔ generic | `from_operation` / `operation` | |
| Dunders | `__str__`, `__repr__`, `__eq__`, `__hash__` | |

## Worked example (type)

Source, in `pliron` (python-free):

```rust
#[pliron_type(name = "builtin.integer", generate_get = true, verifier = "succ")]
pub struct IntegerType { pub width: u32, pub signedness: Signedness }
```

This leaves behind `pliron::__pliron_reflect_ty_IntegerType!`, which
`pliron-python/src/dialects/builtin.rs` invokes:

```rust
pliron::__pliron_reflect_ty_IntegerType!(crate::derive::py_type_from_export);
```

Generated (abbreviated — the wrapper holds a `TypedHandle<IntegerType>`, so
`deref(ctx)` is a `Ref<IntegerType>` and field getters need no downcast; all
paths are absolute `::pliron_python::*`):

```rust
#[pyclass(unsendable, name = "IntegerType", crate = "::pliron_python::pyo3")]
pub struct PyIntegerType { pub(crate) ptr: TypedHandle<IntegerType> }

#[pymethods(crate = "::pliron_python::pyo3")]
impl PyIntegerType {
    #[staticmethod]
    fn from_type(ty: &PyType) -> PyResult<Option<Self>> { /* None on type mismatch */ }
    fn to_type(&self) -> PyType { PyType { ptr: self.ptr.to_handle() } }
    fn __str__(&self) -> PyResult<String> { /* Printable::disp via get_ctx() */ }
    fn __repr__(&self) -> PyResult<String> { self.__str__() }
    fn __eq__(&self, other: &Self) -> bool { self.ptr.to_handle() == other.ptr.to_handle() }
    fn __hash__(&self) -> usize { /* hash to_handle() */ }

    fn get_width(&self) -> PyResult<u32> {            // u32 is Trivial -> direct clone
        let ctx = ::pliron_python::get_ctx()?;
        Ok(self.ptr.deref(ctx).width.clone())
    }
    fn get_signedness(&self) -> PyResult<String> {    // Signedness is PyMapped -> into_py
        let ctx = ::pliron_python::get_ctx()?;
        Ok(<Signedness as PyMap>::into_py(self.ptr.deref(ctx).signedness.clone()))
    }
}

impl ::pliron_python::PyTypeWrapper for PyIntegerType { /* type Concrete = IntegerType; from/to_typed_handle */ }
impl ::pliron_python::PyMapTarget for IntegerType { type PyClass = PyIntegerType; }
const _: () = { /* push __PY_REG{"builtin.integer"} into ::pliron_python::statics::PY_CLASS_REGISTRATIONS */ };
```

See [06-type-exposure.md](06-type-exposure.md) for the full type-layer design.

The constructor is **hand-written** next to the reflect invocation in
[`pliron-python/src/dialects/builtin.rs`](../src/dialects/builtin.rs) (PyO3
can't infer it):

```rust
#[pyo3::pymethods]
impl PyIntegerType {
    #[staticmethod]
    #[pyo3(signature = (width, signedness=None))]
    fn get(width: u32, signedness: Option<&str>) -> PyResult<PyIntegerType> {
        let ctx = crate::get_ctx_mut()?;
        let sign = match signedness.unwrap_or("signless") { "signed" => …, … };
        Ok(PyIntegerType { ptr: IntegerType::get(ctx, width, sign) })
    }
}
```

This second `#[pymethods]` block is exactly why the `multiple-pymethods` PyO3
feature is enabled.
