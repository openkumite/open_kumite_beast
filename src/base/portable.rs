//! The fight logic's float math from simulation version 3: the same bits on every target.
//!
//! Engine addition.
//! State: none (its results feed the state: changing one bit is a `SIM_VERSION` change)
//!
//! The fight logic's float math from simulation version 3 (game-state stage 7): the same bits on every target
//! (macOS / Linux / Windows, aarch64 / x86_64, wasm32), which netplay and replays across machines need.
//!
//! The engine's skeleton (glTF nodes posed from the clips or the pose records) is float math whose results feed the
//! logic: node positions for hits and the push-apart, node rotations for the effect VM, the spring chains. Up to
//! version 2 it ran on glam's f32 types (NEON / SSE2 / scalar by target: other summation orders) and the platform's
//! sin / cos / atan2 (Apple's, glibc's, msvcrt's, compiler-builtins' are not the same bits); that path stays as it
//! was for the older replays (`Loaded::portable` false). From version 3 the logic's pose (`formats::gltf_pose`) runs on
//! this math instead: glam's f64 types (one scalar implementation on every target; Rust never fuses `a * b + c` on its
//! own) with the trig of the pure-Rust `libm` crate. Sums of products, square roots and divisions are IEEE-exact
//! everywhere.

use glam::{DMat3, DMat4, DQuat, DVec3, DVec4, Quat};

/// The logic's f32 trig: `libm`'s when `portable`, else the platform's (`std`; versions 1 and 2).
pub fn sin_cos(x: f32, portable: bool) -> (f32, f32) {
    if portable { libm::sincosf(x) } else { x.sin_cos() }
}
pub fn sin(x: f32, portable: bool) -> f32 {
    if portable { libm::sinf(x) } else { x.sin() }
}
pub fn cos(x: f32, portable: bool) -> f32 {
    if portable { libm::cosf(x) } else { x.cos() }
}
pub fn atan2(y: f32, x: f32, portable: bool) -> f32 {
    if portable { libm::atan2f(y, x) } else { y.atan2(x) }
}
/// The effect code's f64 atan2 (fxvm op 0F, fxnative's wall marks): `libm`'s when `portable`, else the platform's.
pub fn atan2_f64(y: f64, x: f64, portable: bool) -> f64 {
    if portable { libm::atan2(y, x) } else { y.atan2(x) }
}

/// The effect code's rsin on the 4096-unit circle (fight/fx/script_vm, fight/fx/script_link.rs): libm's f64 sine of the angle's low 12 bits
/// (the game's table index), rounded to 4.12. Every value of the circle lies more than 5e-4 from a rounding
/// boundary (`tests::rsin_table`), so any sine closer than that (Apple's, glibc's, msvcrt's, libm's), with the
/// argument masked or not, gives these integers: the same for every simulation version.
pub fn rsin(a: i32) -> i32 {
    (libm::sin((a & 0xFFF) as f64 * std::f64::consts::TAU / 4096.0) * 4096.0).round() as i32
}

/// Rotation about X by `a` radians (glam's `from_rotation_x`, libm's sine).
pub fn rot_x(a: f64) -> DQuat {
    let (s, c) = libm::sincos(a * 0.5);
    DQuat::from_xyzw(s, 0.0, 0.0, c)
}

/// Rotation by `a` radians about the unit `axis` (glam's `from_axis_angle`, libm's sine).
pub fn axis_angle(axis: DVec3, a: f64) -> DQuat {
    let (s, c) = libm::sincos(a * 0.5);
    let v = axis * s;
    DQuat::from_xyzw(v.x, v.y, v.z, c)
}

/// The unit axis and the angle of `q` (glam's `to_axis_angle`, libm's atan2).
pub fn to_axis_angle(q: DQuat) -> (DVec3, f64) {
    let v = DVec3::new(q.x, q.y, q.z);
    let length = v.length();
    if length >= 1e-8 { (v / length, 2.0 * libm::atan2(length, q.w)) } else { (DVec3::X, 0.0) }
}

