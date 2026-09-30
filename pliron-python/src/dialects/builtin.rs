//! Python bindings for pliron's `builtin` dialect.
//!
//! The wrapper classes (`PyModuleOp`, `PyIntegerType`, …) and their
//! `#[pymethods]` mirrors are generated from the token exports pliron-derive
//! leaves at pliron's crate root; hand-written `#[pymethods]` blocks below add
//! the constructors the generated mirrors cannot express (pyo3's
//! `multiple-pymethods` feature allows several blocks per class).
//!
//! The `use` statements matter: generated code references the wrapped types
//! and their method-signature types by the idents used at the definition site.

pub mod ops {
    #[allow(unused_imports)]
    use pliron::{
        builtin::{
            ops::{
                ConstantOp, ForwardRefOp, FuncOp, ModuleOp, ReturnOp, UnrealizedConversionCastOp,
            },
            types::FunctionType,
        },
        identifier::Identifier,
        r#type::{TypeHandle, TypedHandle},
        value::Value,
    };

    pliron::__pliron_reflect_op_ModuleOp!(crate::derive::py_op_from_export);
    pliron::__pliron_reflect_op_FuncOp!(crate::derive::py_op_from_export);
    pliron::__pliron_reflect_op_ConstantOp!(crate::derive::py_op_from_export);
    pliron::__pliron_reflect_op_ForwardRefOp!(crate::derive::py_op_from_export);
    pliron::__pliron_reflect_op_UnrealizedConversionCastOp!(crate::derive::py_op_from_export);
    pliron::__pliron_reflect_op_ReturnOp!(crate::derive::py_op_from_export);

    pliron::__pliron_reflect_op_impl_ModuleOp!(crate::derive::py_op_impl_from_export);
    pliron::__pliron_reflect_op_impl_FuncOp!(crate::derive::py_op_impl_from_export);
    pliron::__pliron_reflect_op_impl_UnrealizedConversionCastOp!(
        crate::derive::py_op_impl_from_export
    );
    pliron::__pliron_reflect_op_impl_ReturnOp!(crate::derive::py_op_impl_from_export);
}

pub mod types {
    #[allow(unused_imports)]
    use pliron::{
        builtin::types::{
            FP16Type, FP32Type, FP64Type, FunctionType, IntegerType, Signedness, UnitType,
        },
        r#type::{Type, TypeHandle, TypeSig, TypedHandle},
    };

    pliron::__pliron_reflect_ty_IntegerType!(crate::derive::py_type_from_export);
    pliron::__pliron_reflect_ty_FunctionType!(crate::derive::py_type_from_export);
    pliron::__pliron_reflect_ty_UnitType!(crate::derive::py_type_from_export);
    pliron::__pliron_reflect_ty_FP32Type!(crate::derive::py_type_from_export);
    pliron::__pliron_reflect_ty_FP64Type!(crate::derive::py_type_from_export);
    pliron::__pliron_reflect_ty_FP16Type!(crate::derive::py_type_from_export);

    pliron::__pliron_reflect_ty_impl_IntegerType!(crate::derive::py_type_impl_from_export);

    // -----------------------------------------------------------------------
    // Hand-written constructors for the generated pyclass wrappers
    // -----------------------------------------------------------------------

    #[pyo3::pymethods]
    impl PyIntegerType {
        /// Create or get a uniqued IntegerType.
        ///
        /// `signedness` must be one of `"signed"`, `"unsigned"`, or `"signless"` (default).
        #[staticmethod]
        #[pyo3(signature = (width, signedness=None))]
        fn get(width: u32, signedness: Option<&str>) -> pyo3::PyResult<PyIntegerType> {
            let ctx = crate::get_ctx_mut()?;
            let sign = match signedness.unwrap_or("signless") {
                "signed" => Signedness::Signed,
                "unsigned" => Signedness::Unsigned,
                "signless" => Signedness::Signless,
                other => {
                    return Err(crate::PlironError::new_err(format!(
                        "Invalid signedness '{}', expected signed/unsigned/signless",
                        other
                    )));
                }
            };
            let ptr = IntegerType::get(ctx, width, sign);
            Ok(PyIntegerType { ptr })
        }
    }

