# 03 — Hand-written core classes

These are the fixed IR wrappers in `src/python/`. They are hand-written (not
derive-generated) because they wrap pliron's structural primitives, which are not
themselves dialect `#[pliron_*]` definitions. Every one is `#[pyclass(unsendable)]`
and resolves its data through the thread-local context via `get_ctx()` /
`get_ctx_mut()`.

A shared shape runs through all of them:

- The struct holds a single arena handle (`Ptr<Operation>`, `Ptr<TypeObj>`,
  `Ptr<BasicBlock>`, `Ptr<Region>`) or a small value (`Value`, `AttrObj`).
- Every method opens with `let ctx = super::get_ctx()?;`.
- `__str__`/`__repr__` print via `Printable::disp(ctx)`; `verify()` maps errors
  through `to_py_err`; `__eq__`/`__hash__` use the underlying handle's identity.

## `PyContext` — `src/python/context.rs`

The only stateful one. Owns `Option<Box<Context>>`, is used as a context manager.

- `Context()` — construct (inactive).
- `__enter__` → `set_active_ctx(ptr)`, returns `self`.
- `__exit__` → `clear_active_ctx()`, returns `False` (so exceptions propagate).
- `is_ir_empty()` — true when no ops/blocks/regions exist.

See [01-architecture.md](01-architecture.md#the-context-model) for the lifetime
and single-active-context rules.

## `PyOperation` — `src/python/operation.rs`

`PyOperation { ptr: Ptr<Operation> }`. This is the rich structural-inspection
surface; typed op wrappers (`PyModuleOp`, …) project to/from it.

- **Inspection**: `op_name`, `num_results`/`get_result`/`results`,
  `num_operands`/`get_operand`/`operands`, `num_regions`/`get_region`/`regions`,
  `num_successors`/`get_successor`.
- **Attributes**: `get_attribute(name) -> Option<PyAttribute>`,
  `attribute_names() -> list[str]`, `set_attribute(name, attr)`.
- **Navigation**: `parent_block`, `next_op`, `prev_op`.
- **Printing / verify**: `__str__`, `__repr__`, `verify`.
- **Mutation**: `erase`, `insert_at_front(block)`, `insert_at_back(block)`,
  `insert_after(op)`, `insert_before(op)`.
- **Identity**: `__eq__`/`__hash__` on the `Ptr`.

`set_attribute` is the consumer of the `into_attr()` convention: it accepts either
a generic `PyAttribute` or any typed attribute object, coercing the latter by
calling its `into_attr()` method ([`operation.rs:260`](../../src/python/operation.rs)):

```rust
let attr = if let Ok(attr) = attr.extract::<PyRef<'_, PyAttribute>>() {
    dyn_clone::clone_box(&*attr.inner)
} else {
    let attr = attr.call_method0("into_attr")?;        // typed -> generic
    dyn_clone::clone_box(&*attr.extract::<PyRef<'_, PyAttribute>>()?.inner)
};
```

## `PyType` — `src/python/types.rs`

`PyType { ptr: TypeHandle }` — the generic type face. Types are globally uniqued
and immutable, so `__eq__` is handle identity and `__hash__` uses the type's own
`hash_type()`.

- `type_name`, `__str__`/`__repr__`, `verify`, `__eq__`, `__hash__`.

Concrete typed wrappers (`PyIntegerType`, …) are derive-generated and wrap a
`TypedHandle<T>`; they project to/from `PyType` via `to_type`/`from_type`. The
full type-layer design is in [06-type-exposure.md](06-type-exposure.md).

## `PyAttribute` — `src/python/attributes.rs`

`PyAttribute { inner: AttrObj }`. Attributes are *not* uniqued; each wrapper owns
a boxed clone.

- `attr_name`, `clone_attr`, `__str__`/`__repr__`, `verify`.
- `__eq__`/`__ne__` via `eq_attr`; `__hash__` via `hash_attr`.

## `PyValue` — `src/python/value.rs`

`PyValue { val: Value }` — an SSA value (op result or block argument).

- `get_type`.
- `is_op_result`, `is_block_argument`, `defining_op` (None for block args).
- Use-def: `num_uses`, `is_used`, `users`, `replace_all_uses_with(other)`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `PyBasicBlock` — `src/python/basic_block.rs`

`PyBasicBlock { ptr: Ptr<BasicBlock> }`.

- Args: `num_arguments`, `get_argument`, `arguments`.
- Ops: `ops()` (walks the intrusive list head→next), `get_terminator`.
- Navigation: `label`, `parent_region`, `parent_op`, `successors`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `PyRegion` — `src/python/region.rs`

`PyRegion { ptr: Ptr<Region> }`.

- Blocks: `blocks()`, `entry_block()`, `num_blocks()` (all walk the block list).
- Navigation: `parent_op`, `index_in_parent`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `PyIRBuilder` — `src/python/irbuild/builder.rs`

Wraps an `IRInserter<DummyListener>` — a movable insertion point.

- **Construct**: `at_block_end(block)`, `at_block_start(block)`, `after_op(op)`,
  `before_op(op)` (all `#[staticmethod]`).
- **Re-point**: `set_at_block_end`, `set_at_block_start`, `set_after_op`,
  `set_before_op`.
- **Insert**: `append_op(op)` (advances past the op), `insert_op(op)` (does not).
- **Create blocks**: `create_block_at_region_end(region, arg_types, label=None)`
  and `…_at_region_start(...)`, returning the new `PyBasicBlock`.

## The match/rewrite bridge — `src/python/irbuild/rewriter.rs`

Lets a pattern be written in Python and driven by pliron's Rust rewrite engine.

**`PyRewriter`** (`name = "Rewriter"`) wraps a `*mut MatchRewriter` that is only
valid *during a callback*. `get_rw()` returns `PlironError` if the pointer is
null. It exposes:

- insertion-point setters (`set_at_block_*`, `set_*_op`),
- `erase_op`, `replace_op_with_op`, `replace_op_with_values`, `erase_block`,
- `move_op_after`/`move_op_before`, `split_block_before`,
  `inline_region_at_start`, `is_modified`.

**`PyMatchRewriteAdapter`** implements the Rust `MatchRewrite` trait by calling
back into a Python object that has `match(op) -> bool` and
`rewrite(rewriter, op)` methods:

- `match` calls Python `match`, printing and treating any Python error as
  `False`.
- `rewrite` constructs a fresh `PyRewriter` over the live Rust rewriter, calls
  Python `rewrite`, then **nulls the rewriter pointer** so any later use raises
  cleanly. A Python exception becomes a `PythonCallbackError` propagated as a
  pliron error.

**`apply_match_rewrite(root_op, rewrite_obj)`** (the module's one free function)
wraps `rewrite_obj` in the adapter and runs the core `apply_match_rewrite` over
the IR tree rooted at `root_op`.

```python
class FoldAddZero:
    def match(self, op):              return op.op_name() == "llvm.add"
    def rewrite(self, rewriter, op):  ...   # inspect & rewrite via rewriter
pliron.apply_match_rewrite(root_op, FoldAddZero())
```

> Note: the rewrite bridge is implemented but **not yet exercised by the test
> suite** as of this commit.