/// Spherical interpolation from `a` to `b` (the short way), as glam's `slerp` with libm's acos / sine.
pub(crate) fn slerp(a: DQuat, mut b: DQuat, s: f64) -> DQuat {
    let mut dot = a.dot(b);
    if dot < 0.0 {
        b = -b;
        dot = -dot;
    }
    if dot > 1.0 - f64::EPSILON {
        return (a * (1.0 - s) + b * s).normalize();
    }
    let theta = libm::acos(dot);
    (a * libm::sin(theta * (1.0 - s)) + b * libm::sin(theta * s)) * (1.0 / libm::sin(theta))
}

/// The model transform of `Match::transform`: game position `m_t` (glTF space: (x, −y, −z) / 300), yaw (4096 per
/// turn) about Y.
pub fn transform(m_t: [i32; 3], yaw: i16) -> DMat4 {
    let (s, c) = libm::sincos(yaw as f64 * std::f64::consts::TAU / 4096.0);
    let t = DVec3::new(m_t[0] as f64, -m_t[1] as f64, -m_t[2] as f64) / 300.0;
    DMat4::from_translation(t) * DMat4::from_cols(DVec4::new(c, 0.0, -s, 0.0), DVec4::Y, DVec4::new(s, 0.0, c, 0.0), DVec4::W)
}

/// `physics::node_positions` in f64.
pub fn node_positions(globals: &[DMat4], model_xf: DMat4) -> Vec<[i32; 3]> {
    globals
        .iter()
        .map(|g| {
            let w = (model_xf * *g).w_axis;
            [(w.x * 300.0) as i32, (-w.y * 300.0) as i32, (-w.z * 300.0) as i32]
        })
        .collect()
}

/// `script_link::node_rotations` in f64.
pub fn node_rotations(globals: &[DMat4], model_xf: DMat4) -> Vec<[[i32; 3]; 3]> {
    let d = DMat3::from_diagonal(DVec3::new(1.0, -1.0, -1.0));
    globals
        .iter()
        .map(|g| {
            let m = DMat3::from_mat4(model_xf * *g);
            let n = DMat3::from_cols(m.x_axis.normalize_or_zero(), m.y_axis.normalize_or_zero(), m.z_axis.normalize_or_zero());
            let r = d * n * d;
            [0, 1, 2].map(|row| [0, 1, 2].map(|col| (r.col(col)[row] * 4096.0).round() as i32))
        })
        .collect()
}

/// A rotation matrix (rows, f32) as a unit quaternion (actclip's `to_quat` in f64), stored as f32.
pub fn quat_of(m: &[[f32; 3]; 3]) -> Quat {
    let c = |j: usize| DVec3::new(m[0][j] as f64, m[1][j] as f64, m[2][j] as f64);
    DQuat::from_mat3(&DMat3::from_cols(c(0), c(1), c(2))).normalize().as_quat()
}

/// A unit quaternion as a rotation matrix (rows, f32; actclip's `from_quat` in f64).
pub fn mat_of(q: DQuat) -> [[f32; 3]; 3] {
    let m = DMat3::from_quat(q);
    [0, 1, 2].map(|i| [m.x_axis[i] as f32, m.y_axis[i] as f32, m.z_axis[i] as f32])
}

#[cfg(test)]
mod tests {
    //! Bit patterns of the portable math on inputs of the logic's paths: they must be the same on every target, so a
    //! target, compiler or dependency change that alters any of them (and with it netplay and replays across
    //! machines) fails here. The determinism matrix (outside the repository) checks whole runs.
    use super::*;

