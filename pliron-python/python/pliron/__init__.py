# pliron Python bindings.
#
# The implementation lives in the native Rust extension (`pliron/_pliron.abi3.so`,
# importable as `pliron._pliron` — maturin's mixed Rust/Python layout places the
# compiled module inside this package; the underscore marks it private). This
# shim publishes the native module's contents under the public `pliron` package:
#
#   - every top-level class/function is re-exported here (`pliron.Operation`), and
#   - every native submodule (`irbuild`, dialect modules like `builtin`,
#     downstream dialects, ...) is registered in `sys.modules` so
#     `import pliron.irbuild` and `from pliron.builtin import IntegerType` work.
#     (PyO3's `add_submodule` only creates an attribute; Python's import
#     statements resolve dotted paths through `sys.modules`.)
#
# Nothing here is hand-maintained per class or per dialect: whatever the native
# module registers is published automatically.

import sys as _sys
from types import ModuleType as _ModuleType

from . import _pliron as _native


def _publish(module, public_name):
    """Make `module` importable as `public_name`, recursing into submodules."""
    _sys.modules[public_name] = module  # noqa: F821
    module.__name__ = public_name
    for _attr in dir(module):
        if _attr.startswith("_"):
            continue
        _value = getattr(module, _attr)
        if isinstance(_value, _ModuleType):  # noqa: F821
            _publish(_value, f"{public_name}.{_attr}")  # noqa: F821


for _attr in dir(_native):
    if _attr.startswith("_"):
        continue
    _value = getattr(_native, _attr)
    globals()[_attr] = _value
    if isinstance(_value, _ModuleType):
        _publish(_value, f"{__name__}.{_attr}")

del _sys, _ModuleType, _native, _publish, _attr, _value
