//! Turtle fractals driven by a number base `b` and a direction number `d`.
//!
//! The whole family comes from one rule: the heading of step `n` is
//! `(PI / b) * d * digit_sum(n, b)`, i.e. every unit of base-`b` digit sum turns the
//! turtle by `d * 180 / b` degrees, and the turtle walks one unit per step.
//!
//! ```text
//! (2, 4) -> dragon curve, self-similarity ratio 1+i   (2, 6) -> hexagonal dragon
//! (3, 5) -> 72-degree dragon                          (4, 6, fold) -> Koch arc
//! dir = 1 -> straight ray          negative dir -> mirror image
//! ```
//!
//! `dir` is the number of equal parts of a full turn, and the per-digit contribution can be
//! switched with [`fractal::DigitRule`] (`linear` / `alternate` / `fold`). Every rule keeps the
//! heading additive over digits, so the block identity - and with it the deep renderer, the
//! level animation and zooming - holds unchanged for all of them.

pub mod animate;
pub mod deep;
pub mod fractal;
pub mod gif;
pub mod palette;
pub mod png;
pub mod render;
pub mod scene;
