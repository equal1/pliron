//! Test-only ops shared between the opt test modules.
//!
//! `TestRegionOp` used to be defined identically in both `sccp` and
//! `simplify_cfg`; the derive macros now export a crate-root reflection macro
//! per op (see pliron-derive's `reflect` module), so same-named ops in one
//! crate would collide.

use pliron::{
    builtin::op_interfaces::{NOpdsInterface, NResultsInterface},
    derive::pliron_op,
};

#[pliron_op(
    name = "test.test_region",
    format = "region($0)",
    interfaces = [NOpdsInterface<0>, NResultsInterface<0>],
    verifier = "succ"
)]
pub struct TestRegionOp;
