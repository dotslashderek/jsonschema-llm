//! Conversion pass modules.
//!
//! Each pass is a self-contained transformation that operates on a JSON Schema.
//! Passes are executed in order (0–9) and each assumes the output of previous passes.
//! Shared cross-pass utilities live in `pass_utils`.

pub mod pass_result;
pub mod pass_utils;

pub mod p0_normalize;
pub mod p1_composition;
pub mod p2_polymorphism;
pub mod p3_dictionary;
pub mod p4_opaque;
pub mod p5_recursion;
pub mod p6_strict;
pub mod p7_constraints;
pub mod p8_adaptive_opaque;
pub mod p9_provider_compat;
