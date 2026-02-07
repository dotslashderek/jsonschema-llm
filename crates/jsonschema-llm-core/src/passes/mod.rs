//! Conversion pass modules.
//!
//! Each pass is a self-contained transformation that operates on a JSON Schema.
//! Passes are executed in order (0-7) and each assumes the output of previous passes.

pub mod p0_normalize;
pub mod p1_composition;
pub mod p2_polymorphism;
pub mod p3_dictionary;
pub mod p4_opaque;
pub mod p5_recursion;
pub mod p6_strict;
pub mod p7_constraints;
