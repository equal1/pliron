# 06 — Type exposure

How pliron types are represented and exposed in the Python API. This is the
authoritative reference for the type layer; the general codegen/`PyMap` mechanics
live in [02-pymap-and-codegen.md](02-pymap-and-codegen.md).

## The two faces of a type

A pliron type is a uniqued, immutable value living in the active `Context`,
addressed two ways in Rust:

- **`TypeHandle`** — an untyped index into the context's type store. `deref(ctx)`
  yields a `Ref<'_, dyn Type>`.
- **`TypedHandle<T>`** — a `TypeHandle` statically tagged with the concrete Rust
  type `T`. `deref(ctx)` yields a `Ref<'_, T>` directly.

Python mirrors exactly this split, and **never exposes a raw `&dyn Type` or
`TypeObj`** — every type object owns a handle and re-borrows through the active
context on each call.

| Rust | Python class | Wraps | Role |
|---|---|---|---|
| `TypeHandle` | `Type` (`PyType`) | `TypeHandle` | the generic type face |
| `TypedHandle<T>` | `<Name>` (`Py<Name>`) | `TypedHandle<T>` | the concrete type face |

### `PyType` — the generic face (`src/python/types.rs`)

```rust
#[pyclass(unsendable, name = "Type")]
pub struct PyType { pub ptr: TypeHandle }
```

`PyType` is everything you can do with a `TypeHandle` (and, by deref, a
`&dyn Type`). Each method is `self.ptr.deref(ctx).<method>(…)` against the
thread-local context. The exposed surface is the **curated, Python-meaningful
subset** of the `Type` trait (not a mechanical 1:1 mirror — `eq_type(&dyn Type)`,
`verify_interfaces`, `register_instance`, raw `hash_type`/`get_self_handle` stay
internal):

- `type_name()` — the fully-qualified type id (`"builtin.integer"`).
- `verify()` — verify the type and its interfaces.
- `__str__`/`__repr__` — pretty-print (`i32`, `si64`, …).
- `__eq__`/`__hash__` — handle identity (types are uniqued, so this is semantic
  equality).

Adding more is a one-liner following the same `deref(ctx).<method>` shape.

### `Py<Name>` — the concrete face (derive-generated)

```rust
#[pyclass(unsendable, name = "IntegerType")]
pub struct PyIntegerType { pub(crate) ptr: TypedHandle<IntegerType> }
```

A `#[pliron_type]` struct generates a `Py<Name>` wrapper that **holds a
`TypedHandle<Name>` and behaves like one**. Because the handle is statically
typed, `self.ptr.deref(ctx)` is a `Ref<Name>` — field getters and
`#[pliron_type_impl]` methods call straight into the concrete Rust type, with no
runtime downcast.

Generated members:

- `get_<field>()` — one per struct field. Trivial fields return as-is; others go
  through `PyMap` (`<FieldTy as PyMap>::Owned`). Implemented as
  `self.ptr.deref(ctx).<field>.clone()`.
- `to_type() -> Type` — project to the generic `PyType`.
- `from_type(ty: Type) -> Optional[Self]` (`@staticmethod`) — downcast a generic
  `Type`; returns `None` when the handle isn't this concrete type, and raises
  only when there is no active context.
- `__str__`/`__repr__`/`__eq__`/`__hash__` — via the underlying `TypeHandle`.

