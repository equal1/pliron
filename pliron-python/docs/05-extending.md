# 05 — Extending the bindings (cookbook + known gaps)

This is the practical checklist for adding to the Python surface in later phases,
plus the gaps left open by the first-phase commit.

## Expose a new dialect type

1. Define it as usual: `#[pliron_type(name = "mydialect.foo", ...)]`. The derive
   *already* emits `PyFoo`, its field getters, `to_type`/`from_type`, and the
   registration entry — gated by the `python` feature.
2. Ensure every field type has a `PyMap` impl. Trivial fields (primitives,
   `String`, `Vec`/`Option` of those) need nothing. `TypePtr<T>` works
   automatically via the `PyMapTarget` blanket impl. Any other field type needs a
   one-off `impl PyMap for ThatType` (put it next to the type, gated
   `#[cfg(feature = "python")]`).
3. Expose the constructor. Either annotate the `impl` block with
   `#[pliron_type_impl]` (auto-mirrors `pub fn get(...)`), or hand-write a
   `#[staticmethod] fn get(...)` in a second `#[pymethods]` block (the builtins do
   this — see `src/builtin/types.rs`). The second block requires the PyO3
   `multiple-pymethods` feature, which the `python` feature already enables.

No edits to `pliron-derive`, `pliron-python`, or any central registry are needed —
`linkme` picks the class up at link time.

## Expose a new attribute

Same as a type, with `#[pliron_attr]` / `#[pliron_attr_impl]`. The generated
`into_attr()` makes it usable directly with `op.set_attribute(name, my_attr)`.
Field types again flow through `PyMap`.

## Expose a new op

`#[pliron_op]` gives you `PyMyOp` with `from_operation`/`operation` and printing.
You then provide a constructor: a `#[staticmethod] fn new(...)` that builds the
operands/results/regions Rust-side (via `Operation::new`) and returns a generic
`PyOperation` — the builtin `ModuleOp::new` / `FuncOp::new` in
`src/builtin/ops.rs` are the templates. Structural inspection comes for free
through the core `PyOperation`.

## Teach `PyMap` a new field type

The single extension point for the type bridge. Implement both directions:

```rust
#[cfg(feature = "python")]
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
is the parameter type.

## Make a downstream dialect's classes actually load

The machinery is automatic, but two things must hold:

1. The dialect crate enables `python = ["pliron/python"]` (passthrough feature).
2. The dialect crate is a **dependency of the `pliron-python` assembly crate** so
   it gets linked into the cdylib. `linkme` only collects from linked crates.

As of this commit, `pliron-python/Cargo.toml` depends only on `pliron`, **not** on
`pliron-llvm` — so LLVM ops are not yet in the wheel despite the codegen
supporting them. Adding `pliron-llvm` (with its `python` feature) to the assembly
crate's dependencies is all that is required.

## Known gaps / caveats (as of `bb7425d`)

- **Exposure is all-or-nothing.** No per-item opt-in; the `python` feature
  exposes every derived type/attr/op in every linked dialect. If selective
  exposure is ever wanted, it needs a new macro argument.
- **`pliron-llvm` not linked** into the assembly crate yet (above), so only
  builtin dialect classes are reachable from Python.
- **`__all__` and `.pyi` are hand-maintained and partial.** `python/pliron/__init__.py`
  enumerates the re-exports by hand; `python/pliron/__init__.pyi` only stubs
  `PlironError`, `Context`, and `Operation`. The crate depends on
  `pyo3-stub-gen`, suggesting stub generation is meant to be automated later —
  but the committed stubs are manual and must be kept in sync.
- **`requires-python` mismatch.** `pyproject.toml` says `>=3.12` while the abi3
  floor is `py39`. Harmless but inconsistent.
- **`APInt::from_py` is `unimplemented!`** — integer attribute values round-trip
  out to a string but cannot be reconstructed from one through the generic
  `PyMap` path (the builtin `IntegerAttr` works around this with a hand-written
  constructor that builds the `APInt` from the integer type's width).
- **Dangling handles panic.** A Python handle used after its IR entity is erased
  will panic on the next deref. There is no generation/liveness check.
- **No match/rewrite bridge.** The Python rewrite bindings (`Rewriter`,
  `apply_match_rewrite`) have been removed pending a re-spec, so rewrite patterns
  cannot currently be written in Python.
