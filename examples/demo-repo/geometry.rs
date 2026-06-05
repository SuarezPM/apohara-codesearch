// SPDX-License-Identifier: MIT OR Apache-2.0
//
// A small geometry module: a couple of free functions plus a method, with
// real imports and a `pub use` re-export so the indexer records structural
// rows (file_imports / file_exports) for this file.

use std::f64::consts::PI;
use std::fmt;

pub use self::Shape;

/// Compute the area of a circle from its radius.
pub fn circle_area(radius: f64) -> f64 {
    PI * radius * radius
}

/// Compute the perimeter (circumference) of a circle from its radius.
pub fn circle_perimeter(radius: f64) -> f64 {
    2.0 * PI * radius
}

/// A circle shape carrying its radius.
pub struct Shape {
    pub radius: f64,
}

impl Shape {
    /// Scale the shape's radius by a factor, returning the new area.
    pub fn scaled_area(&self, factor: f64) -> f64 {
        circle_area(self.radius * factor)
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shape(radius={})", self.radius)
    }
}