`#[pliron_type_impl]` methods are exposed the same way (`02` §"impl-block
methods"): instance methods deref `TypedHandle<T> → Ref<T>` and call the native
method; `&Context`/`&mut Context` is dropped and supplied from the thread-local.

## Projecting between the two

```
            to_type()                  generic ops
 Py<Name>  ───────────────►  PyType  ───────────────► type_name(), verify(), str…
    ▲                          │
    └──────────────────────────┘
            from_type()  (Optional — None on type mismatch)
```

- **Concrete → generic**: `i32.to_type()` gives a `Type` you can pass anywhere a
  generic type is expected, or call the generic ops on.
- **Generic → concrete**: `IntegerType.from_type(some_type)` returns the typed
  wrapper or `None`.

From Python:

```python
i32 = pliron.builtin.IntegerType.get(32)     # Py<IntegerType>
i32.get_width()                       # concrete accessor -> 32
i32.to_type().type_name()             # project, then a generic op
maybe = pliron.builtin.IntegerType.from_type(i32.to_type())   # -> IntegerType | None
```

## How this flows through `PyMap`

`PyMap` (the Rust↔Python bridge; see [02](02-pymap-and-codegen.md)) routes types
so that **every `TypeHandle` becomes a `PyType` and every `TypedHandle<T>` becomes
that type's concrete wrapper**:

```rust
impl PyMap for TypeHandle {            // generic
    type Owned = PyType;  type Borrowed<'py> = PyRef<'py, PyType>;
}

impl<T: Type + PyMapTarget> PyMap for TypedHandle<T> {   // concrete (blanket)
    type Owned = T::PyClass;  type Borrowed<'py> = PyRef<'py, T::PyClass>;
    fn into_py(self) -> T::PyClass { T::PyClass::from_typed_handle(self) }
    fn from_py(py) -> Self { py.to_typed_handle() }
}
```

The blanket impl is lifted from two derive-emitted traits (orphan-rule
workaround — both `PyMap` and `TypedHandle` are foreign to a dialect crate):

```rust
pub trait PyTypeWrapper: pyo3::PyClass {
    type Concrete: Type;
    fn from_typed_handle(h: TypedHandle<Self::Concrete>) -> Self;
    fn to_typed_handle(&self) -> TypedHandle<Self::Concrete>;
}
pub trait PyMapTarget { type PyClass: PyTypeWrapper<Concrete = Self>; }
```

`#[pliron_type]` emits `impl PyTypeWrapper for Py<Name>` (with
`type Concrete = Name`) and `impl PyMapTarget for Name { type PyClass = Py<Name> }`.

Consequence: a derive-generated method that takes or returns a `TypedHandle<T>`
(e.g. a `get` constructor) automatically speaks the concrete wrapper, and one
returning a bare `TypeHandle` speaks the generic `PyType`.

## Consuming a type generically

A typed wrapper (`PyIntegerType`) is a distinct PyO3 class from `PyType`, so a
parameter typed as `PyType` won't accept it directly. Hand-written constructors
that take "any type" (`FunctionType.get`, `IRInserter.create_block`,
`IntegerAttr.new`) use the coercion helper
[`type_handle_from_any`](../../src/python/types.rs) — it accepts a generic `Type`
or projects any typed wrapper via its `to_type()` method (mirroring how
`set_attribute` coerces attributes via `into_attr()`):

```rust
pub fn type_handle_from_any(obj: &Bound<'_, PyAny>) -> PyResult<TypeHandle> {
    if let Ok(ty) = obj.extract::<PyRef<'_, PyType>>() { return Ok(ty.ptr); }
    let generic = obj.call_method0("to_type")?;          // typed wrapper -> generic
    Ok(generic.extract::<PyRef<'_, PyType>>()?.ptr)
}
```

So `FunctionType.get([i32], [i64])` works whether the elements are generic `Type`
or typed wrappers like `PyIntegerType`.

> **Equality across the two faces.** A generic `Type` and a typed wrapper for the
> *same* underlying type are different Python classes that hash differently, so
> `value.get_type() == IntegerType.get(32)` is `False`. Normalize one side first:
> `value.get_type() == i32.to_type()`.

## Related: IR-handle `PyMap` impls

The same "handle → its `#[pyclass]` wrapper" pattern backs the other IR nodes, so
derive-generated op/attr methods can take and return them (e.g. an op constructor
taking `Vec<Value>`, or an accessor returning a block):

| Rust | Python wrapper |
|---|---|
| `Value` | `PyValue` |
| `Ptr<Operation>` | `PyOperation` |
| `Ptr<BasicBlock>` | `PyBasicBlock` |
| `Ptr<Region>` | `PyRegion` |

These live alongside the type impls in [`src/python/py_map.rs`](../../src/python/py_map.rs).

## Adding a new type (cookbook)

1. Define it with `#[pliron_type(name = "dialect.foo", …)]`. You automatically get
   `PyFoo { ptr: TypedHandle<Foo> }` with `to_type`/`from_type`, field getters,
   the `PyTypeWrapper`/`PyMapTarget` impls, and registration.
2. Make sure each field type has a `PyMap` impl (primitives/`String`/`Vec`/`Option`
   are free; other field types need one `impl PyMap`).
3. Expose the constructor: a hand-written `#[staticmethod] get(...)` that returns
   `Py<Foo> { ptr: <TypedHandle from Foo::get> }`, or annotate the `impl` with
   `#[pliron_type_impl]`.
