//! Turtle fractals driven by a number base `b` and a direction number `dir`, up to `2^50` steps.
//!
//! [`fractal`] holds the rule, the block identity and the self-similarity criterion behind every
//! renderer; the per-digit contribution is switched with [`fractal::DigitRule`].

pub mod animate;
pub mod deep;
pub mod fractal;
pub mod gif;
pub mod palette;
pub mod png;
pub mod render;
pub mod scene;
