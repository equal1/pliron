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
    i64 = pliron.builtin.IntegerType.get(64, "signed")
```

**Constructors.** Uniqued types use `.get(...)`; attributes and ops use
`.new(...)`:

```python
pliron.builtin.IntegerType.get(32)                 # signless
pliron.builtin.IntegerType.get(64, "signed")
pliron.builtin.FunctionType.get([i64], [i64])
pliron.builtin.UnitType.get()
pliron.builtin.StringAttr.new("hello")
pliron.builtin.IntegerAttr.new(42, i64)
pliron.builtin.ModuleOp.new("name")
pliron.builtin.FuncOp.new("foo", func_ty)
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

Builtin: `IntegerType`, `FunctionType`, `UnitType` (also `FP16Type`, `FP32Type`,
`FP64Type` registered).

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

Builtin: `StringAttr`, `IntegerAttr`, `BoolAttr`, `UnitAttr`, `TypeAttr`,
`IdentifierAttr`, `FPHalfAttr`/`FPSingleAttr`/`FPDoubleAttr`, `DictAttr`,
`VecAttr`, … (see `src/dialects/builtin.rs`).

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

Builtin: `ModuleOp`, `FuncOp`, `ForwardRefOp`, `ConstantOp`, `ReturnOp`,
`UnrealizedConversionCastOp`.

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

## pliron.irbuild — IRInserter / IRRewriter

Insertion points are objects:
`OpInsertionPoint.at_block_end(b) / at_block_start(b) / after_operation(op) /
before_operation(op) / unset()` and
`BlockInsertionPoint.at_region_end(r) / at_region_start(r) / after_block(b) /
before_block(b) / unset()`.

`IRInserter(insertion_point=None, listener=None)`: `append_operation(op)`,
`insert_operation(op)`, `insert_block(point, block)`,
`create_block(point, arg_types, label=None)`, `get/set_insertion_point`,
`get_insertion_block()`, `is_modified()`, `listener` / `set_listener(obj)`.

`IRRewriter(insertion_point=None, listener=None)`: everything above, plus
`replace_operation(op, new_op)`, `replace_operation_with_values(op, values)`,
`replace_value_uses_with(old, new)`, `erase_operation/erase_block/erase_region`,
`unlink_operation/unlink_block`, `move_operation(op, point)` /
`move_block(block, point)`, `split_block(block, position, new_block_label=None)`,
`inline_region(src_region, dest_point)`, `set_value_type(value, type)`. The
attached listener (any object with `notify_*` hooks) hears
insertions, erasures, unlinking,
and value replacements.

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
    i64 = pliron.builtin.IntegerType.get(64, "signed")
    func_ty = pliron.builtin.FunctionType.get([], [i64])
    module = pliron.builtin.ModuleOp.new("test_mod")
    func = pliron.builtin.FuncOp.new("foo", func_ty)
    mod_block = module.get_region(0).entry_block()
    func.insert_at_back(mod_block)
    return module, func, i64
```

Create a block via the builder:

```python
with pliron.Context():
    f = pliron.builtin.FuncOp.new("multi_block", pliron.builtin.FunctionType.get([], []))
    region = f.get_region(0)
    i32 = pliron.builtin.IntegerType.get(32)
    ins = pliron.irbuild.IRInserter()
    new_block = ins.create_block(
        pliron.irbuild.BlockInsertionPoint.at_region_end(region), [i32], "second"
    )
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
