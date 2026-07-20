# 05 — Extending the bindings (cookbook + known gaps)

This is the practical checklist for adding to the Python surface in later phases,
plus the gaps still open.

## The two routes for a dialect crate

The core `pliron` crate is Python-free, so exposing a dialect is now a decision
made *by the bindings side*, item by item. There are two routes, sharing all
the same codegen ([`pliron-python-derive`](../../pliron-python-derive/src/lib.rs)):

**Route A — attribute macros in the dialect crate itself** (feature-gated).
Add `pliron-python` as an optional dependency and stack the `#[py_*]` attribute
macros *above* your `#[pliron_*]` definitions:

```toml
# mydialect/Cargo.toml
[dependencies]
pliron-python = { version = "0", optional = true }

[features]
python = ["dep:pliron-python"]
```

```rust
#[cfg_attr(feature = "python", pliron_python::derive::py_op)]
#[pliron_op(name = "mydialect.my_op")]
pub struct MyOp;

#[cfg_attr(feature = "python", pliron_python::derive::py_op_impl)]
#[pliron_op_impl]
impl MyOp {
    pub fn new(ctx: &mut Context, /* … */) -> Self { /* … */ }
}
```

The class macros (`py_op`/`py_attr`/`py_type`) read the `"dialect.name"` from
the sibling `#[pliron_op(name = …)]` / `#[def_op("…")]` attribute (which is why
they must be stacked above it), or accept `name = "…"` directly. No
`pyo3`/`linkme` dependency is needed — the generated code reaches everything
through `::pliron_python::*` re-exports.

**Route B — a separate bindings crate** consuming the reflect exports. Keep the
dialect crate untouched (it already exports `__pliron_reflect_*` macros for
every `#[pliron_*]` item, at zero cost) and invoke them from a crate that
depends on both the dialect and `pliron-python` — exactly what
[`pliron-python/src/dialects/builtin.rs`](../src/dialects/builtin.rs) does for
pliron's builtin dialect:

```rust
use mydialect::MyOp;   // generated code names the wrapped types by ident

mydialect::__pliron_reflect_op_MyOp!(pliron_python::derive::py_op_from_export);
mydialect::__pliron_reflect_op_impl_MyOp!(pliron_python::derive::py_op_impl_from_export);
```

The `use` statements matter: generated code references the wrapped types and
their method-signature types by the idents used at the definition site, so they
must be in scope at the invocation.

Either way, exposure is **per item**: a wrapper exists exactly for the items you
annotate or whose exports you invoke.

## Expose a new dialect type

1. Define it as usual: `#[pliron_type(name = "mydialect.foo", ...)]`, and pick a
   route above (stack `#[py_type]`, or invoke
   `__pliron_reflect_ty_Foo!(pliron_python::derive::py_type_from_export)`). That
   emits `PyFoo`, its field getters, `to_type`/`from_type`, and the registration
   entry.
2. Ensure every field type has a `PyMap` impl. Trivial fields (primitives,
   `String`, `Vec`/`Option` of those) need nothing. `TypePtr<T>` works
   automatically via the `PyMapTarget` blanket impl. Any other field type needs a
   one-off `impl PyMap for ThatType` (put it next to the wrapper generation,
   gated the same way).
3. Expose the constructor. Either mirror the `impl` block
   (`#[py_type_impl]` above `#[pliron_type_impl]`, or
   `__pliron_reflect_ty_impl_Foo!(… py_type_impl_from_export)` — auto-mirrors
   `pub fn get(...)`), or hand-write a `#[staticmethod] fn get(...)` in a second
   `#[pymethods]` block (the builtins do the latter — see
   `pliron-python/src/dialects/builtin.rs`). The second block requires the PyO3
   `multiple-pymethods` feature, which `pliron-python` already enables.

No edits to `pliron`, `pliron-python-derive`, or any central registry are needed —
`linkme` picks the class up at link time.

## Expose a new attribute

