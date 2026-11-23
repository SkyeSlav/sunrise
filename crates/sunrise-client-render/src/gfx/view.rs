//! The views a frame is drawn from. A flat screen has one; a headset has
//! one per eye. Every view shares the frame's camera position, so the
//! camera-relative world (section origins, entity offsets) is built once and
//! only each view's rotation and lens differ.

use super::{Frame, Frustum, JoinedFrusta, Renderer, frustum, taa_frustum};
#[cfg(feature = "test-support")]
use glam::Vec4;
use glam::{DVec3, Mat4, Vec3};

/// The most views one frame draws.
const MAX_VIEWS: usize = 2;

/// One eye's view of the camera-relative world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Camera-relative world to eye: a rotation, plus the eye's offset from
    /// the camera on a headset.
    pub view: Mat4,
    /// Reversed-Z perspective: the near plane at depth 1, far at 0.
    pub projection: Mat4,
    /// Width over height of the image this view fills.
    pub aspect: f32,
    /// `tan(fov_y / 2)`, for shaders rebuilding view rays from NDC.
    pub tan_half_fov: f32,
}

impl View {
    /// The flat-screen view: vanilla's camera rotation and a symmetric
    /// vertical field of view.
    pub fn screen(yaw: f32, pitch: f32, fov_degrees: f32, aspect: f32, far: f32) -> Self {
        Self {
            view: crate::camera::view_matrix(yaw, pitch),
            projection: super::world_projection(fov_degrees, aspect, far),
            aspect,
            tan_half_fov: (fov_degrees.to_radians() / 2.0).tan(),
        }
    }

    /// Clip space from the camera-relative world, with `pose` (view bobbing,
    /// nausea spin) between the lens and the rotation.
    pub fn view_proj(&self, pose: Mat4) -> Mat4 {
        self.projection * pose * self.view
    }
}

/// Every view of a frame, and the one geometry is culled with.
#[derive(Clone, Copy, Debug)]
pub struct ViewSet {
    views: [View; MAX_VIEWS],
    len: usize,
    cull: View,
}

/// Composed draw transform and unposed cull transform for one frame (`GameRenderer.renderLevel`).
pub(super) struct FrameTransforms {
    pub draw_view_proj: Mat4,
    pub cull_view_proj: Mat4,
    pub cull_frustum: Frustum,
    pub rendered_eye: Option<Vec3>,
}

impl ViewSet {
    /// A frame drawn from `view` alone, culled with it.
    pub fn single(view: View) -> Self {
        Self {
            views: [view; MAX_VIEWS],
            len: 1,
            cull: view,
        }
    }

    /// A frame drawn from `view` and culled with `cull`.
    pub fn single_with_cull(view: View, cull: View) -> Self {
        Self {
            views: [view; MAX_VIEWS],
            len: 1,
            cull,
        }
    }

    /// Build separate draw and cull views for the frame.
    pub(super) fn for_frame(frame: &Frame, width: u32, height: u32) -> Self {
        let aspect = width as f32 / height.max(1) as f32;
        let view = View::screen(
            frame.yaw,
            frame.pitch,
            frame.fov_degrees,
            aspect,
            frame.depth_far,
        );
        let cull = View::screen(
            frame.yaw,
            frame.pitch,
            cull_fov(frame.fov_degrees, frame.cull_fov_degrees),
            aspect,
            frame.depth_far,
        );
        Self::single_with_cull(view, cull)
    }

    /// The view the HUD, hand and screen-space effects follow.
    pub fn primary(&self) -> &View {
        &self.views[0]
    }

    pub fn views(&self) -> &[View] {
        &self.views[..self.len]
    }

    /// The view whose frustum covers every other view's.
    pub fn cull(&self) -> &View {
        &self.cull
    }

    /// Unposed culling transform (`Camera.prepareCullFrustum`: no view bob or nausea spin).
    pub fn cull_view_proj(&self) -> Mat4 {
        self.cull.view_proj(Mat4::IDENTITY)
    }

    /// Composed draw transform for drawing and eye recovery, beside the unposed cull transform.
    pub(super) fn frame_transforms(&self, pose: Mat4) -> FrameTransforms {
        let draw_view_proj = self.primary().view_proj(pose);
        let cull_view_proj = self.cull_view_proj();
        FrameTransforms {
            draw_view_proj,
            cull_view_proj,
            cull_frustum: Frustum::from_planes(frustum(cull_view_proj)),
            rendered_eye: projection_center(draw_view_proj),
        }
    }
}

/// `Camera.createProjectionMatrixForCulling`: culling FOV is `max(fov, option_fov)`.
pub fn cull_fov(fov_degrees: f32, option_fov: f32) -> f32 {
    fov_degrees.max(option_fov)
}

