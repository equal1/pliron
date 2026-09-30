# 01 — Architecture

## Crate layout

There are four Rust pieces and one Python package:

| Piece | Path | Role |
|---|---|---|
| Core IR crate | `pliron` (repo root) | **100% Python-free** — no pyo3 dependency, no `python` feature. Its only contribution to the bindings is the reflect token exports emitted by `pliron-derive` (below), plus a few Python-agnostic `pub` API additions the out-of-crate codegen needs. |
| IR derive macros | `pliron-derive/src/` | `def_op`/`def_attribute`/`def_type` (and the unified `pliron_op`/`pliron_attr`/`pliron_type`) additionally emit, per item, an inert `#[macro_export] macro_rules! __pliron_reflect_<kind>_<Ident>` **token export** ([`reflect.rs`](../../pliron-derive/src/reflect.rs)); the `pliron_*_impl` hooks do the same for `impl` blocks. No Python codegen lives here. |
| Python codegen | `pliron-python-derive/src/` | Proc-macro crate holding **all** Python codegen: emits a `Py<Name>` `#[pyclass]` (and `#[pymethods]` impl mirrors) either from a reflect export (`py_*_from_export!`) or as stacked attributes (`#[py_op]`, …). |
| Bindings crate | `pliron-python/` | `rlib` + `cdylib` (lib name `pliron_python`). Hand-written `#[pyclass]` wrappers for the fixed IR entities + inserter/rewriter + the registration machinery and `PyMap` bridge, the builtin-dialect wrappers (`src/dialects/builtin.rs`), and the `#[pymodule] fn _pliron`. **The cdylib is the importable extension module.** |
| Python package | `pliron-python/python/pliron/` | `__init__.py` shim that re-exports the native module + hand-written `__init__.pyi` stubs. |

The `#[pymodule]` lives at the bottom of
[`pliron-python/src/lib.rs`](../src/lib.rs):

```rust
#[pymodule]
fn _pliron(m: &Bound<'_, PyModule>) -> PyResult<()> {
    register_core_types(m)?;   // fixed core classes + PlironError
    register_all_classes(m)?;  // generated dialect classes (linkme slice)
    Ok(())
}
```

## Dependency direction: `pliron` knows nothing about Python

`pliron-python` depends on `pliron`, `pliron-python-derive`, and `pyo3`
directly ([`pliron-python/Cargo.toml`](../Cargo.toml)):

```toml
[lib]
name = "pliron_python"
crate-type = ["rlib", "cdylib"]

[dependencies]
pliron = { path = "../", version = "0" }
pliron-python-derive = { path = "../pliron-python-derive", version = "0" }
pyo3 = { version = "0.23", features = ["abi3-py39", "multiple-pymethods"] }
```

- `multiple-pymethods` is **required** because dialect classes routinely get two
  `#[pymethods]` blocks — one derive-generated, one hand-written constructor.
- `abi3-py39` builds a single stable-ABI wheel that loads on CPython ≥ 3.9.
- pyo3's `extension-module` feature is enabled via `pyproject.toml` (maturin),
  deliberately *not* as a cargo feature, so `rlib` consumers don't inherit it.

`pliron-python/src/lib.rs` re-exports the dependencies the generated code names
by absolute path, so downstream dialect crates need no direct
`pyo3`/`linkme`/`pliron-python-derive` dependency:

```rust
pub use pliron_python_derive as derive;  // generated-code entry points
pub use pyo3;                            // generated code names ::pliron_python::pyo3
pub use linkme;                          // (inventory on wasm)
```

Downstream dialects opt in with an optional dependency and a feature, e.g.
`python = ["dep:pliron-python"]`, then gate per item with
`#[cfg_attr(feature = "python", pliron_python::derive::py_op)]` — see
[05-extending.md](05-extending.md).

> **Design property: exposure is per-item, chosen by the bindings side.** The
> `#[pliron_type]`/`_attr`/`_op` macros themselves emit only the inert reflect
> export — the item's crate ships no Python code at all. A wrapper exists for
> exactly the items whose reflect export is invoked (or whose definition is
> stacked with a `#[py_*]` attribute). `pliron-python` currently invokes the
> exports for the whole builtin dialect, so builtin remains all-exposed.

