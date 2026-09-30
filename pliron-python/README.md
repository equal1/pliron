# pliron-python

Python bindings for the [pliron](https://github.com/vaivaswatha/pliron) compiler
IR framework, built with [PyO3](https://pyo3.rs) and
[maturin](https://www.maturin.rs).

## Prerequisites

* **Rust toolchain** – install via [rustup](https://rustup.rs) (stable is fine).
* **Python ≥ 3.9** – the extension uses the Python stable ABI (`abi3-py39`).
* A Python **virtual environment** (see below).

## Quick start with uv

[uv](https://docs.astral.sh/uv/) is the recommended way to manage the virtual
environment and install the package.

```bash
# From the repository root
cd pliron-python

# Create & activate a venv (if you haven't already)
uv venv
source .venv/bin/activate        # Linux / macOS
# .venv\Scripts\activate         # Windows

# Install the package in editable/dev mode (debug build by default)
uv pip install -e .
```

Under the hood `uv pip install -e .` invokes the maturin build backend declared
in `pyproject.toml`, which compiles the Rust code and installs the resulting
wheel into the active virtual environment.

### Using pip / maturin directly

If you prefer plain pip:

```bash
cd pliron-python
python -m venv .venv && source .venv/bin/activate

pip install maturin
maturin develop          # debug build, installed into the active venv
```

## Debug vs Release builds

By default both `uv pip install -e .` and `maturin develop` produce a **debug**
(unoptimised) build of the Rust code, which compiles faster and includes debug
symbols.

### Release build

For optimised native code, pass `--release`:

```bash
# With maturin directly
maturin develop --release

# Or build a release wheel and install it
maturin build --release
uv pip install target/wheels/pliron-*.whl
# (or: pip install target/wheels/pliron-*.whl)
```

To get the same effect through `uv pip install`, set the
`MATURIN_PEP517_ARGS` environment variable:

```bash
MATURIN_PEP517_ARGS="--release" uv pip install -e .
```

### Debug build (explicit)

A debug build is the default, but you can be explicit:

```bash
maturin develop            # already debug
# equivalent to:
maturin develop --profile dev
```

### Custom Cargo profiles

Any Cargo profile can be forwarded to `maturin develop`:

```bash
maturin develop --profile profiling   # if you have such a profile in Cargo.toml
```

## Running the tests

```bash
# Inside the activated venv
uv pip install pytest      # or: pip install pytest
pytest tests/
```

## Project layout

```
pliron-python/
├── Cargo.toml              # Rust bindings crate (rlib + cdylib, PyO3 extension)
├── pyproject.toml           # Python package metadata (maturin backend)
├── src/
│   ├── lib.rs               # PyO3 module entry point, context/error/registration machinery
│   ├── context.rs …         # hand-written core IR wrappers (Operation, BasicBlock, …)
│   ├── py_map.rs            # Rust↔Python type bridge (PyMap)
│   ├── irbuild/             # inserter/rewriter bindings (pliron.irbuild)
│   └── dialects/
│       └── builtin.rs       # builtin-dialect wrappers, generated via pliron-python-derive
├── python/
│   └── pliron/
│       ├── __init__.py       # Re-exports from the native extension
│       └── __init__.pyi      # Type stubs for IDE support
└── tests/
    └── test_bindings.py      # pytest suite
```

The Python codegen macros live in the sibling
[`pliron-python-derive`](../pliron-python-derive) crate (re-exported here as
`pliron_python::derive`); the core `pliron` crate itself is entirely
Python-free.
