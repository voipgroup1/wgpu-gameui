//! World-space → UI-pixel projection helpers.
//!
//! This crate has no camera or 3D knowledge of its own. These functions let a
//! consumer that *does* (a game with a view-projection matrix) place 2D UI —
//! labels, damage numbers, health bars — anchored to 3D world positions. They
//! back Teardown's `UiWorldToPixel` / `UiWorldToScreen`.
//!
//! # Coordinate conventions
//!
//! - Output is **UI pixel space**: origin top-left, X right, Y **down** — the
//!   same space `UiContext::translate` / `DrawList` operate in. So a projected
//!   `(sx, sy)` can be fed straight into `UiContext::translate(sx, sy)`.
//! - The `view_proj` matrix is **column-major** (`vp[col][row]`), matching the
//!   layout produced by `nalgebra`'s `Matrix4::into() -> [[f32; 4]; 4]` and what
//!   wgpu/`bytemuck` upload. **Note:** this crate's own internal
//!   `ortho_matrix` (in the renderer) is row-major; do not confuse the two —
//!   feed a camera view-proj here, not the UI ortho.
//! - wgpu clip space: X,Y in `[-1, 1]`, Y up, near plane at `z = 0`. A point is
//!   in front of the camera when its clip `w > 0`.

use nalgebra::{Matrix4, Point3};

/// Project a world point to UI pixel space (top-left origin, Y-down).
///
/// `view_proj` is column-major (`vp[col][row]`). `width`/`height` are the
/// viewport dimensions in the **same pixel space** the UI is laid out in
/// (logical pixels when a DPI scale is in use — see [`crate::UiRenderer`]).
///
/// Returns `Some((sx, sy, w))` where `(sx, sy)` is the pixel position and `w`
/// is the clip-space w (the perspective divisor, `> 0` in front of the camera
/// and proportional to view-space depth). Returns `None` when the point is on
/// or behind the near plane (`w <= 0`), so callers can skip drawing without a
/// separate visibility test.
pub fn world_to_screen(
    world: [f32; 3],
    view_proj: &[[f32; 4]; 4],
    width: f32,
    height: f32,
) -> Option<(f32, f32, f32)> {
    let vp = view_proj;
    let [px, py, pz] = world;
    // Column-major: clip = vp * [x y z 1]^T, reading down each column.
    let x = vp[0][0] * px + vp[1][0] * py + vp[2][0] * pz + vp[3][0];
    let y = vp[0][1] * px + vp[1][1] * py + vp[2][1] * pz + vp[3][1];
    let w = vp[0][3] * px + vp[1][3] * py + vp[2][3] * pz + vp[3][3];

    if w <= 0.0 {
        // On or behind the near plane — projecting would mirror the point.
        return None;
    }

    let ndc_x = x / w;
    let ndc_y = y / w;

    // NDC (Y up, -1 at bottom) → pixel (Y down, 0 at top).
    let sx = (ndc_x + 1.0) * 0.5 * width;
    let sy = (1.0 - ndc_y) * 0.5 * height;

    Some((sx, sy, w))
}

/// `nalgebra`-typed convenience over [`world_to_screen`].
///
/// Takes a `Point3<f32>` and a `Matrix4<f32>` (nalgebra is column-major, so its
/// internal storage matches the `[[f32; 4]; 4]` convention used by
/// [`world_to_screen`]). Identical semantics and return value.
pub fn world_to_screen_na(
    world: Point3<f32>,
    view_proj: &Matrix4<f32>,
    width: f32,
    height: f32,
) -> Option<(f32, f32, f32)> {
    // Matrix4 stores column-major; `.into()` yields [[col0],[col1],...] = [col][row].
    let cols: [[f32; 4]; 4] = (*view_proj).into();
    world_to_screen([world.x, world.y, world.z], &cols, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{Matrix4, Perspective3, Point3};

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /// A view-projection looking down -Z from the origin, right-handed.
    /// World +X is right, world +Y is up.
    fn camera() -> Matrix4<f32> {
        // Perspective with a 90° vertical FOV, 1:1 aspect.
        let proj = Perspective3::new(1.0, std::f32::consts::FRAC_PI_2, 0.1, 100.0);
        // View: camera at origin looking toward -Z (identity view leaves the
        // camera at origin looking down -Z in RH/OpenGL-style eye space).
        proj.to_homogeneous()
    }

    #[test]
    fn point_on_axis_lands_at_center() {
        let vp = camera();
        // Straight ahead, 10 units down -Z.
        let p = Point3::new(0.0, 0.0, -10.0);
        let (sx, sy, w) = world_to_screen_na(p, &vp, 800.0, 600.0).expect("in front");
        assert!(approx(sx, 400.0, 0.5), "sx={sx}");
        assert!(approx(sy, 300.0, 0.5), "sy={sy}");
        assert!(w > 0.0);
    }

    #[test]
    fn behind_camera_is_none() {
        let vp = camera();
        // Behind the camera (positive Z is behind when looking down -Z).
        let p = Point3::new(0.0, 0.0, 10.0);
        assert!(world_to_screen_na(p, &vp, 800.0, 600.0).is_none());
    }

    #[test]
    fn point_to_the_right_maps_right_of_center() {
        let vp = camera();
        // To the right (+X) and ahead: should land right of screen center.
        let p = Point3::new(2.0, 0.0, -10.0);
        let (sx, _sy, _w) = world_to_screen_na(p, &vp, 800.0, 600.0).expect("in front");
        assert!(sx > 400.0, "expected right of center, got sx={sx}");
    }

    #[test]
    fn point_above_maps_above_center() {
        let vp = camera();
        // Above (+Y world) and ahead: Y-down screen → smaller sy than center.
        let p = Point3::new(0.0, 2.0, -10.0);
        let (_sx, sy, _w) = world_to_screen_na(p, &vp, 800.0, 600.0).expect("in front");
        assert!(
            sy < 300.0,
            "expected above center (smaller sy), got sy={sy}"
        );
    }

    #[test]
    fn array_and_nalgebra_overloads_agree() {
        let vp = camera();
        let cols: [[f32; 4]; 4] = vp.into();
        let p = Point3::new(1.5, -0.7, -12.0);
        let a = world_to_screen([p.x, p.y, p.z], &cols, 1280.0, 720.0);
        let b = world_to_screen_na(p, &vp, 1280.0, 720.0);
        assert_eq!(a, b);
    }

    #[test]
    fn exactly_on_near_plane_is_none() {
        // w == 0 must be rejected (would divide by zero / mirror).
        let vp = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 0.0], // w-row contributes 0 → clip w = 0
        ];
        assert!(world_to_screen([0.0, 0.0, 0.0], &vp, 800.0, 600.0).is_none());
    }
}