/// `Camera.prepareCullFrustum`: culling frustum at `max(fov, option_fov)` with no bob or spin.
pub fn cull_frustum(
    fov_degrees: f32,
    option_fov: f32,
    yaw: f32,
    pitch: f32,
    aspect: f32,
    depth_far: f32,
) -> Frustum {
    let view = View::screen(
        yaw,
        pitch,
        cull_fov(fov_degrees, option_fov),
        aspect,
        depth_far,
    );
    Frustum::from_planes(frustum(view.view_proj(Mat4::IDENTITY)))
}

impl Renderer {
    /// The views `render` draws `frame` from.
    pub fn views(&self, frame: &Frame) -> ViewSet {
        ViewSet::for_frame(frame, self.config.width, self.config.height)
    }
    /// The frusta `render` will cull with from `camera`, for culling geometry built before the
    /// frame (entities, block entities) the same way: the camera's and this frame's world
    /// views', which it asks the mods for (once a frame, `view_sections`).
    pub fn camera_frustum(
        &mut self,
        camera: DVec3,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        _bob: Mat4,
        depth_far: f32,
    ) -> JoinedFrusta {
        let views = ViewSet::single(self.screen_view(fov_degrees, yaw, pitch, depth_far));
        // Vanilla Camera.prepareCullFrustum omits view bob and nausea spin.
        let planes = frustum(views.cull_view_proj());
        self.ask_world_views(camera, depth_far, Frustum::from_planes(planes))
    }
    /// A conservative view for geometry built before TAA chooses this frame's
    /// jitter. Cover a whole rendered pixel on each side, at the configured
    /// resolution scale/upscale, rather than borrowing the previous frame's size.
    /// With `camera_frustum`'s world views.
    pub fn camera_frustum_with_jitter(
        &self,
        fov_degrees: f32,
        yaw: f32,
        pitch: f32,
        _bob: Mat4,
        depth_far: f32,
    ) -> JoinedFrusta {
        let views = ViewSet::single(self.screen_view(fov_degrees, yaw, pitch, depth_far));
        let size = self
            .world_post
            .taa_render_size([self.config.width, self.config.height]);
        self.asked_world_views(taa_frustum(views.cull_view_proj(), size))
    }
    fn screen_view(&self, fov_degrees: f32, yaw: f32, pitch: f32, far: f32) -> View {
        let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        View::screen(yaw, pitch, fov_degrees, aspect, far)
    }
}

/// Where a perspective `view_proj` sees from, in its own (camera-relative)
/// space: the point whose clip x, y and w are all 0. `None` for a
/// projection without one (orthographic).
pub(super) fn projection_center(view_proj: Mat4) -> Option<Vec3> {
    let m = view_proj.as_dmat4();
    let rows = [m.row(0), m.row(1), m.row(3)];
    let a = glam::DMat3::from_cols(
        DVec3::new(rows[0].x, rows[1].x, rows[2].x),
        DVec3::new(rows[0].y, rows[1].y, rows[2].y),
        DVec3::new(rows[0].z, rows[1].z, rows[2].z),
    );
    let det = a.determinant();
    if !det.is_normal() {
        return None;
    }
    let b = DVec3::new(rows[0].w, rows[1].w, rows[2].w);
    Some((a.inverse() * -b).as_vec3())
}

/// `view_proj` mirrored about the horizontal plane at the camera-relative height `plane`:
/// with F the mirror (y flips and moves to 2 * plane), `view_proj * F` negates column 1
/// and moves column 3 by 2 * plane times column 1.
///
/// `mirror_inverse_view` is the inverse's other side: F * `inverse_view`.
pub(super) fn mirror_view_projection(view_proj: Mat4, plane: f32) -> Mat4 {
    let mut m = view_proj.to_cols_array_2d();
    let col1 = m[1];
    m[1] = col1.map(|v| -v);
    m[3] = std::array::from_fn(|i| m[3][i] + 2.0 * plane * col1[i]);
    Mat4::from_cols_array_2d(&m)
}

/// The mirror's `inverse_view` (the sky's rays): what the view sees, mirrored about the
/// horizontal plane at the camera-relative height `plane`, so F * `inverse_view`.
pub(super) fn mirror_inverse_view(inverse_view: [[f32; 4]; 4], plane: f32) -> [[f32; 4]; 4] {
    inverse_view.map(|[x, y, z, w]| [x, 2.0 * plane * w - y, z, w])
}

