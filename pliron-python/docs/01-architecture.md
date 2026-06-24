# 01 — Architecture

## Crate layout

There are three Rust pieces and one Python package:

| Piece | Path | Role |
|---|---|---|
| Core binding module | `src/python/` (in the `pliron` crate, gated by feature `python`) | Hand-written `#[pyclass]` wrappers for the fixed IR entities + builder/rewriter + the registration machinery and `PyMap` bridge. |
| Derive codegen | `pliron-derive/src/` | Emits a `Py<Name>` `#[pyclass]` for every dialect `#[pliron_type]` / `#[pliron_attr]` / `#[pliron_op]`. |
| Assembly crate | `pliron-python/` | A `cdylib` that depends on `pliron` with `features = ["python"]` and exposes the `#[pymodule] fn pliron(...)`. **This is the importable extension module.** |
| Python package | `pliron-python/python/pliron/` | `__init__.py` shim that re-exports the native module + hand-written `__init__.pyi` stubs. |

The assembly crate owns no IR logic. Its entire job is the `#[pymodule]`
([`pliron-python/src/lib.rs:14`](../src/lib.rs)):

```rust
#[pymodule]
fn pliron(m: &Bound<'_, PyModule>) -> PyResult<()> {
    ::pliron::python::register_core_types(m)?;   // fixed core classes
    ::pliron::python::register_all_classes(m)?;  // derive-generated dialect classes
    m.add_function(wrap_pyfunction!(::pliron::python::irbuild::rewriter::py_apply_match_rewrite, m)?)?;
    Ok(())
}
```

## The `python` cargo feature

Defined minimally in the root [`Cargo.toml`](../../Cargo.toml):

```toml
[features]
python = ["dep:pyo3"]

[dependencies.pyo3]
version = "0.23"
features = ["abi3-py39", "multiple-pymethods"]
optional = true
```

- It only flips on the optional `pyo3` dependency. Nothing else is gated by it
  directly.
- `multiple-pymethods` is **required** because dialect classes routinely get two
  `#[pymethods]` blocks — one derive-generated, one hand-written constructor.
- `abi3-py39` builds a single stable-ABI wheel that loads on CPython ≥ 3.9.

`src/lib.rs` re-exports the dependencies the generated code names by absolute
path, so downstream dialect crates need no direct `pyo3`/`linkme` dependency:

```rust
#[cfg(feature = "python")]
pub use pyo3;          // generated code names ::pliron::pyo3
#[cfg(feature = "python")]
pub mod python;
```

Downstream dialects opt in with a passthrough feature only, e.g.
`pliron-llvm/Cargo.toml`: `python = ["pliron/python"]`.

> **Important design property: exposure is all-or-nothing, not per-item.** The
> derive macros emit the Python `#[pyclass]` *unconditionally*, wrapped in
> `#[cfg(feature = "python")]`. There is no `python`-specific argument on
> `#[pliron_type]`/`_attr`/`_op` to opt a single type in or out. Turn on the
> feature and every derived type/attr/op in every linked dialect is exposed.

## The context model

pliron's IR lives in an arena owned by a `Context`; every handle (`Ptr<T>`,
`Value`) is meaningless without the `Context` it indexes into. Rust threads that
context explicitly through every call. Python cannot — it would be unbearable to
pass `ctx` to every method — so the binding installs a **thread-local active
context** ([`src/python/mod.rs:36`](../../src/python/mod.rs)):

```rust
thread_local! {
    static ACTIVE_CTX_PTR: Cell<*mut crate::context::Context> = Cell::new(null_mut());
}
```

- `PyContext` (`name = "Context"`) owns a `Box<Context>` for a stable address and
  acts as a **context manager**. `__enter__` calls `set_active_ctx(ptr)`;
  `__exit__` calls `clear_active_ctx()` and returns `False` so in-flight Python
  exceptions propagate.
- Every wrapper method begins with `let ctx = super::get_ctx()?;` (or
  `get_ctx_mut()?`). `get_ctx` returns `PlironError` if no context is active, and
  otherwise hands out a `&'static Context` by dereferencing the raw pointer.
- **Single active context per thread.** `set_active_ctx` errors if one is already
  active — nested or parallel contexts are unsupported. Sequential `with` blocks
  are fine.

Safety rests on two facts: the `PyContext` keeps the `Box<Context>` alive for the
duration of the `with` block, and Python's GIL serializes access so the
`&'static mut` aliasing never actually races. All `#[pyclass]`es are declared
`unsendable` to keep them on the owning thread.

Consequences to remember:

- A Python handle used **outside** any `with pliron.Context()` raises
  `PlironError("No active pliron context…")`.
- A handle outliving the entity it points to (e.g. after `op.erase()`) will
  **panic** when next dereferenced — this is a known sharp edge.

## Error model

A single Python exception type, created in [`src/python/mod.rs:104`](../../src/python/mod.rs):

```rust
pyo3::create_exception!(pliron, PlironError, PyException, "Base exception for all pliron compiler errors.");
```

Every fallible binding funnels through it. `to_py_err(err: pliron::result::Error)`
`Display`-formats a core error into a `PlironError`. It is installed into the
module by `register_core_types` (`m.add("PlironError", …)`), so Python code can
`except pliron.PlironError`.

## Class registration via `linkme`

Core classes are registered explicitly; dialect classes register themselves.

**Core** — [`register_core_types`](../../src/python/mod.rs) hand-lists the fixed
wrappers: `PyContext`, `PyOperation`, `PyBasicBlock`, `PyRegion`, `PyValue`,
`PyType`, `PyAttribute`, `PyIRBuilder`, `PyRewriter`, plus the `PlironError` type.

**Dialect** — every generated class appends an entry to a global
`distributed_slice` ([`src/python/mod.rs:152`](../../src/python/mod.rs)):

```rust
pub struct PyClassRegistration {
    pub name: &'static str,                              // e.g. "builtin.integer"
    pub register: fn(&Bound<'_, PyModule>) -> PyResult<()>,
}

#[::pliron::linkme::distributed_slice]
pub static PY_CLASS_REGISTRATIONS: [PyClassRegistration] = [..];
```

`register_all_classes` walks the slice and calls each `register` fn. Because
`linkme` collects slice entries across *all linked crates*, merely linking a
dialect crate into the assembly cdylib makes its classes appear in the module —
no central list to edit. The public `py_class_registration!` macro exposes the
same hook for fully hand-written classes; the derive macros inline the equivalent
`const _: () = { … }` block. (On `wasm`, where `linkme` is unavailable, the same
role is played by `inventory`.)

This deliberately mirrors pliron's existing `context_registration!` mechanism for
registering dialects, types, and ops into a `Context`.

## End-to-end: what happens at `import pliron`

1. CPython loads the `pliron` cdylib and calls the `#[pymodule]` init.
2. `register_core_types` adds the fixed wrappers and `PlironError`.
3. `register_all_classes` walks `PY_CLASS_REGISTRATIONS` (populated at link time
   by every derived type/attr/op in every linked dialect) and adds each class.
4. `apply_match_rewrite` is added as a free function.
5. The Python `pliron/__init__.py` shim re-exports `pliron.pliron.*` so users
   write `import pliron; pliron.IntegerType.get(32)`.
