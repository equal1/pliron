"""Comprehensive tests for the pliron Python bindings.

These tests cover:
 - Context lifecycle (context manager, single-active enforcement)
 - Builtin type classes (IntegerType, FunctionType, UnitType)
 - Builtin attribute classes (StringAttr, IntegerAttr)
 - Builtin op classes (ModuleOp, FuncOp)
 - Operation accessors (op_name, results, operands, regions, attributes, navigation)
 - BasicBlock accessors (arguments, ops, terminator, label, navigation)
 - Region accessors (blocks, entry_block, parent_op)
 - Value accessors (type, variant queries, use-def, printing)
 - Type / Attribute introspection and printing
 - IRInserter / IRRewriter (insertion points, op insertion, block creation, rewrites)
 - Structural verification
 - __str__ / __repr__ / __eq__ / __hash__ on all handle types
"""

import pytest

import pliron


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_simple_module():
    """Build: module @test_mod { func @foo : () -> (si64) { entry(): } }"""
    i64 = pliron.builtin.IntegerType.get(64, "signed")
    func_ty = pliron.builtin.FunctionType.get([], [i64])
    module = pliron.builtin.ModuleOp.new("test_mod").operation()
    func = pliron.builtin.FuncOp.new("foo", func_ty).operation()
    # Insert func into module's region's single block
    mod_region = module.get_region(0)
    mod_block = mod_region.entry_block()
    func.insert_at_back(mod_block)
    return module, func, i64


# ===================================================================
#  Context lifecycle
# ===================================================================


class TestContext:
    def test_context_manager_basic(self):
        """Context can be created and entered/exited."""
        with pliron.Context():
            pass  # no error

    def test_context_is_ir_empty(self):
        """A fresh context has empty IR."""
        with pliron.Context() as ctx:
            assert ctx.is_ir_empty()

    def test_no_context_raises(self):
        """Calling API outside context manager raises PlironError."""
        with pytest.raises(pliron.PlironError, match="No active pliron context"):
            pliron.builtin.IntegerType.get(32)

    def test_nested_context_raises(self):
        """Only one context per thread is allowed."""
        with pliron.Context():
            with pytest.raises(pliron.PlironError, match="already active"):
                with pliron.Context():
                    pass

    def test_context_reusable_sequentially(self):
        """Two sequential contexts work fine."""
        with pliron.Context():
            pliron.builtin.IntegerType.get(32)
        with pliron.Context():
            pliron.builtin.IntegerType.get(64)


# ===================================================================
#  Builtin types
# ===================================================================