    #[pyo3::pymethods]
    impl PyFunctionType {
        /// Create or get a uniqued FunctionType.
        ///
        /// `inputs` and `results` are lists of `Type` handles.
        #[staticmethod]
        fn get(
            inputs: Vec<pyo3::Bound<'_, pyo3::PyAny>>,
            results: Vec<pyo3::Bound<'_, pyo3::PyAny>>,
        ) -> pyo3::PyResult<PyFunctionType> {
            let in_ptrs = crate::types::type_handles_from_any(&inputs)?;
            let res_ptrs = crate::types::type_handles_from_any(&results)?;
            let ctx = crate::get_ctx_mut()?;
            let ptr = FunctionType::get(ctx, in_ptrs, res_ptrs);
            Ok(PyFunctionType { ptr })
        }
    }

    #[pyo3::pymethods]
    impl PyUnitType {
        /// Get the singleton UnitType.
        #[staticmethod]
        fn get() -> pyo3::PyResult<PyUnitType> {
            let ctx = crate::get_ctx()?;
            let ptr = UnitType::get(ctx);
            Ok(PyUnitType { ptr })
        }
    }
}

pub mod attributes {
    #[allow(unused_imports)]
    use pliron::{
        attribute::{AttrObj, AttributeDict},
        builtin::{
            attributes::{
                BoolAttr, DictAttr, FPDoubleAttr, FPHalfAttr, FPSingleAttr, IdentifierAttr,
                IntegerAttr, OperandSegmentSizesAttr, StringAttr, TypeAttr, UnitAttr, VecAttr,
            },
            types::IntegerType,
        },
        identifier::Identifier,
        r#type::{TypeHandle, TypedHandle},
        utils::{apfloat, apint::APInt},
    };

    pliron::__pliron_reflect_attr_IdentifierAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_StringAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_BoolAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_IntegerAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_FPHalfAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_FPSingleAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_FPDoubleAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_DictAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_VecAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_UnitAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_TypeAttr!(crate::derive::py_attr_from_export);
    pliron::__pliron_reflect_attr_OperandSegmentSizesAttr!(crate::derive::py_attr_from_export);

    pliron::__pliron_reflect_attr_impl_StringAttr!(crate::derive::py_attr_impl_from_export);
    pliron::__pliron_reflect_attr_impl_BoolAttr!(crate::derive::py_attr_impl_from_export);
    pliron::__pliron_reflect_attr_impl_UnitAttr!(crate::derive::py_attr_impl_from_export);

    // -----------------------------------------------------------------------
    // Hand-written constructors for the generated pyclass wrappers
    // -----------------------------------------------------------------------

    // The generic `APInt::from_py` path is unimplemented, so IntegerAttr gets a
    // hand-written constructor taking a plain i64 plus an integer type.
    #[pyo3::pymethods]
    impl PyIntegerAttr {
        #[staticmethod]
        fn new(value: i64, ty: &pyo3::Bound<'_, pyo3::PyAny>) -> pyo3::PyResult<Self> {
            let ty_handle = crate::types::type_handle_from_any(ty)?;
            let ctx = crate::get_ctx()?;
            let width = {
                let ty_guard = ty_handle.deref(ctx);
                let int_ty = ty_guard
                    .downcast_ref::<IntegerType>()
                    .ok_or_else(|| crate::PlironError::new_err("Expected an integer type"))?;
                int_ty.width() as usize
            };
            let ty = TypedHandle::<IntegerType>::from_handle(ty_handle, ctx)
                .map_err(crate::to_py_err)?;
            let val = APInt::from_str(&value.to_string(), width, 10).map_err(crate::to_py_err)?;
            Ok(Self {
                inner: IntegerAttr::new(ty, val),
            })
        }

        fn value(&self) -> pyo3::PyResult<i64> {
            Ok(self.inner.value().to_i64())
        }
    }
}
