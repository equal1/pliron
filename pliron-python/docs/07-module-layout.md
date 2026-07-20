# 07 — Python module layout

How the Python package is structured and how classes find their module. This is
implemented; the shim, registration routing, and layout rule below are current.

## Layout

```python
import pliron
pliron.Operation, pliron.BasicBlock, pliron.Value      # core: flat on the package
from pliron.irbuild import IRInserter, IRStatus        # folder -> submodule
from pliron.builtin import IntegerType, ModuleOp       # dialect -> submodule
from pliron.llvm import ...                            # downstream dialect, same rule
```

**The rule:**

- Classes from files directly under `pliron-python/src/` (`operation.rs`,
  `basic_block.rs`, `value.rs`, …) live **flat on `pliron`**.
- Each **folder** under `pliron-python/src/` (a Rust module directory) is one
  Python submodule, populated flat by its files. Its `mod.rs` builds it via a
  `pub fn register(parent)` (see `src/irbuild/mod.rs`). Nested folders
  nest submodules. (`src/dialects/` is an exception — it holds the reflect-export
  invocations for the builtin dialect, whose classes are prefix-routed like any
  dialect's, below.)
- Each **dialect** is one submodule (`pliron.builtin`, `pliron.llvm`), holding
  every generated class of that dialect.

`PlironError` stays on the root.

## Dialect routing: the prefix is the module path

There is no static "dialect declaration" item in Rust — a `Dialect` is a runtime
entity (`Dialect::register(ctx, …)`). The dialect name exists statically in
exactly one place: the `"dialect.name"` string in every
`#[pliron_op/attr/type(name = …)]`, which travels through the reflect envelope
and is baked into each generated registration:

```rust
PyClassRegistration { name: "builtin.integer", register: … }
```

`register_all_classes` splits that prefix and routes the class into a
get-or-created `pliron.<dialect>` submodule
([`pliron-python/src/lib.rs`](../src/lib.rs), `get_or_create_submodule`).
Consequences:

- **No convention needed from dialect authors.** The `name = "mydialect.foo"`
  they already write *is* the module placement. Linking the crate that holds the
  generated wrappers is enough (linkme collects registrations across crates).
- Get-or-create makes linkme's arbitrary iteration order irrelevant.
- A registration without a `.` prefix registers at the top level (escape hatch).
- Downstream dialects **cannot** be folders under `pliron-python/src/` (their
  code lives in their own crate), which is why dialects are prefix-routed rather
  than folder-mapped. A dialect crate can still hand-write extras onto its
  module via `py_class_registration!` with its dialect prefix.

## Why `add_submodule` alone is not enough

PyO3's `add_submodule(sub)` is just `setattr(parent, name, sub)` — an
**attribute**. Python's import *statements* don't do attribute lookups; they
resolve dotted paths through the import system, which consults the
`sys.modules` cache before searching the filesystem. A native submodule has no
file, so:

```python
pliron.irbuild.IRInserter              # ✓ attribute chain
from pliron import irbuild             # ✓ (import has an attribute fallback)
import pliron.irbuild                  # ✗ ModuleNotFoundError without sys.modules
from pliron.irbuild import IRInserter  # ✗ same
```

The fix — the standard PyO3 workaround (polars, pydantic-core) — is to add
`sys.modules["pliron.irbuild"] = sub`. Since nothing can `import
pliron.irbuild` before `import pliron` has run, seeding during package import
is always in time.

## Division of labor: Rust builds, the shim publishes

The native extension is `pliron._pliron` — maturin's mixed layout
(`python-source = "python"`, with `module-name = "pliron._pliron"` in
`pyproject.toml`) places the compiled module *inside* the Python package. An
extension `.so` can only ever be loaded *as a module* (CPython's loader calls
its `PyInit_*` function, which must return a module object), so an inner module
is unavoidable; the underscore marks it private, and the layout exists so we
can ship `.pyi` stubs and pure-Python helpers. Rust sees its own `__name__` as
`pliron._pliron` and can't know the public prefix, so it does **not** touch
`sys.modules`. Instead:

- **Rust** builds the submodule *tree* with plain `add_submodule`
  (`register_core_types` → `irbuild::register`; `register_all_classes` →
  dialect routing).
- **The shim** (`python/pliron/__init__.py`) walks the native module once,
  re-exports every top-level name, and registers every submodule (recursively)
  in `sys.modules` under `pliron.<name>`, fixing up `__name__` for clean reprs.
  It is fully generic — nothing is hand-maintained per class or dialect, and
  the old stale-prone `__all__` list is gone.

## Downstream dialects

- **Linked into the pliron wheel** (e.g. an LLVM bindings crate as a dependency
  of `pliron-python`): nothing to do — prefix routing lands its classes in
  `pliron.llvm`.
- **Own wheel** (`mylang` extension embedding pliron): their extension crate
  depends on `pliron-python` as an `rlib` and calls the same
  `pliron_python::register_core_types(m)` / `register_all_classes(m)` on their
  own `#[pymodule]`, and they copy the generic shim. Everything (core, builtin,
  their dialects) appears under `mylang.*`. Best-effort by design: two separate
  cdylibs each embedding pliron would have disjoint arenas/registries, so core
  cannot be split across extension modules.

## Known gaps

- **Stubs**: `__init__.pyi` predates this layout; it should become a stub
  package (`irbuild.pyi`, `builtin.pyi`, …).
- **`__module__` on classes** is not set (`#[pyclass(module = "…")]`), so
  `repr(type(x))` shows `builtins.…`. `pliron-python-derive` knows the dialect
  at codegen time and could emit `module = "pliron.builtin"` etc.; cosmetic,
  deferred.