## End goal: an independent bindings repository

`pliron-python` + `pliron-python-derive` are designed so they can eventually
move to their own repository. The only cross-repo contract is pliron's public
Rust API plus the **versioned reflect-envelope** format (`pliron_reflect_v1`,
see [02-pymap-and-codegen.md](02-pymap-and-codegen.md)) — nothing in `pliron`
or `pliron-derive` references the bindings. To support out-of-crate codegen,
`pliron` gained a few small Python-agnostic additions:
`TypedHandle::from_handle_unchecked`, `apfloat` half↔`f64` helpers, and `pub`
visibility on some builtin attr/type payload fields (accessors are now
generated outside the defining crate).

## The context model

pliron's IR lives in an arena owned by a `Context`; every handle (`Ptr<T>`,
`Value`) is meaningless without the `Context` it indexes into. Rust threads that
context explicitly through every call. Python cannot — it would be unbearable to
pass `ctx` to every method — so the binding installs a **thread-local active
context** ([`pliron-python/src/lib.rs`](../src/lib.rs)):

```rust
thread_local! {
    static ACTIVE_CTX_PTR: Cell<*mut ::pliron::context::Context> = Cell::new(null_mut());
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

A single Python exception type, created in [`pliron-python/src/lib.rs`](../src/lib.rs):

```rust
pyo3::create_exception!(pliron, PlironError, PyException, "Base exception for all pliron compiler errors.");
```

Every fallible binding funnels through it. `to_py_err(err: pliron::result::Error)`
`Display`-formats a core error into a `PlironError`. It is installed into the
module by `register_core_types` (`m.add("PlironError", …)`), so Python code can
`except pliron.PlironError`.

## Class registration via `linkme`

Core classes are registered explicitly; dialect classes register themselves.

**Core** — [`register_core_types`](../src/lib.rs) hand-lists the fixed
wrappers: `PyContext`, `PyOperation`, `PyBasicBlock`, `PyRegion`, `PyValue`,
`PyType`, `PyAttribute`, plus the `pliron.irbuild` submodule (inserter, rewriter,
insertion points, listeners, cloning) and the `PlironError` type.

**Dialect** — every generated class appends an entry to a global
`distributed_slice` ([`pliron-python/src/lib.rs`](../src/lib.rs)):

```rust
pub struct PyClassRegistration {
    pub name: &'static str,                              // e.g. "builtin.integer"
    pub register: fn(&Bound<'_, PyModule>) -> PyResult<()>,
}

pub mod statics {
    #[linkme::distributed_slice]
    pub static PY_CLASS_REGISTRATIONS: [PyClassRegistration] = [..];
}
```

`register_all_classes` walks the slice and calls each `register` fn against the
class's dialect submodule (`pliron.builtin`, `pliron.llvm`, …), created on
demand from the `"dialect."` prefix of `name` — see
[07-module-layout.md](07-module-layout.md). Because `linkme` collects slice
entries across *all linked crates*, merely linking a dialect crate into the
cdylib makes its classes appear in their module — no central list to
edit. The public `py_class_registration!` macro exposes the
same hook for fully hand-written classes; the generated code inlines the
equivalent `const _: () = { … }` block (referencing
`::pliron_python::statics::PY_CLASS_REGISTRATIONS`). (On `wasm`, where `linkme`
is unavailable, the same role is played by `inventory`.)

This deliberately mirrors pliron's existing `context_registration!` mechanism for
registering dialects, types, and ops into a `Context`.

## End-to-end: what happens at `import pliron`

1. CPython loads the `pliron._pliron` cdylib and calls the `#[pymodule]` init.
2. `register_core_types` adds the fixed wrappers and `PlironError`.
3. `register_all_classes` walks `PY_CLASS_REGISTRATIONS` (populated at link time
   by every generated wrapper — the builtin dialect from
   `pliron-python/src/dialects/builtin.rs`, plus any linked dialect crate) and
   adds each class.
4. The Python `pliron/__init__.py` shim re-exports `pliron._pliron.*` so users
   write `import pliron; pliron.builtin.IntegerType.get(32)`.
