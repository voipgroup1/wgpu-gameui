//! Utilities for applying geometric transformations.

use na::Affine2;

/// An object that can be transformed under an affine transformation.
pub trait Transform {
    /// Transforms this curve using an affine transformation.
    ///
    /// Bézier curves are transformed by applying the transformation to each of the control points.
    fn transform(&mut self, transformation: &Affine2<f64>);
}
