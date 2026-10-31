//! The camera, as in Unity, Unreal or Godot: the right mouse button held looks round and
//! WASD / Q E fly (the wheel sets the speed meanwhile), the middle button pans, Alt + left
//! button orbits round the point under the cursor, the wheel moves towards the cursor.
//! Without the right button WASD / arrows move over the map and Q / E turn it.
//!
//! It is kept as a point looked at (the pivot) and the eye's distance and angles from it.

use glam::{DVec2, DVec3, Mat4, Vec3, Vec4};
use serde::{Deserialize, Serialize};

/// How fast and how sensitive the camera is (View > Controls > Camera). Every number is a
/// multiplier of the built-in behaviour (1 = as it was) except where a unit is said.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Flying speed with the right button held (m/s); the wheel changes it meanwhile.
    pub fly_speed: f64,
    /// Keys over the map and the arrows.
    pub move_speed: f32,
    /// What Shift multiplies the keyboard speed by.
    pub fast: f32,
    /// Turning with Q / E over the map (degrees a second).
    pub turn_speed: f32,
    /// Looking round (right button) and orbiting (Alt, middle button in OMSI 2's controls).
    pub look: f32,
    pub orbit: f32,
    pub pan: f32,
    /// The wheel, pinch and Alt + right drag.
    pub zoom: f32,
    pub invert_look_y: bool,
    pub invert_orbit_y: bool,
    pub invert_wheel: bool,
    /// Flying builds its speed up the longer the keys are held.
    pub accelerate: bool,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { fly_speed: 30.0, move_speed: 1.0, fast: 3.0, turn_speed: 90.0, look: 1.0, orbit: 1.0, pan: 1.0, zoom: 1.0, invert_look_y: false, invert_orbit_y: false, invert_wheel: false, accelerate: true }
    }
}

impl Tuning {
    /// Brings values read from the settings back into what the camera can use.
    pub fn sane(mut self) -> Tuning {
        let m = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.fly_speed = if self.fly_speed.is_finite() { self.fly_speed.clamp(2.0, 2000.0) } else { 30.0 };
        self.move_speed = m(self.move_speed, 0.1, 10.0, 1.0);
        self.fast = m(self.fast, 1.0, 20.0, 3.0);
        self.turn_speed = m(self.turn_speed, 10.0, 360.0, 90.0);
        self.look = m(self.look, 0.1, 5.0, 1.0);
        self.orbit = m(self.orbit, 0.1, 5.0, 1.0);
        self.pan = m(self.pan, 0.1, 5.0, 1.0);
        self.zoom = m(self.zoom, 0.1, 5.0, 1.0);
        self
    }
}

#[derive(Debug, Clone)]
pub struct Camera {
    /// The point looked at (world).
    pub target: DVec3,
    /// Heading of the view (degrees, 0 = north, clockwise).
    pub yaw: f64,
    /// Down angle (degrees, -85..89.5: below 0 the view looks up).
    pub pitch: f64,
    pub distance: f64,
    pub fov_y: f32,
    /// The plan view: looking straight down, north up; nothing tilts or turns it.
    pub plan: bool,
    /// Flying speed with the right button held (m/s).
    pub fly_speed: f64,
    /// The speeds and sensitivities chosen in the settings.
    pub tune: Tuning,
    /// Smoothed state the view follows.
    smooth_target: DVec3,
    smooth_distance: f64,
    smooth_yaw: f64,
    smooth_pitch: f64,
}

impl Default for Camera {
    fn default() -> Self {
        let t = DVec3::new(150.0, 150.0, 0.0);
        Camera { target: t, yaw: 20.0, pitch: 45.0, distance: 260.0, fov_y: 50f32.to_radians(), plan: false, fly_speed: 30.0, tune: Tuning::default(), smooth_target: t, smooth_distance: 260.0, smooth_yaw: 20.0, smooth_pitch: 45.0 }
    }
}

impl Camera {
    pub fn jump_to(&mut self, target: DVec3, distance: f64) {
        self.target = target;
        self.distance = distance;
        self.smooth_target = target;
        self.smooth_distance = distance;
    }

    /// Follow the wanted view at once (a drag of the mouse must not trail behind it).
    pub fn snap(&mut self) {
        self.smooth_target = self.target;
        self.smooth_distance = self.distance;
        self.smooth_yaw = self.yaw;
        self.smooth_pitch = self.pitch;
    }

    /// Move the smoothed view towards the wanted one.
    pub fn update(&mut self, dt: f64) {
        let k = 1.0 - (-dt * 18.0).exp();
        self.smooth_target += (self.target - self.smooth_target) * k;
        self.smooth_distance += (self.distance - self.smooth_distance) * k;
        let dy = (self.yaw - self.smooth_yaw + 540.0).rem_euclid(360.0) - 180.0;
        self.smooth_yaw += dy * k;
        self.smooth_pitch += (self.pitch - self.smooth_pitch) * k;
    }

