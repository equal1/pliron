# 04 — Python API reference

This is the Python surface as it actually behaves, grouped by class. It is
derived from the binding source and from `tests/test_bindings.py`, which is the
best behavioral source of truth. Everything is under the top-level `pliron`
module.

## Idioms

**Context manager.** All IR work happens inside `with pliron.Context():`. The
context is thread-bound and single-active; any IR call outside an active context
raises `pliron.PlironError`.

```python
with pliron.Context():
    i64 = pliron.IntegerType.get(64, "signed")
```

**Constructors.** Uniqued types use `.get(...)`; attributes and ops use
`.new(...)`:

```python
pliron.IntegerType.get(32)                 # signless
pliron.IntegerType.get(64, "signed")
pliron.FunctionType.get([i64], [i64])
pliron.UnitType.get()
pliron.StringAttr.new("hello")
pliron.IntegerAttr.new(42, i64)
pliron.ModuleOp.new("name")
pliron.FuncOp.new("foo", func_ty)
```

**Handle semantics.** All handle types (`Operation`, `BasicBlock`, `Region`,
`Value`, `Type`, `Attribute`) support `str()`, `repr()` (= `str()`), `==`/`!=`,
and `hash()`. Equality is identity/value based: two handles to the same entity
compare and hash equal.

## Context

| Method | Purpose |
|---|---|
| `pliron.Context()` | construct; use as `with` context manager (`as ctx` optional) |
| `is_ir_empty()` | True when the context holds no IR |

## Type / builtin type classes

Builtin: `IntegerType`, `FunctionType`, `UnitType` (also `FP32Type`, `FP64Type`
registered).

| Method | Purpose |
|---|---|
| `IntegerType.get(bits[, signedness])` | uniqued int type; signedness `"signed"`/`"unsigned"`/default signless |
| `FunctionType.get(inputs, results)` | lists of `Type` |
| `UnitType.get()` | the unit type |
| `type_name()` | qualified name, e.g. `"builtin.integer"` |
| `verify()` | structural check |
| `to_type()` / `from_type(t)` | project to a generic `Type` / downcast one back (`None` on mismatch) — see [06](06-type-exposure.md) |
| `get_<field>()` | field accessors (e.g. `get_width`, `get_signedness`) |

Uniquing: equal params → identical object (`==`, equal `hash`). Prints as
`i32` / `si64` / `ui8`.

## Attribute / builtin attribute classes

Builtin: `StringAttr`, `IntegerAttr`, `BoolAttr`, … (see `__init__.py`).

| Method | Purpose |
|---|---|
| `StringAttr.new(s)` | |
| `IntegerAttr.new(value, int_type)` | type must be integer, else `PlironError` |
| `attr_name()` | qualified name |
| `clone_attr()` | independent copy |
| `into_attr()` | coerce typed → generic `Attribute` (used by `set_attribute`) |
| `from_attr(a)` | downcast generic `Attribute` → typed (generated) |
| `get_<field>()` | field accessors |
| `verify()` | |

## Operation / builtin op classes

Builtin: `ModuleOp`, `FuncOp`, `ForwardRefOp`.

| Method | Purpose |
|---|---|
| `ModuleOp.new(name)` / `FuncOp.new(name, func_ty)` | construct (build operands/regions Rust-side) |
| `op_name()` | e.g. `"builtin.module"` |
| `num_results()` / `get_result(i)` / `results()` | SSA results |
| `num_operands()` / `get_operand(i)` / `operands()` | operands |
| `num_regions()` / `get_region(i)` / `regions()` | nested regions |
| `num_successors()` / `get_successor(i)` | CFG successors |
| `get_attribute(name)` / `attribute_names()` / `set_attribute(name, attr)` | attributes |
| `parent_block()` / `next_op()` / `prev_op()` | navigation |
| `insert_at_back(b)` / `insert_at_front(b)` / `insert_after(op)` / `insert_before(op)` | placement |
| `erase()` | remove from block (uses must be gone) |
| `verify()` | |
| `from_operation(op)` / `operation()` | typed ↔ generic projection (generated) |

## Region

`num_blocks()`, `blocks()`, `entry_block()`, `parent_op()`, `index_in_parent()`,
`verify()`.

## BasicBlock

`num_arguments()`, `arguments()`, `get_argument(i)`, `ops()`, `get_terminator()`,
`label()`, `parent_region()`, `parent_op()`, `successors()`, `verify()`.

## Value

`get_type()`, `is_op_result()`, `is_block_argument()`, `defining_op()`,
`num_uses()`, `is_used()`, `users()`, `replace_all_uses_with(other)`, `verify()`.

## IRBuilder

Construct: `IRBuilder.at_block_end(b)`, `at_block_start(b)`, `after_op(op)`,
`before_op(op)`. Re-point: `set_at_block_end`, `set_at_block_start`,
`set_after_op`, `set_before_op`. Insert: `append_op(op)`, `insert_op(op)`. Create:
`create_block_at_region_end(region, arg_types, label=None)`,
`create_block_at_region_start(...)`.

## Rewriter + `apply_match_rewrite`

`pliron.apply_match_rewrite(root_op, obj)` where `obj` has `match(op) -> bool` and
`rewrite(rewriter, op)`. The `rewriter` exposes `erase_op`, `replace_op_with_op`,
`replace_op_with_values`, `erase_block`, `move_op_after`/`before`,
`split_block_before`, `inline_region_at_start`, insertion-point setters, and
`is_modified`. See [03-core-classes.md](03-core-classes.md#the-matchrewrite-bridge).

## Errors

All failures raise `pliron.PlironError`. Tested cases:

| Trigger | Message contains |
|---|---|
| IR call with no active context | `"No active pliron context"` |
| entering a second `Context` while one is active | `"already active"` |
| `IntegerType.get(32, "bogus")` | `"Invalid signedness"` |
| `IntegerAttr.new(1, function_type)` | `"Expected an integer type"` |
| `verify()` on a block with no terminator | `"missing a terminator"` |

## Representative snippets (from the test suite)

Build and navigate a module/func:

```python
def _make_simple_module():
    i64 = pliron.IntegerType.get(64, "signed")
    func_ty = pliron.FunctionType.get([], [i64])
    module = pliron.ModuleOp.new("test_mod")
    func = pliron.FuncOp.new("foo", func_ty)
    mod_block = module.get_region(0).entry_block()
    func.insert_at_back(mod_block)
    return module, func, i64
```

Create a block via the builder:

```python
with pliron.Context():
    f = pliron.FuncOp.new("multi_block", pliron.FunctionType.get([], []))
    region = f.get_region(0)
    i32 = pliron.IntegerType.get(32)
    builder = pliron.IRBuilder.at_block_end(region.entry_block())
    new_block = builder.create_block_at_region_end(region, [i32], "second")
    assert region.num_blocks() == 2
    assert new_block.label() == "second"
    assert new_block.num_arguments() == 1
```

Erase and inspect:

```python
func.erase()
ops = mod_block.ops()
assert len(ops) == 1 and ops[0] == func2
```
