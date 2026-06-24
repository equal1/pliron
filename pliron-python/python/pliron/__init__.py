# pliron Python bindings
#
# The actual implementation lives in the native Rust extension (`pliron.so`).
# This file re-exports everything from that extension so IDE tools can see the
# package structure correctly.

from pliron.pliron import *  # noqa: F401, F403

# Core IR wrapper types
from pliron.pliron import PlironError, Context, Operation, BasicBlock, Region  # noqa: F401
from pliron.pliron import Value, Type, Attribute, IRBuilder, Rewriter  # noqa: F401
from pliron.pliron import apply_match_rewrite  # noqa: F401

# Auto-generated builtin dialect classes (types, ops, attributes)
from pliron.pliron import IntegerType, FunctionType, UnitType  # noqa: F401
from pliron.pliron import FP32Type, FP64Type  # noqa: F401
from pliron.pliron import ModuleOp, FuncOp, ForwardRefOp  # noqa: F401
from pliron.pliron import (  # noqa: F401
    StringAttr, IntegerAttr, BoolAttr, IdentifierAttr,
    TypeAttr, UnitAttr, DictAttr, VecAttr,
    FPSingleAttr, FPDoubleAttr, OperandSegmentSizesAttr,
)

__all__ = [
    # Core IR wrapper types
    "PlironError",
    "Context",
    "Operation",
    "BasicBlock",
    "Region",
    "Value",
    "Type",
    "Attribute",
    "IRBuilder",
    "Rewriter",
    "apply_match_rewrite",
    # Builtin types
    "IntegerType",
    "FunctionType",
    "UnitType",
    "FP32Type",
    "FP64Type",
    # Builtin ops
    "ModuleOp",
    "FuncOp",
    "ForwardRefOp",
    # Builtin attributes
    "StringAttr",
    "IntegerAttr",
    "BoolAttr",
    "IdentifierAttr",
    "TypeAttr",
    "UnitAttr",
    "DictAttr",
    "VecAttr",
    "FPSingleAttr",
    "FPDoubleAttr",
    "OperandSegmentSizesAttr",
]