class TestTypes:
    def test_integer_type_default_signless(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(32)
            assert "i32" in str(ty)

    def test_integer_type_signed(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(64, "signed")
            assert "si64" in str(ty)

    def test_integer_type_unsigned(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(8, "unsigned")
            assert "ui8" in str(ty)

    def test_integer_type_invalid_signedness(self):
        with pliron.Context():
            with pytest.raises(pliron.PlironError, match="Invalid signedness"):
                pliron.builtin.IntegerType.get(32, "bogus")

    def test_function_type(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            i64 = pliron.builtin.IntegerType.get(64, "signed")
            fty = pliron.builtin.FunctionType.get([i32], [i64])
            s = str(fty)
            assert "i32" in s
            assert "si64" in s

    def test_unit_type(self):
        with pliron.Context():
            u = pliron.builtin.UnitType.get()
            assert "unit" in str(u).lower()

    def test_type_equality_and_hash(self):
        """Same type parameters → same uniqued type."""
        with pliron.Context():
            a = pliron.builtin.IntegerType.get(32, "signed")
            b = pliron.builtin.IntegerType.get(32, "signed")
            assert a == b
            assert hash(a) == hash(b)

    def test_type_inequality(self):
        with pliron.Context():
            a = pliron.builtin.IntegerType.get(32)
            b = pliron.builtin.IntegerType.get(64)
            assert a != b

    def test_type_name(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(16)
            name = ty.to_type().type_name()
            assert "builtin" in name
            assert "integer" in name

    def test_type_verify(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(32)
            ty.to_type().verify()  # should not raise


# ===================================================================
#  Builtin attributes
# ===================================================================


class TestAttributes:
    def test_string_attr(self):
        with pliron.Context():
            a = pliron.builtin.StringAttr.new("hello")
            assert "hello" in str(a)

    def test_string_attr_name(self):
        with pliron.Context():
            a = pliron.builtin.StringAttr.new("test")
            name = a.attr_name()
            assert "builtin" in name
            assert "string" in name

    def test_integer_attr(self):
        with pliron.Context():
            i64 = pliron.builtin.IntegerType.get(64, "signed")
            a = pliron.builtin.IntegerAttr.new(42, i64)
            s = str(a)
            assert "42" in s

    def test_integer_attr_name(self):
        with pliron.Context():
            i64 = pliron.builtin.IntegerType.get(64, "signed")
            a = pliron.builtin.IntegerAttr.new(0, i64)
            assert "integer" in a.attr_name()

    def test_integer_attr_wrong_type(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            with pytest.raises(pliron.PlironError, match="Expected an integer type"):
                pliron.builtin.IntegerAttr.new(1, fty)

    def test_attr_clone(self):
        with pliron.Context():
            a = pliron.builtin.StringAttr.new("clone_me")
            b = a.clone_attr()
            assert str(a) == str(b)

    def test_attr_verify(self):
        with pliron.Context():
            i64 = pliron.builtin.IntegerType.get(64, "signed")
            a = pliron.builtin.IntegerAttr.new(99, i64)
            a.verify()


# ===================================================================
#  Module and function ops
# ===================================================================


class TestOps:
    def test_module_op_name(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("my_mod").operation()
            assert m.op_name() == "builtin.module"

    def test_module_has_one_region(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("m").operation()
            assert m.num_regions() == 1

    def test_module_region_has_one_block(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("m").operation()
            r = m.get_region(0)
            assert r.num_blocks == 1

    def test_func_op_name(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("my_fn", fty).operation()
            assert f.op_name() == "builtin.func"

    def test_func_has_region_and_entry_block(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("add", fty).operation()
            assert f.num_regions() == 1
            r = f.get_region(0)
            assert r.num_blocks == 1
            entry = r.entry_block()
            assert entry is not None

    def test_func_entry_block_arguments_match_type(self):
        """FuncOp's entry block should have arguments matching the input types."""
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            i64 = pliron.builtin.IntegerType.get(64, "signed")
            fty = pliron.builtin.FunctionType.get([i32, i64], [])
            f = pliron.builtin.FuncOp.new("multi_arg", fty).operation()
            entry = f.get_region(0).entry_block()
            assert entry.num_arguments == 2
            args = entry.arguments
            assert len(args) == 2
            # Types match
            assert args[0].get_type() == i32.to_type()
            assert args[1].get_type() == i64.to_type()

    def test_func_entry_block_label(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("labeled", fty).operation()
            entry = f.get_region(0).entry_block()
            assert entry.label == "entry"

    def test_module_func_structure(self):
        """Build module { func { entry() } } and navigate the structure."""
        with pliron.Context():
            module, func, i64 = _make_simple_module()

            # Module region/block structure
            mod_region = module.get_region(0)
            mod_block = mod_region.entry_block()
            ops = list(mod_block.ops())
            assert len(ops) == 1
            assert ops[0] == func

            # Func region/block
            func_region = func.get_region(0)
            entry = func_region.entry_block()
            assert entry is not None
            assert entry.num_arguments == 0

    def test_no_results_no_operands(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("x").operation()
            assert m.num_results() == 0
            assert m.num_operands() == 0

    def test_op_printing(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("printable").operation()
            s = str(m)
            assert "printable" in s
            assert "builtin.module" in s

    def test_op_repr(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("r").operation()
            assert repr(m) == str(m)

    def test_op_verify_valid(self):
        """verify() raises when blocks lack terminators."""
        with pliron.Context():
            module, _, _ = _make_simple_module()
            # FuncOp's entry block has no terminator, so verification should fail.
            with pytest.raises(pliron.PlironError, match="missing a terminator"):
                module.verify()

    def test_op_equality_and_hash(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("eq_test").operation()
            ops = list(m.get_region(0).entry_block().ops())
            # Module has zero ops in its single block initially
            # But comparing m with itself should be true
            assert m == m  # noqa: PLR0124
            assert hash(m) == hash(m)

    def test_op_inequality(self):
        with pliron.Context():
            m1 = pliron.builtin.ModuleOp.new("a").operation()
            m2 = pliron.builtin.ModuleOp.new("b").operation()
            assert m1 != m2


# ===================================================================
#  Operation insertion and navigation
# ===================================================================


class TestOperationPlacement:
    def test_insert_at_back(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            # Create a second func, insert at back of module's block
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("bar", fty2).operation()
            mod_block = module.get_region(0).entry_block()
            func2.insert_at_back(mod_block)
            ops = list(mod_block.ops())
            assert len(ops) == 2
            assert ops[0] == func
            assert ops[1] == func2

    def test_insert_at_front(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("bar", fty2).operation()
            mod_block = module.get_region(0).entry_block()
            func2.insert_at_front(mod_block)
            ops = list(mod_block.ops())
            assert len(ops) == 2
            assert ops[0] == func2
            assert ops[1] == func

    def test_insert_after(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("after_this", fty2).operation()
            mod_block = module.get_region(0).entry_block()
            func2.insert_after(func)
            ops = list(mod_block.ops())
            assert len(ops) == 2
            assert ops[1] == func2

    def test_insert_before(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("before_this", fty2).operation()
            func2.insert_before(func)
            mod_block = module.get_region(0).entry_block()
            ops = list(mod_block.ops())
            assert len(ops) == 2
            assert ops[0] == func2
            assert ops[1] == func

    def test_next_prev_op(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("second", fty2).operation()
            mod_block = module.get_region(0).entry_block()
            func2.insert_at_back(mod_block)

            assert func.next_op() == func2
            assert func2.prev_op() == func
            assert func.prev_op() is None
            assert func2.next_op() is None

    def test_parent_block(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            assert func.parent_block() == mod_block


# ===================================================================
#  BasicBlock accessors
# ===================================================================


class TestBasicBlock:
    def test_block_arguments(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("with_args", fty).operation()
            entry = f.get_region(0).entry_block()
            assert entry.num_arguments == 1
            arg = entry.get_argument(0)
            assert arg.is_block_argument()
            assert not arg.is_op_result()
            assert arg.get_type() == i32.to_type()

    def test_block_parent_region(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            region = module.get_region(0)
            block = region.entry_block()
            assert block.parent_region == region

    def test_block_parent_op(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            block = module.get_region(0).entry_block()
            assert block.parent_op == module

    def test_block_str(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("pr", fty).operation()
            entry = f.get_region(0).entry_block()
            s = str(entry)
            assert isinstance(s, str)
            assert len(s) > 0

    def test_block_ops_iterable(self):
        """`block.ops()` is a lazy iterator equivalent to the `get_ops()` list."""
        with pliron.Context():
            module, _, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            fty = pliron.builtin.FunctionType.get([], [])
            for name in ("a", "b"):
                pliron.builtin.FuncOp.new(name, fty).operation().insert_at_back(mod_block)

            snapshot = [op.op_name() for op in mod_block.ops()]
            assert [op.op_name() for op in mod_block.ops()] == snapshot  # for-loop
            assert len(list(mod_block.ops())) == len(snapshot)           # list()
            # each call yields a fresh, independent iterator
            it = mod_block.ops()
            assert next(it) == list(mod_block.ops())[0]

    def test_block_equality(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("eq", fty).operation()
            entry = f.get_region(0).entry_block()
            same = f.get_region(0).entry_block()
            assert entry == same
            assert hash(entry) == hash(same)


# ===================================================================
#  Region accessors
# ===================================================================


class TestRegion:
    def test_region_blocks(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            region = module.get_region(0)
            blocks = list(region.blocks())
            assert len(blocks) == 1
            assert blocks[0] == region.entry_block()

    def test_region_parent_op(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            region = module.get_region(0)
            assert region.parent_op == module

    def test_region_index_in_parent(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            region = module.get_region(0)
            assert region.index_in_parent == 0

    def test_region_str(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            region = module.get_region(0)
            s = str(region)
            assert isinstance(s, str)

    def test_region_equality(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            r1 = module.get_region(0)
            r2 = module.get_region(0)
            assert r1 == r2
            assert hash(r1) == hash(r2)


# ===================================================================
#  Value accessors
# ===================================================================


class TestValue:
    def test_block_arg_value(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("val_test", fty).operation()
            entry = f.get_region(0).entry_block()
            arg = entry.get_argument(0)

            assert arg.is_block_argument()
            assert not arg.is_op_result()
            assert arg.defining_op() is None
            assert arg.get_type() == i32.to_type()

    def test_value_str(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("vstr", fty).operation()
            entry = f.get_region(0).entry_block()
            arg = entry.get_argument(0)
            s = str(arg)
            assert isinstance(s, str)
            assert len(s) > 0

    def test_value_equality(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("veq", fty).operation()
            entry = f.get_region(0).entry_block()
            a = entry.get_argument(0)
            b = entry.get_argument(0)
            assert a == b
            assert hash(a) == hash(b)

    def test_value_not_used(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("unused_arg", fty).operation()
            entry = f.get_region(0).entry_block()
            arg = entry.get_argument(0)
            assert arg.num_uses() == 0
            assert not arg.is_used()
            assert arg.users() == []


# ===================================================================
#  Operation attributes
# ===================================================================


class TestOperationAttributes:
    def test_set_and_get_attribute(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            attr = pliron.builtin.StringAttr.new("my_value")
            func.set_attribute("my_key", attr)
            retrieved = func.get_attribute("my_key")
            assert retrieved is not None
            assert "my_value" in str(retrieved)

            assert func.has_attribute("my_key")
            assert not func.has_attribute("Foo_Bar")
            assert func.has_attribute("func_type")

    def test_attribute_names(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            names = func.attribute_names()
            # FuncOp should have at least sym_name and func_type attributes
            assert isinstance(names, list)
            assert len(names) >= 1

    def test_get_nonexistent_attribute(self):
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("attr_test").operation()
            result = m.get_attribute("does_not_exist")
            assert result is None

    def test_integer_attr_on_op(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            i32 = pliron.builtin.IntegerType.get(32)
            attr = pliron.builtin.IntegerAttr.new(100, i32)
            func.set_attribute("my_int", attr)
            retrieved = func.get_attribute("my_int")
            assert retrieved is not None
            assert "100" in str(retrieved)
            assert "integer" in retrieved.attr_name()


# ===================================================================
#  IRInserter
# ===================================================================


class TestIRInserter:
    def test_insert_at_block_end(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("built", fty2).operation()
            point = pliron.irbuild.OpInsertionPoint.at_block_end(mod_block)
            ins = pliron.irbuild.IRInserter(point)
            ins.append_operation(func2)
            ops = list(mod_block.ops())
            assert len(ops) == 2
            assert ops[1] == func2
            assert ins.is_modified()

    def test_insert_at_block_start(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("first", fty2).operation()
            ins = pliron.irbuild.IRInserter(
                pliron.irbuild.OpInsertionPoint.at_block_start(mod_block)
            )
            ins.append_operation(func2)
            ops = list(mod_block.ops())
            assert ops[0] == func2

    def test_insert_after_operation(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("after", fty2).operation()
            ins = pliron.irbuild.IRInserter(
                pliron.irbuild.OpInsertionPoint.after_operation(func)
            )
            ins.append_operation(func2)
            ops = list(mod_block.ops())
            assert ops[1] == func2

    def test_create_block(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("multi_block", fty).operation()
            region = f.get_region(0)
            assert region.num_blocks == 1

            i32 = pliron.builtin.IntegerType.get(32)
            ins = pliron.irbuild.IRInserter()
            new_block = ins.create_block(
                pliron.irbuild.BlockInsertionPoint.at_region_end(region), [i32], "second"
            )
            assert region.num_blocks == 2
            assert new_block.label == "second"
            assert new_block.num_arguments == 1
            # create_block moves the op insertion point into the new block
            assert ins.get_insertion_block() == new_block

    def test_set_insertion_point(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            ins = pliron.irbuild.IRInserter(
                pliron.irbuild.OpInsertionPoint.at_block_start(mod_block)
            )
            # Change to end
            ins.set_insertion_point(pliron.irbuild.OpInsertionPoint.at_block_end(mod_block))
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("moved", fty2).operation()
            ins.append_operation(func2)
            ops = list(mod_block.ops())
            assert ops[-1] == func2

    def test_insertion_point_object(self):
        with pliron.Context():
            module, _, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            unset = pliron.irbuild.OpInsertionPoint.unset()
            assert not unset.is_set()
            point = pliron.irbuild.OpInsertionPoint.at_block_end(mod_block)
            assert point.is_set()
            assert point.get_insertion_block() == mod_block

    def test_inserter_notifies_listener(self):
        class Recorder:
            def __init__(self):
                self.ops = []

            def notify_operation_inserted(self, op):
                self.ops.append(op)

        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            rec = Recorder()
            ins = pliron.irbuild.IRInserter(
                pliron.irbuild.OpInsertionPoint.at_block_end(mod_block), listener=rec
            )
            assert ins.listener is rec
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("noted", fty2).operation()
            ins.append_operation(func2)
            assert rec.ops == [func2]


# ===================================================================
#  IRRewriter
# ===================================================================


class TestIRRewriter:
    def test_erase_operation(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            rw = pliron.irbuild.IRRewriter()
            rw.erase_operation(func)
            assert len(list(mod_block.ops())) == 0
            assert rw.is_modified()

    def test_rewriter_is_an_inserter(self):
        """IRRewriter exposes the full Inserter surface too."""
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            rw = pliron.irbuild.IRRewriter(
                pliron.irbuild.OpInsertionPoint.at_block_end(mod_block)
            )
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("via_rw", fty2).operation()
            rw.append_operation(func2)
            assert list(mod_block.ops())[-1] == func2

    def test_unlink_and_move(self):
        with pliron.Context():
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("second", fty2).operation()
            func2.insert_at_back(mod_block)
            rw = pliron.irbuild.IRRewriter()
            # Move func2 to the front
            rw.move_operation(func2, pliron.irbuild.OpInsertionPoint.at_block_start(mod_block))
            assert list(mod_block.ops())[0] == func2

    def test_rewrite_listener_hooks(self):
        """A rewrite listener hears erasure/unlinking events from IRRewriter."""

        class Events:
            def __init__(self):
                self.erased = []
                self.unlinked = []

            def notify_operation_erasure(self, op):
                self.erased.append(op)

            def notify_operation_unlinking(self, op):
                self.unlinked.append(op)

        with pliron.Context():
            module, func, _ = _make_simple_module()
            ev = Events()
            rw = pliron.irbuild.IRRewriter(listener=ev)
            rw.unlink_operation(func)
            assert ev.unlinked == [func]
            rw.erase_operation(func)
            assert ev.erased == [func]


# ===================================================================
#  Operation erase
# ===================================================================


class TestOperationErase:
    def test_erase_op(self):
        """Erasing an operation removes it from its block."""
        with pliron.Context() as ctx:
            module, func, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()

            # Add second func, erase the first
            fty2 = pliron.builtin.FunctionType.get([], [])
            func2 = pliron.builtin.FuncOp.new("survivor", fty2).operation()
            func2.insert_at_back(mod_block)
            assert len(list(mod_block.ops())) == 2

            func.erase()
            ops = list(mod_block.ops())
            assert len(ops) == 1
            assert ops[0] == func2


# ===================================================================
#  Printing round-trip
# ===================================================================


class TestPrinting:
    def test_module_with_func_prints(self):
        """Printing a module with a function inside produces structured output."""
        with pliron.Context():
            module, _, _ = _make_simple_module()
            printed = str(module)
            assert "builtin.module" in printed
            assert "test_mod" in printed
            assert "builtin.func" in printed
            assert "foo" in printed

    def test_type_prints(self):
        with pliron.Context():
            ty = pliron.builtin.IntegerType.get(64, "signed")
            assert "si64" in str(ty)

    def test_attr_prints(self):
        with pliron.Context():
            a = pliron.builtin.StringAttr.new("world")
            assert "world" in str(a)

    def test_value_repr_eq_str(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("repr_eq", fty).operation()
            val = f.get_region(0).entry_block().get_argument(0)
            assert repr(val) == str(val)


# ===================================================================
#  Verification
# ===================================================================


class TestVerification:
    def test_module_verifies_missing_terminator(self):
        """Module with a func whose entry block has no terminator fails verification."""
        with pliron.Context():
            module, _, _ = _make_simple_module()
            with pytest.raises(pliron.PlironError, match="missing a terminator"):
                module.verify()

    def test_standalone_func_verifies_missing_terminator(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("standalone", fty).operation()
            with pytest.raises(pliron.PlironError, match="missing a terminator"):
                f.verify()

    def test_region_verifies_missing_terminator(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("rv", fty).operation()
            with pytest.raises(pliron.PlironError, match="missing a terminator"):
                f.get_region(0).verify()

    def test_block_verifies_missing_terminator(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("bv", fty).operation()
            with pytest.raises(pliron.PlironError, match="missing a terminator"):
                f.get_region(0).entry_block().verify()

    def test_value_verifies(self):
        with pliron.Context():
            i32 = pliron.builtin.IntegerType.get(32)
            fty = pliron.builtin.FunctionType.get([i32], [])
            f = pliron.builtin.FuncOp.new("vv", fty).operation()
            f.get_region(0).entry_block().get_argument(0).verify()


# ===================================================================
#  IR emptiness after erase
# ===================================================================


class TestIREmptiness:
    def test_context_not_empty_after_creating_ir(self):
        with pliron.Context() as ctx:
            pliron.builtin.ModuleOp.new("m").operation()
            assert not ctx.is_ir_empty()

    def test_context_empty_after_erase(self):
        with pliron.Context() as ctx:
            m = pliron.builtin.ModuleOp.new("m").operation()
            m.erase()
            assert ctx.is_ir_empty()


# ===================================================================
#  Module op with no func (simpler verification)
# ===================================================================


class TestModuleOnly:
    def test_module_only_verifies(self):
        """A module with an empty block verifies successfully."""
        with pliron.Context():
            m = pliron.builtin.ModuleOp.new("simple").operation()
            m.verify()


# ===================================================================
#  Cloning (pliron.irbuild.clone_*)
# ===================================================================


class TestCloning:
    def test_clone_operation_unlinked(self):
        """clone_operation returns an unlinked, structurally-equal clone."""
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            clone = pliron.irbuild.clone_operation(f)
            assert clone.parent_block() is None
            assert clone != f
            assert clone.op_name() == f.op_name()

    def test_clone_operation_insertable(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            m = pliron.builtin.ModuleOp.new("m").operation()
            mblk = m.get_region(0).entry_block()
            clone = pliron.irbuild.clone_operation(f)
            clone.insert_at_back(mblk)
            assert clone.parent_block() == mblk

    def test_clone_operation_records_mapping(self):
        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            mapping = pliron.irbuild.IrMapping()
            clone = pliron.irbuild.clone_operation(f, mapping)
            assert mapping.lookup_op(f) == clone
            assert mapping.lookup_op(clone) is None


# ===================================================================
#  Listeners (duck-typed protocol: any object with notify_* hooks)
# ===================================================================


class TestListener:
    def test_listener_notified_on_clone(self):
        """A (duck-typed) listener is notified of blocks/ops created while cloning."""

        class Recorder:
            def __init__(self):
                self.blocks = 0
                self.ops = 0

            def notify_block_inserted(self, block):
                self.blocks += 1

            def notify_operation_inserted(self, op):
                self.ops += 1

        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            rec = Recorder()
            pliron.irbuild.clone_operation(f, listener=rec)
            # FuncOp has a region with one entry block, which gets cloned.
            assert rec.blocks >= 1

    def test_partial_protocol_ok(self):
        """Hooks the listener object does not define are skipped."""

        class OnlyOps:
            def __init__(self):
                self.ops = 0

            def notify_operation_inserted(self, op):
                self.ops += 1
            # no other notify_* hooks defined

        with pliron.Context():
            module, _, _ = _make_simple_module()
            mod_block = module.get_region(0).entry_block()
            l = OnlyOps()
            rw = pliron.irbuild.IRRewriter(
                pliron.irbuild.OpInsertionPoint.at_block_end(mod_block), listener=l
            )
            fty = pliron.builtin.FunctionType.get([], [])
            f2 = pliron.builtin.FuncOp.new("noted", fty).operation()
            rw.append_operation(f2)
            assert l.ops == 1
            rw.erase_operation(f2)  # fires erasure hooks; undefined -> skipped
            assert l.ops == 1

    def test_native_recorder_records(self):
        """pliron.irbuild.Recorder is a native listener; it records clone events."""
        from pliron.irbuild import Recorder

        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            rec = Recorder()
            assert len(rec) == 0 and rec.is_empty()
            pliron.irbuild.clone_operation(f, listener=rec)
            assert len(rec) >= 1
            assert len(rec.events()) == len(rec)
            rec.clear()
            assert len(rec) == 0

    def test_native_dummy_listener(self):
        """pliron.irbuild.DummyListener is a usable no-op listener."""
        from pliron.irbuild import DummyListener

        with pliron.Context():
            fty = pliron.builtin.FunctionType.get([], [])
            f = pliron.builtin.FuncOp.new("orig", fty).operation()
            clone = pliron.irbuild.clone_operation(f, listener=DummyListener())
            assert clone.op_name() == "builtin.func"