    pub fn is_moving(&self) -> bool {
        (self.target - self.smooth_target).length() > 0.01 || (self.distance - self.smooth_distance).abs() > 0.01 || ((self.yaw - self.smooth_yaw + 540.0).rem_euclid(360.0) - 180.0).abs() > 0.01 || (self.pitch - self.smooth_pitch).abs() > 0.01
    }

    /// From the pivot towards the eye, for a heading and down angle.
    fn back(yaw: f64, pitch: f64) -> DVec3 {
        let h = yaw.to_radians();
        let p = pitch.to_radians();
        DVec3::new(-h.sin() * p.cos(), -h.cos() * p.cos(), p.sin())
    }

    /// The pivot and distance as shown this frame (smoothed).
    pub fn smooth_target_now(&self) -> DVec3 {
        self.smooth_target
    }

    pub fn smooth_distance_now(&self) -> f64 {
        self.smooth_distance
    }

    /// The eye position (world).
    pub fn eye(&self) -> DVec3 {
        self.smooth_target + Self::back(self.smooth_yaw, self.smooth_pitch) * self.smooth_distance
    }

    /// Where the eye is going to be (the wanted view, not the smoothed one).
    pub fn wanted_eye(&self) -> DVec3 {
        self.target + Self::back(self.yaw, self.pitch) * self.distance
    }

    /// The way the view looks, and its right and up (the wanted view).
    pub fn axes(&self) -> (DVec3, DVec3, DVec3) {
        let f = -Self::back(self.yaw, self.pitch);
        let r = f.cross(DVec3::Z).normalize_or(DVec3::X);
        (f, r, r.cross(f))
    }

    /// Look round from where the eye is (right button held): the eye stays, the pivot
    /// swings round it.
    pub fn look_pixels(&mut self, dx: f32, dy: f32) {
        if self.plan {
            return;
        }
        let eye = self.wanted_eye();
        let k = 0.25 * self.tune.look as f64;
        self.yaw += dx as f64 * k;
        self.pitch = (self.pitch + dy as f64 * k * if self.tune.invert_look_y { -1.0 } else { 1.0 }).clamp(-85.0, 89.5);
        self.target = eye - Self::back(self.yaw, self.pitch) * self.distance;
        self.snap();
    }

    /// Fly (right button held): `fwd`, `side`, `up` in -1..1, per second; `fast` with Shift;
    /// `held`: seconds the keys have been held (the speed builds up, as in Unity).
    pub fn fly(&mut self, fwd: f64, side: f64, up: f64, fast: bool, dt: f64, held: f64) {
        let (f, r, _) = self.axes();
        let accel = if self.tune.accelerate { 1.0 + (held * 1.5).min(3.0) } else { 1.0 };
        let m = (f * fwd + r * side + DVec3::Z * up) * self.fly_speed * accel * if fast { self.tune.fast as f64 } else { 1.0 } * dt;
        self.target += m;
        self.snap();
    }

    /// The wheel with the right button held: faster or slower flying.
    pub fn fly_faster(&mut self, steps: f32) {
        self.fly_speed = (self.fly_speed * 1.25f64.powf(steps as f64)).clamp(2.0, 2000.0);
        self.tune.fly_speed = self.fly_speed;
    }

    /// Turn round a new pivot from here on (Alt + left button: the point under the cursor),
    /// the eye staying where it is.
    pub fn set_pivot(&mut self, p: DVec3) {
        if self.plan {
            return;
        }
        let eye = self.wanted_eye();
        let d = eye - p;
        if d.length() < 0.5 {
            return;
        }
        self.distance = d.length();
        self.pitch = (d.z / self.distance).clamp(-1.0, 1.0).asin().to_degrees().clamp(-85.0, 89.5);
        // (a heading pointing from the eye to the pivot; kept near the old one, unwrapped)
        let h = (-d.x).atan2(-d.y).to_degrees();
        self.yaw += (h - self.yaw + 540.0).rem_euclid(360.0) - 180.0;
        self.target = p;
        self.snap();
    }

    /// Keep the eye above the ground under it.
    pub fn keep_above(&mut self, ground: f64) {
        let eye = self.wanted_eye();
        let lift = ground + 1.0 - eye.z;
        if lift > 0.0 {
            self.target.z += lift;
            // (shown at once: easing up while the view turns would shake it)
            self.snap();
        }
    }

