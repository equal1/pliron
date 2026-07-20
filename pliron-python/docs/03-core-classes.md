# 03 — Hand-written core classes

These are the fixed IR wrappers directly under `pliron-python/src/`. They are
hand-written (not derive-generated) because they wrap pliron's structural
primitives, which are not themselves dialect `#[pliron_*]` definitions. Every
one is `#[pyclass(unsendable)]` and resolves its data through the thread-local
context via `get_ctx()` / `get_ctx_mut()` (defined in the crate root,
`src/lib.rs`).

A shared shape runs through all of them:

- The struct holds a single arena handle (`Ptr<Operation>`, `Ptr<TypeObj>`,
  `Ptr<BasicBlock>`, `Ptr<Region>`) or a small value (`Value`, `AttrObj`).
- Every method opens with `let ctx = super::get_ctx()?;`.
- `__str__`/`__repr__` print via `Printable::disp(ctx)`; `verify()` maps errors
  through `to_py_err`; `__eq__`/`__hash__` use the underlying handle's identity.

## `PyContext` — `src/context.rs`

The only stateful one. Owns `Option<Box<Context>>`, is used as a context manager.

- `Context()` — construct (inactive).
- `__enter__` → `set_active_ctx(ptr)`, returns `self`.
- `__exit__` → `clear_active_ctx()`, returns `False` (so exceptions propagate).
- `is_ir_empty()` — true when no ops/blocks/regions exist.

See [01-architecture.md](01-architecture.md#the-context-model) for the lifetime
and single-active-context rules.

## `PyOperation` — `src/operation.rs`

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
calling its `into_attr()` method ([`operation.rs:260`](../src/operation.rs)):

```rust
let attr = if let Ok(attr) = attr.extract::<PyRef<'_, PyAttribute>>() {
    dyn_clone::clone_box(&*attr.inner)
} else {
    let attr = attr.call_method0("into_attr")?;        // typed -> generic
    dyn_clone::clone_box(&*attr.extract::<PyRef<'_, PyAttribute>>()?.inner)
};
```

## `PyType` — `src/types.rs`

`PyType { ptr: TypeHandle }` — the generic type face. Types are globally uniqued
and immutable, so `__eq__` is handle identity and `__hash__` uses the type's own
`hash_type()`.

- `type_name`, `__str__`/`__repr__`, `verify`, `__eq__`, `__hash__`.

Concrete typed wrappers (`PyIntegerType`, …) are derive-generated and wrap a
`TypedHandle<T>`; they project to/from `PyType` via `to_type`/`from_type`. The
full type-layer design is in [06-type-exposure.md](06-type-exposure.md).

## `PyAttribute` — `src/attributes.rs`

`PyAttribute { inner: AttrObj }`. Attributes are *not* uniqued; each wrapper owns
a boxed clone.

- `attr_name`, `clone_attr`, `__str__`/`__repr__`, `verify`.
- `__eq__`/`__ne__` via `eq_attr`; `__hash__` via `hash_attr`.

## `PyValue` — `src/value.rs`

`PyValue { val: Value }` — an SSA value (op result or block argument).

- `get_type`.
- `is_op_result`, `is_block_argument`, `defining_op` (None for block args).
- Use-def: `num_uses`, `is_used`, `users`, `replace_all_uses_with(other)`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `PyBasicBlock` — `src/basic_block.rs`

`PyBasicBlock { ptr: Ptr<BasicBlock> }`.

- Args: `num_arguments`, `get_argument`, `arguments`.
- Ops: `ops()` (walks the intrusive list head→next), `get_terminator`.
- Navigation: `label`, `parent_region`, `parent_op`, `successors`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `PyRegion` — `src/region.rs`

`PyRegion { ptr: Ptr<Region> }`.

- Blocks: `blocks()`, `entry_block()`, `num_blocks()` (all walk the block list).
- Navigation: `parent_op`, `index_in_parent`.
- `__str__`/`__repr__`, `verify`, `__eq__`/`__hash__`.

## `pliron.irbuild` — inserter / rewriter / listeners

Mirrors `src/irbuild/*` (folder → submodule). The key classes:

**Insertion points** (`OpInsertionPoint`, `BlockInsertionPoint`) — first-class
objects built by static constructors (`at_block_end(block)`,
`after_operation(op)`, `at_region_end(region)`, `unset()`, …), with `is_set()` /
`get_insertion_block()`/`get_insertion_region()`. Having these as objects keeps
the Python API 1:1 with the Rust traits and lets points round-trip through
Python-defined inserters.

**`IRInserter`** (`PyIRInserter`, wraps `IRInserter<PyInsertionListener>`) —
`IRInserter(insertion_point=None, listener=None)`. Methods: `append_operation`,
`insert_operation`, `insert_block(point, block)`,
`create_block(point, arg_types, label=None)`, `get/set_insertion_point`,
`is_insertion_point_set`, `get_insertion_block`, `is_modified`/`mark_modified`,
`listener` (getter) / `set_listener`.

**`IRRewriter`** (`PyIRRewriter`, wraps `IRRewriter<PyRewriteListener>`) — the
full `Inserter` surface above (shared via a macro, not duplicated) **plus**:
`replace_operation`, `replace_operation_with_values`, `replace_value_uses_with`,
`erase_operation`/`erase_block`/`erase_region`,
`unlink_operation`/`unlink_block`, `move_operation`/`move_block`,
`split_block(block, position, new_block_label=None)`,
`inline_region(src_region, dest_point)`, `set_value_type(value, type)`.
An attached rewrite listener hears every event (erasures, unlinking, value
replacement, …).

**Listeners are duck-typed.** A listener is *any* Python object implementing
(a subset of) the listener protocol — the `notify_*` hooks mirroring the Rust
`InsertionListener`/`RewriteListener` traits. No base class is required; hooks
the object does not define are skipped, and a raising hook is printed and
swallowed (a notification must not abort an in-flight IR mutation). Express the
protocol with `typing.Protocol` on the Python side if static checking is wanted.

Rust-side, four hidden dispatch structs (plain structs holding `Py<PyAny>`,
*not* pyclasses) implement the Rust traits by calling the same-named method on
the wrapped Python object:

- `PyInsertionListener` / `PyRewriteListener` — the `L` plugged into
  `IRInserter<L>`/`IRRewriter<L>` at intake (`IRInserter(listener=obj)`,
  `clone_*(…, listener=obj)`). Dispatch is defensive: undefined hooks are
  skipped, raising hooks are swallowed.
- `PyInserter` / `PyRewriter` — a Python object as a Rust `Inserter`/`Rewriter`
  for Rust entry points that take one. Dispatch is **hard-fail** (missing or
  raising methods panic → Python exception): these are required effects, not
  optional notifications. No entry point consumes them yet.

`ScopedInserter`/`ScopedRewriter` are not exposed (drop-based scoping doesn't
map to Python).

**Native listeners**: `DummyListener` (no-op) and `Recorder` (records events;
`len()`, `events()`, `clear()`, `is_empty()`), both valid `listener=` values.

## `PyIRStatus` — `src/irbuild/mod.rs`

Python binding for `IRStatus`, exposed as `pliron.irbuild.IRStatus` with `Unchanged` /
`Changed` variants. It is truthy when the IR was changed, so it reads either as
an enum comparison or a plain condition.

> **Note:** the Python match/rewrite bridge has been removed pending a re-spec —
> there is no `apply_match_rewrite` function. (The `pliron.irbuild.Rewriter`
> class above is the *trait adapter*, not the old match/rewrite bridge.)
