// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Plane geometry primitives. Free functions plus a couple of methods so the
// benchmark has both `function` and `method` symbol kinds to label.

use std::f64::consts::PI;

/// A point in the 2-D plane.
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    /// Euclidean distance from this point to another.
    pub fn distance_to(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// An axis-aligned rectangle defined by its width and height.
pub struct Rectangle {
    pub width: f64,
    pub height: f64,
}

impl Rectangle {
    /// Area of the rectangle: width times height.
    pub fn area(&self) -> f64 {
        self.width * self.height
    }

    /// Perimeter of the rectangle: twice the sum of width and height.
    pub fn perimeter(&self) -> f64 {
        2.0 * (self.width + self.height)
    }
}

/// Area of a circle from its radius.
pub fn circle_area(radius: f64) -> f64 {
    PI * radius * radius
}

/// Circumference of a circle from its radius.
pub fn circle_circumference(radius: f64) -> f64 {
    2.0 * PI * radius
}

/// Convert an angle from degrees to radians.
pub fn degrees_to_radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

/// Heron's formula: the area of a triangle from the lengths of its three sides.
/// Returns `NaN` for a degenerate triangle whose sides cannot close.
pub fn triangle_area_from_sides(a: f64, b: f64, c: f64) -> f64 {
    let s = (a + b + c) / 2.0;
    (s * (s - a) * (s - b) * (s - c)).sqrt()
}