    /// Render origin: everything is drawn relative to it, so f32 keeps its precision far
    /// from the map's origin.
    pub fn origin(&self) -> DVec3 {
        let t = self.smooth_target;
        DVec3::new((t.x / 64.0).round() * 64.0, (t.y / 64.0).round() * 64.0, 0.0)
    }

    pub fn view(&self) -> Mat4 {
        let o = self.origin();
        let eye = (self.eye() - o).as_vec3();
        let target = (self.smooth_target - o).as_vec3();
        Mat4::look_at_rh(eye, target, Vec3::Z)
    }

    pub fn near_far(&self) -> (f32, f32) {
        let d = self.smooth_distance as f32;
        ((d * 0.002).clamp(0.05, 2.0), (d * 30.0).clamp(3000.0, 40000.0))
    }

    /// Reversed depth with the far plane at infinity: 1 at the near plane, towards 0 far
    /// away, so a float depth buffer keeps its precision where it is needed (no flicker
    /// between a road and the ground under it).
    pub fn proj(&self, aspect: f32) -> Mat4 {
        let (n, _) = self.near_far();
        Mat4::perspective_infinite_reverse_rh(self.fov_y, aspect, n)
    }

    /// The ray under a screen point (pixels, origin top left): world origin and direction.
    pub fn ray(&self, px: f32, py: f32, w: f32, h: f32) -> (DVec3, DVec3) {
        let vp = self.proj(w / h) * self.view();
        let inv = vp.inverse();
        let x = px / w * 2.0 - 1.0;
        let y = 1.0 - py / h * 2.0;
        let a = inv * Vec4::new(x, y, 1.0, 1.0);
        let b = inv * Vec4::new(x, y, 0.1, 1.0);
        let a = a.truncate() / a.w;
        let b = b.truncate() / b.w;
        let o = self.origin();
        (a.as_dvec3() + o, (b - a).normalize().as_dvec3())
    }