    /// FNV-1a 64 over the values' bits.
    fn fnv(v: impl IntoIterator<Item = f64>) -> u64 {
        v.into_iter().fold(0xCBF2_9CE4_8422_2325, |h, x| x.to_bits().to_le_bytes().iter().fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3)))
    }

    /// The effect code's rsin (fight/fx/hit_effects, fight/fx/script_vm, fight/fx/script_link.rs: libm's f64 sine rounded to 4.12) over the whole circle. No
    /// value lies within 5e-4 of a rounding boundary, so any sine accurate to ~1e-7 (every platform's), of the
    /// masked angle or of the whole i32 (whose f64 argument is off by < 1e-5 units), gives the same integers: the
    /// platform sines of simulation versions 1 and 2 included.
    #[test]
    fn rsin_table() {
        let exact = |a: i32| libm::sin(a as f64 * std::f64::consts::TAU / 4096.0) * 4096.0;
        let margin = (0..4096).map(|a| (exact(a) - exact(a).floor() - 0.5).abs()).fold(f64::INFINITY, f64::min);
        assert!(margin > 5e-4, "{margin}");
        assert_eq!(fnv((0..4096).map(|a| rsin(a) as f64)), 0xFA67142A0DC5FB35);
        assert_eq!([rsin(0), rsin(0x200), rsin(0x400), rsin(0x155), rsin(0xC00)], [0, 2896, 4096, 2046, -4096]);
        let far = [i32::MIN, i32::MIN + 0x400, -(1 << 24) - 3, -0x401, -1, (1 << 20) + 0x155, (1 << 30) + 0x200, i32::MAX - 0x400];
        assert!(far.iter().all(|&a| exact(a).round() as i32 == rsin(a)));
        assert!(((-1 << 20)..(1 << 20)).all(|a| exact(a).round() as i32 == rsin(a)));
    }

    /// The f32 trig of the spring chains and the clips (fight/anim/spring.rs, fight/anim/act_clips.rs).
    #[test]
    fn f32_trig() {
        let a = 1000.0 * std::f32::consts::PI / 2048.0;
        let (s, c) = sin_cos(a, true);
        let v = [s, c, sin(0.7, true), cos(-2.1, true), atan2(0.3, -0.9, true), atan2(-41.5, 12.25, true)];
        assert_eq!(v.map(f32::to_bits), [0x3F7FD398, 0x3D16C31C, 0x3F24EB73, 0xBF013D97, 0x4034784B, 0xBFA45278]);
    }

    /// The skeleton's f64 steps: the model transform, the clip slerp, the jaw rotation, a connector's half rotation,
    /// the clip quaternion of a rotation matrix.
    #[test]
    fn skeleton_bits() {
        let xf = transform([1234, -567, 890], 0x5A3);
        let a = DQuat::from_xyzw(0.1, 0.7, -0.2, 0.6).normalize();
        let b = DQuat::from_xyzw(-0.3, 0.4, 0.5, 0.7).normalize();
        let q = slerp(a, b, 0.37) * rot_x(0.81);
        let (axis, angle) = to_axis_angle(q);
        let half = axis_angle(axis, angle / 2.0);
        let m = [[0.36f32, 0.48, -0.8], [-0.8, 0.6, 0.0], [0.48, 0.64, 0.6]];
        let cq = quat_of(&m).as_dquat();
        assert_eq!(fnv(xf.to_cols_array()), 0x9D5503072F0586FC);
        assert_eq!(fnv(xf.inverse().to_cols_array()), 0xA9E3D6B9EE2419C9);
        assert_eq!(fnv(q.to_array().into_iter().chain(half.to_array()).chain(cq.to_array())), 0xFE2AAF65BCADF2D2);
        let g = [DMat4::from_rotation_translation(q, DVec3::new(0.1, 0.9, -0.05))];
        assert_eq!(
            (node_positions(&g, xf), node_rotations(&g, xf)),
            (vec![[1204, -837, 905]], vec![[[-3452, 1254, 1814], [-95, 3282, -2449], [-2203, -2106, -2737]]])
        );
    }
}