#[cfg(feature = "test-support")]
pub fn test_frustum(planes: [Vec4; 6]) -> JoinedFrusta {
    JoinedFrusta::new(super::Frustum::from_planes(planes), [])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The formulas `Renderer` used before views existed, kept to pin the
    /// screen view to them bit for bit.
    fn legacy_view_proj(fov: f32, yaw: f32, pitch: f32, pose: Mat4, aspect: f32, far: f32) -> Mat4 {
        super::super::world_projection(fov, aspect, far)
            * pose
            * crate::camera::view_matrix(yaw, pitch)
    }

    fn poses() -> [Mat4; 3] {
        [
            Mat4::IDENTITY,
            Mat4::from_translation(glam::Vec3::new(0.013, -0.027, 0.0))
                * Mat4::from_rotation_z(0.021),
            Mat4::from_rotation_y(0.4) * Mat4::from_scale(glam::Vec3::new(1.1, 0.9, 1.0)),
        ]
    }

    #[test]
    fn the_screen_view_matches_the_legacy_projection_bit_for_bit() {
        for (fov, yaw, pitch, aspect, far) in [
            (70.0, 0.0, 0.0, 16.0 / 9.0, 512.0),
            (110.0, -137.25, 61.5, 21.0 / 9.0, 8192.0),
            (30.0, 359.9, -90.0, 4.0 / 3.0, 96.0),
            (82.333, 12.5, 7.75, 1.0, 1024.0),
        ] {
            let view = View::screen(yaw, pitch, fov, aspect, far);
            for pose in poses() {
                let old = legacy_view_proj(fov, yaw, pitch, pose, aspect, far);
                let new = view.view_proj(pose);
                assert_eq!(
                    old.to_cols_array().map(f32::to_bits),
                    new.to_cols_array().map(f32::to_bits),
                    "fov {fov} yaw {yaw} pitch {pitch}"
                );
            }
            assert_eq!(view.view, crate::camera::view_matrix(yaw, pitch));
            assert_eq!(
                view.tan_half_fov.to_bits(),
                (fov.to_radians() / 2.0).tan().to_bits()
            );
            assert_eq!(view.aspect.to_bits(), aspect.to_bits());
        }
    }

    #[test]
    fn a_single_view_set_draws_and_culls_with_its_view() {
        let view = View::screen(45.0, 10.0, 70.0, 16.0 / 9.0, 512.0);
        let set = ViewSet::single(view);
        assert_eq!(set.views(), &[view]);
        assert_eq!(*set.primary(), view);
        assert_eq!(*set.cull(), view);
    }

    #[test]
    #[ignore = "run by tools/test_camera_parity.py"]
    fn cull_frustum_matches_26_3_witness_with_zoom_widen_and_bob() {
        let path = std::env::var("SUNRISE_CAMERA_PARITY_WITNESS")
            .expect("run through tools/test_camera_parity.py, which records the witness");
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!(
                "failed to read SUNRISE_CAMERA_PARITY_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
            )
        });
        let witness: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let f32_hex = |v: &serde_json::Value| {
            f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
        };
        let f64_hex = |v: &serde_json::Value| {
            f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
        };
        let cases = witness["cull_frustum"]["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 3);
        let bob = Mat4::from_translation(Vec3::new(0.5, -0.5, 0.0)) * Mat4::from_rotation_z(0.35);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let fov = f32_hex(&case["fov_bits"]);
            let opt_fov = case["option_fov"].as_i64().unwrap() as f32;
            let yaw = f32_hex(&case["yaw_bits"]);
            let pitch = f32_hex(&case["pitch_bits"]);
            let depth_far = f32_hex(&case["depth_far_bits"]);
            let width = case["width"].as_f64().unwrap() as f32;
            let height = case["height"].as_f64().unwrap() as f32;
            let aspect = width / height;

            let render_view = View::screen(yaw, pitch, fov, aspect, depth_far);
            let cull_view = View::screen(yaw, pitch, cull_fov(fov, opt_fov), aspect, depth_far);
            let hud = crate::hud::HudBatch::scratch([4, 4]);
            let mut frame = crate::gfx::frame_tests::frame(&hud);
            frame.camera = DVec3::ZERO;
            frame.yaw = yaw;
            frame.pitch = pitch;
            frame.fov_degrees = fov;
            frame.cull_fov_degrees = opt_fov;
            frame.depth_far = depth_far;
            frame.bob = bob;
            frame.spin = Mat4::IDENTITY;
            let set = ViewSet::for_frame(&frame, width as u32, height as u32);
            assert_eq!(*set.primary(), render_view);
            assert_eq!(*set.cull(), cull_view);
            let transforms = set.frame_transforms(frame.bob * frame.spin);
            assert_eq!(
                transforms.cull_view_proj.to_cols_array().map(f32::to_bits),
                set.cull_view_proj().to_cols_array().map(f32::to_bits),
                "{name} draw_frame cull transform must be unposed"
            );
            assert_eq!(
                transforms.draw_view_proj.to_cols_array().map(f32::to_bits),
                render_view.view_proj(bob).to_cols_array().map(f32::to_bits),
                "{name} draw_frame draw transform must keep bob/spin pose"
            );
            let expected_eye = projection_center(transforms.draw_view_proj).unwrap();
            assert_eq!(
                transforms
                    .rendered_eye
                    .unwrap()
                    .to_array()
                    .map(f32::to_bits),
                expected_eye.to_array().map(f32::to_bits),
                "{name} rendered_eye must come from composed draw transform"
            );
            assert!(
                expected_eye.length() > 0.1,
                "{name} bob pose must shift rendered_eye off camera origin"
            );

            let want_view_rot: Vec<u32> = case["view_rot_bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
                .collect();
            let got_view_rot = crate::camera::view_rotation_matrix(yaw, pitch)
                .0
                .to_cols_array()
                .map(f32::to_bits);
            assert_eq!(
                got_view_rot.as_slice(),
                want_view_rot.as_slice(),
                "{name} view_rot exact bits"
            );
            let got_cull_proj = crate::camera::perspective(
                cull_fov(fov, opt_fov),
                width,
                height,
                0.05,
                depth_far,
                true,
            )
            .map(f32::to_bits);
            let want_cull_proj: Vec<u32> = case["cull_proj_bits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
                .collect();
            assert_eq!(
                got_cull_proj.as_slice(),
                want_cull_proj.as_slice(),
                "{name} cull_proj exact bits"
            );

            let got_frustum = cull_frustum(fov, opt_fov, yaw, pitch, aspect, depth_far);
            let view_frustum = transforms.cull_frustum;
            let raw_fov_frustum =
                Frustum::from_planes(frustum(render_view.view_proj(Mat4::IDENTITY)));
            let bobbed_frustum = Frustum::from_planes(frustum(cull_view.view_proj(bob)));

            let boxes = case["boxes"].as_array().unwrap();
            let mut raw_diffs = 0usize;
            let mut bob_diffs = 0usize;
            for (idx, b) in boxes.iter().enumerate() {
                let ab = b["aabb_bits"].as_array().unwrap();
                let min = Vec3::new(
                    f64_hex(&ab[0]) as f32,
                    f64_hex(&ab[1]) as f32,
                    f64_hex(&ab[2]) as f32,
                );
                let max = Vec3::new(
                    f64_hex(&ab[3]) as f32,
                    f64_hex(&ab[4]) as f32,
                    f64_hex(&ab[5]) as f32,
                );
                let want = b["visible"].as_bool().unwrap();
                assert_eq!(
                    got_frustum.intersects(min, max),
                    want,
                    "{name} box {idx} via cull_frustum"
                );
                assert_eq!(
                    view_frustum.intersects(min, max),
                    want,
                    "{name} box {idx} via draw_frame unposed cull frustum"
                );
                if raw_fov_frustum.intersects(min, max) != want {
                    raw_diffs += 1;
                }
                if bobbed_frustum.intersects(min, max) != want {
                    bob_diffs += 1;
                }
            }
            if name == "zoomed_fov_30_opt_70" {
                let mut sections = crate::gfx::resident::Resident::default();
                let edge_section = (2, 2, 1);
                sections.insert(edge_section, ());
                let mut visible_unposed = Vec::new();
                sections.visible(
                    DVec3::ZERO,
                    8,
                    &transforms.cull_frustum,
                    |_| true,
                    |_, _| false,
                    &mut visible_unposed,
                );
                let mut visible_bobbed = Vec::new();
                sections.visible(
                    DVec3::ZERO,
                    8,
                    &bobbed_frustum,
                    |_| true,
                    |_, _| false,
                    &mut visible_bobbed,
                );
                let mut visible_narrow = Vec::new();
                sections.visible(
                    DVec3::ZERO,
                    8,
                    &raw_fov_frustum,
                    |_| true,
                    |_, _| false,
                    &mut visible_narrow,
                );
                assert_eq!(
                    visible_unposed.iter().map(|v| v.key).collect::<Vec<_>>(),
                    vec![edge_section],
                    "renderer section walk must keep edge section under bob/nausea and zoom"
                );
                assert!(
                    visible_bobbed.is_empty(),
                    "bobbed cull frustum would drop the edge section"
                );
                assert!(
                    visible_narrow.is_empty(),
                    "narrowed draw FOV frustum would drop the edge section"
                );
            }
            if fov < opt_fov {
                assert!(
                    raw_diffs > 0,
                    "{name}: narrowed render FOV must differ from cull FOV"
                );
                assert!(
                    bob_diffs > 0,
                    "{name}: bobbed culling must differ from unbobbed 26.3 witness"
                );
            }
        }
    }
}