Same as a type, with `#[py_attr]` / `#[py_attr_impl]` (or the
`py_attr_from_export!` / `py_attr_impl_from_export!` macros). The generated
`into_attr()` makes it usable directly with `op.set_attribute(name, my_attr)`.
Field types again flow through `PyMap`.

## Expose a new op

`#[py_op]` (or `py_op_from_export!`) gives you `PyMyOp` with
`from_operation`/`operation` and printing. You then provide a constructor:
mirror the Rust `pub fn new(...)` via `#[py_op_impl]` /
`py_op_impl_from_export!`, or hand-write a `#[staticmethod] fn new(...)` that
builds the operands/results/regions Rust-side (via `Operation::new`).
Structural inspection comes for free through the core `PyOperation`.

## Teach `PyMap` a new field type

The single extension point for the type bridge. Implement both directions:

```rust
impl PyMap for MyScalar {
    type Owned = String;                 // or a #[pyclass] wrapper for richer access
    type Borrowed<'py> = String;
    fn into_py(self) -> String { /* ... */ }
    fn from_py(py: String) -> Self { /* ... */ }
}
```

Use a primitive/`String` `Owned` for a quick exposure; promote to a dedicated
`#[pyclass]` wrapper when Python needs structured access. Remember the
asymmetry: `Owned` is the return type, `Borrowed<'py>` (must be `FromPyObject`)
is the parameter type. (In route A, gate the impl behind your `python` feature
like the rest of the binding code.)

## Make a downstream dialect's classes actually load

The machinery is automatic, but two things must hold:

1. The crate holding the generated wrappers depends on `pliron-python` (route A:
   the dialect crate itself, via `python = ["dep:pliron-python"]`; route B: the
   separate bindings crate).
2. That crate is **linked into the final wheel's cdylib** (i.e. it is a
   dependency of the crate maturin builds). `linkme` only collects
   `PY_CLASS_REGISTRATIONS` entries from linked crates.

As of this commit, `pliron-python/Cargo.toml` depends only on `pliron` (whose
builtin dialect it binds itself), **not** on `pliron-llvm` — so LLVM ops are not
yet in the wheel despite the codegen supporting them. Adding an LLVM bindings
module (route B) and linking it into `pliron-python` is all that is required.

## Known gaps / caveats

- **Builtin exposure is still all-or-nothing in practice.** The refactor made
  exposure per-item (you choose which reflect exports to invoke / which items to
  annotate), but `pliron-python` simply invokes the exports for every builtin
  op/type/attr, so the whole builtin dialect is exposed. Selective builtin
  exposure would just mean deleting invocations in `src/dialects/builtin.rs`.
- **`pliron-llvm` not linked** into the wheel yet (above), so only builtin
  dialect classes are reachable from Python.
- **`.pyi` stubs are hand-maintained and partial.** `python/pliron/__init__.pyi`
  only stubs `PlironError`, `Context`, and `Operation`. The crate depends on
  `pyo3-stub-gen`, suggesting stub generation is meant to be automated later —
  but the committed stubs are manual and must be kept in sync. (The
  `__init__.py` shim itself is generic — nothing hand-maintained per class.)
- **`requires-python` mismatch.** `pyproject.toml` says `>=3.12` while the abi3
  floor is `py39`. Harmless but inconsistent.
- **One `pliron_*_impl` hook per type.** `#[macro_export]` macros live in a flat
  crate-root namespace, so a crate can export the reflect tokens of at most one
  `impl` block per type (and op/attr/type struct names must be unique per
  crate).
- **`APInt::from_py` is `unimplemented!`** — integer attribute values round-trip
  out to a string but cannot be reconstructed from one through the generic
  `PyMap` path (the builtin `IntegerAttr` works around this with a hand-written
  constructor that builds the `APInt` from the integer type's width).
- **Dangling handles panic.** A Python handle used after its IR entity is erased
  will panic on the next deref. There is no generation/liveness check.
- **No match/rewrite bridge.** The Python rewrite bindings (`Rewriter`,
  `apply_match_rewrite`) have been removed pending a re-spec, so rewrite patterns
  cannot currently be written in Python.