    /// Pan by screen pixels (middle button, two fingers on a trackpad, dragging the empty
    /// ground): the view slides across the screen, the point grabbed staying under the cursor
    /// at the pivot's distance. Looking down steeply it slides over the ground.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32, h: f32) {
        let scale = self.distance * (self.fov_y as f64 / 2.0).tan() * 2.0 / h as f64 * self.tune.pan as f64;
        let (f, r, u) = self.axes();
        // (over the ground when the view looks down: forward along it, so the map follows the
        // hand; looking level, up and down)
        let ground_f = DVec3::new(f.x, f.y, 0.0).normalize_or(DVec3::Y);
        let down = self.pitch.to_radians().sin().clamp(0.0, 1.0);
        let vertical = u * (1.0 - down) + ground_f * down / down.max(0.3);
        let m = -r * dx as f64 * scale + vertical * dy as f64 * scale;
        self.target += m;
        self.snap();
    }

    /// Drag the map by a point of it: `anchor` (world) is put back under the ray of the
    /// cursor, sliding over the level of the anchor, so the ground grabbed stays exactly
    /// under the hand however far or tilted the view is.
    pub fn grab(&mut self, anchor: DVec3, o: DVec3, d: DVec3) {
        // (a ray nearly level with the anchor's plane would throw the map to the horizon)
        if d.z.abs() < 0.02 || (anchor.z - o.z) / d.z <= 0.0 {
            return;
        }
        let t = (anchor.z - o.z) / d.z;
        let hit = o + d * t;
        let m = anchor - hit;
        // (far towards the horizon one pixel is kilometres: keep a step sane)
        let m = DVec2::new(m.x, m.y).clamp_length_max(self.distance * 2.0 + 50.0);
        self.target.x += m.x;
        self.target.y += m.y;
        self.snap();
    }

    /// Pan in the screen's plane by pixels (the middle button, as in Unity: what is under
    /// the cursor at the pivot's distance follows the hand).
    pub fn pan_screen(&mut self, dx: f32, dy: f32, h: f32) {
        let scale = self.distance * (self.fov_y as f64 / 2.0).tan() * 2.0 / h as f64 * self.tune.pan as f64;
        let (_, r, u) = self.axes();
        self.target += (-r * dx as f64 + u * dy as f64) * scale;
        self.snap();
    }

    /// Dolly towards the pivot (the wheel, Alt + right drag), at once.
    pub fn dolly(&mut self, steps: f32) {
        let steps = self.zoom_steps(steps);
        self.distance = (self.distance * (0.85f64).powf(steps as f64)).clamp(0.5, 8000.0);
        self.snap();
    }

    /// Orbit round the pivot by screen pixels (Alt + left button, Option + two fingers).
    pub fn orbit_pixels(&mut self, dx: f32, dy: f32) {
        if self.plan {
            return;
        }
        // (yaw is kept unwrapped so the smoothing never spins the long way round)
        let k = self.tune.orbit as f64;
        self.yaw += dx as f64 * 0.35 * k;
        self.pitch = (self.pitch + dy as f64 * 0.3 * k * if self.tune.invert_orbit_y { -1.0 } else { 1.0 }).clamp(-85.0, 89.5);
        self.snap();
    }

    /// Move towards a point (the wheel: towards the cursor), by wheel steps.
    pub fn zoom(&mut self, steps: f32, toward: Option<DVec3>) {
        let steps = self.zoom_steps(steps);
        let old = self.distance;
        self.distance = (self.distance * (0.85f64).powf(steps as f64)).clamp(1.0, 8000.0);
        if let Some(p) = toward {
            let k = 1.0 - self.distance / old;
            self.target += (p - self.target) * k;
        }
    }

    /// Wheel steps as the settings have them: faster or slower, the other way round.
    fn zoom_steps(&self, steps: f32) -> f32 {
        steps * self.tune.zoom * if self.tune.invert_wheel { -1.0 } else { 1.0 }
    }

    /// Keyboard movement without the right button: over the map, `fwd`, `side` in -1..1,
    /// `turn` in -1..1, per second.
    pub fn drive(&mut self, fwd: f64, side: f64, turn: f64, fast: bool, dt: f64) {
        let h = self.yaw.to_radians();
        let f = DVec2::new(h.sin(), h.cos());
        let r = DVec2::new(f.y, -f.x);
        let speed = (self.distance * 1.2).max(30.0) * self.tune.move_speed as f64 * if fast { self.tune.fast as f64 } else { 1.0 };
        let m = (f * fwd + r * side) * speed * dt;
        self.target.x += m.x;
        self.target.y += m.y;
        self.yaw += turn * self.tune.turn_speed as f64 * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_round_and_a_new_pivot_keep_the_eye() {
        let mut c = Camera::default();
        c.snap();
        let eye = c.wanted_eye();
        c.look_pixels(120.0, -40.0);
        assert!((c.wanted_eye() - eye).length() < 1e-6);
        c.set_pivot(DVec3::new(10.0, 300.0, 4.0));
        assert!((c.wanted_eye() - eye).length() < 1e-6, "{:?} vs {eye:?}", c.wanted_eye());
        assert!((c.target - DVec3::new(10.0, 300.0, 4.0)).length() < 1e-9);
    }

    #[test]
    fn flying_forward_goes_where_the_view_looks() {
        let mut c = Camera::default();
        c.yaw = 0.0;
        c.pitch = 0.0;
        c.snap();
        let eye = c.wanted_eye();
        c.fly(1.0, 0.0, 0.0, false, 1.0, 0.0);
        let d = c.wanted_eye() - eye;
        assert!(d.y > 29.0 && d.x.abs() < 1e-6 && d.z.abs() < 1e-6, "{d:?}");
    }

    #[test]
    fn the_settings_change_the_speed_and_the_direction() {
        let mut c = Camera::default();
        c.yaw = 0.0;
        c.pitch = 0.0;
        c.snap();
        c.tune.fly_speed = 60.0;
        c.fly_speed = 60.0;
        c.tune.fast = 2.0;
        let eye = c.wanted_eye();
        c.fly(1.0, 0.0, 0.0, true, 1.0, 0.0);
        assert!(((c.wanted_eye() - eye).y - 120.0).abs() < 1e-6);
        let d = c.distance;
        c.zoom(1.0, None);
        let in_d = c.distance;
        c.distance = d;
        c.tune.invert_wheel = true;
        c.zoom(1.0, None);
        assert!(in_d < d && c.distance > d);
        // (the settings file may hold anything)
        let t = Tuning { look: f32::NAN, fast: 0.0, fly_speed: 1e9, ..Tuning::default() }.sane();
        assert_eq!((t.look, t.fast, t.fly_speed), (1.0, 1.0, 2000.0));
    }

    #[test]
    fn a_grabbed_point_stays_under_the_cursor() {
        let mut c = Camera::default();
        c.pitch = 35.0;
        c.snap();
        let (w, h) = (1200.0, 800.0);
        let hit = |c: &Camera, px: f32, py: f32| {
            let (o, d) = c.ray(px, py, w, h);
            o + d * (-o.z / d.z)
        };
        let anchor = hit(&c, 500.0, 500.0);
        // the hand moves the cursor in steps, as a drag does
        for i in 1..=10 {
            let (o, d) = c.ray(500.0 + i as f32 * 30.0, 500.0 - i as f32 * 12.0, w, h);
            c.grab(anchor, o, d);
        }
        let now = hit(&c, 800.0, 380.0);
        assert!((now - anchor).truncate().length() < 0.05, "{now:?} vs {anchor:?}");
    }
}
