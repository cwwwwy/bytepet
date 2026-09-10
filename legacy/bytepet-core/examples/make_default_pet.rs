//! Deterministic generator for BytePet's bundled default pet ("ByteBot").
//!
//! ```text
//! cargo run -p bytepet-core --example make_default_pet
//! cargo run -p bytepet-core --example make_default_pet -- \
//!     --preview /tmp/default-pet-preview.png --zoom /tmp/default-pet-zoom.png
//! ```
//!
//! The pet is self-authored, license-clean pixel art drawn on a 48x52 logical
//! grid and upscaled x4 with nearest-neighbour into the Codex V1 atlas: 8
//! columns x 9 rows of 192x208 px cells (1536x1872 px RGBA, transparent
//! background). Cells past each row's last used column stay fully transparent.
//!
//! Everything is hand-authored arithmetic (no RNG, no assets, no extra
//! dependencies): re-running produces byte-identical PNG bytes for the same
//! `image` version. After writing, the PNG is re-opened and self-checked:
//! geometry must be 1536x1872, every used cell must contain opaque pixels and
//! every unused cell must be fully transparent.

use std::path::{Path, PathBuf};
use std::time::Instant;

use image::{imageops, Rgba, RgbaImage};

// ---------------------------------------------------------------------------
// Palette: 8 colours + transparency. Chosen to stay readable on white and on
// dark wallpapers (mint body + near-black teal outline + white highlights).
// ---------------------------------------------------------------------------
const OUTLINE: Rgba<u8> = Rgba([0x14, 0x48, 0x3f, 0xff]); // deep teal outline
const BODY: Rgba<u8> = Rgba([0x3d, 0xd6, 0xc0, 0xff]); // mint body
const BODY_SHADE: Rgba<u8> = Rgba([0x27, 0xa8, 0x96, 0xff]); // shading / desaturated
const BELLY: Rgba<u8> = Rgba([0xf6, 0xe8, 0xc9, 0xff]); // cream belly panel
const EYE: Rgba<u8> = Rgba([0x16, 0x23, 0x3c, 0xff]); // dark navy eyes/mouth
const WHITE: Rgba<u8> = Rgba([0xff, 0xff, 0xff, 0xff]); // highlight / tear / spark
const AMBER: Rgba<u8> = Rgba([0xff, 0xb3, 0x40, 0xff]); // antenna tip / glow
const BLUSH: Rgba<u8> = Rgba([0xf2, 0x8c, 0x96, 0xff]); // soft blush

const GRID_W: i32 = 48;
const GRID_H: i32 = 52;
const SCALE: u32 = 4;
const CELL_W: u32 = 192;
const CELL_H: u32 = 208;
const COLS: usize = 8;
const ROWS: usize = 9;

/// Frames actually used per row (columns `0..used`).
const ROW_FRAMES: [usize; ROWS] = [6, 8, 8, 4, 5, 8, 6, 6, 6];

/// Baseline (bottom edge of the feet) in logical pixels: 50 * 4 = 200 of 208.
const BASELINE: f32 = 50.0;
/// Body bottom edge when standing: 44.5 * 4 = 178 of 208. The gap down to the
/// baseline is the visible stubby leg + foot.
const BODY_BOTTOM: f32 = 44.5;
const BASE_RX: f32 = 13.0;
const BASE_RY: f32 = 14.5;

// ---------------------------------------------------------------------------
// Tiny raster helpers over a logical grid.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Canvas {
    w: i32,
    h: i32,
    px: Vec<Rgba<u8>>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    fn new() -> Self {
        Self {
            w: GRID_W,
            h: GRID_H,
            px: vec![Rgba([0, 0, 0, 0]); (GRID_W * GRID_H) as usize],
        }
    }

    fn idx(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            None
        } else {
            Some((y * self.w + x) as usize)
        }
    }

    /// Set one logical pixel (out-of-bounds writes are dropped).
    fn px(&mut self, x: i32, y: i32, c: Rgba<u8>) {
        if let Some(i) = self.idx(x, y) {
            self.px[i] = c;
        }
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.px(x, y, c);
            }
        }
    }

    /// Rect with a 1px outline of `OUTLINE` around it.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
        self.fill_rect(x0 - 1, y0 - 1, x1 + 1, y1 + 1, OUTLINE);
        self.fill_rect(x0, y0, x1, y1, c);
    }

    /// Filled ellipse. `shear` shifts each scanline horizontally, which is how
    /// the character leans/tilts without needing a full rotation.
    fn fill_ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, shear: f32, c: Rgba<u8>) {
        if rx <= 0.0 || ry <= 0.0 {
            return;
        }
        let y0 = (cy - ry).floor() as i32;
        let y1 = (cy + ry).ceil() as i32;
        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            let dy = (py - cy) / ry;
            if dy.abs() > 1.0 {
                continue;
            }
            let half = rx * (1.0 - dy * dy).max(0.0).sqrt();
            let cxi = cx + shear * (cy - py);
            let x0 = (cxi - half).round() as i32;
            let x1 = (cxi + half).round() as i32;
            for x in x0..=x1 {
                self.px(x, y, c);
            }
        }
    }

    /// Filled ellipse with a 1px outline of `OUTLINE` around it.
    ///
    /// The outline ellipse is inflated a full pixel horizontally but only 0.6px
    /// vertically, which keeps the side border exactly 1px while avoiding a
    /// stray outline nub at the top/bottom pole of the shape.
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, shear: f32, c: Rgba<u8>) {
        self.fill_ellipse(cx, cy, rx + 1.0, ry + 0.6, shear, OUTLINE);
        self.fill_ellipse(cx, cy, rx, ry, shear, c);
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, c: Rgba<u8>) {
        self.ellipse(cx, cy, r, r, 0.0, c);
    }

    /// 1px Bresenham line.
    fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            self.px(x, y, c);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn thick_line_color(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Rgba<u8>) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let steps = (len * 2.0).ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.fill_ellipse(x0 + dx * t, y0 + dy * t, r, r, 0.0, c);
        }
    }

    /// Stubby limb segment with a 1px outline.
    fn thick_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, c: Rgba<u8>) {
        self.thick_line_color(x0, y0, x1, y1, r + 1.0, OUTLINE);
        self.thick_line_color(x0, y0, x1, y1, r, c);
    }

    /// Add a 1px `OUTLINE` border around every opaque pixel inside the rect.
    /// Existing outlines are left untouched, so this is safe to call over
    /// pixels that were already outlined.
    fn outline(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let before = self.px.clone();
        let (w, h) = (self.w, self.h);
        let opaque = |x: i32, y: i32| -> bool {
            if x < 0 || y < 0 || x >= w || y >= h {
                false
            } else {
                before[(y * w + x) as usize].0[3] != 0
            }
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                if opaque(x, y) {
                    continue;
                }
                if opaque(x - 1, y) || opaque(x + 1, y) || opaque(x, y - 1) || opaque(x, y + 1) {
                    self.px(x, y, OUTLINE);
                }
            }
        }
    }

    /// Mirror around the logical centre x = 24 (the character is symmetric).
    fn mirror_h(&self) -> Canvas {
        let mut out = Canvas::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let c = self.px[(y * self.w + x) as usize];
                if c.0[3] == 0 {
                    continue;
                }
                out.px(self.w - x, y, c);
            }
        }
        out
    }

    /// Nearest-neighbour upscale of the logical grid.
    fn upscale_to(&self, scale: u32) -> RgbaImage {
        let mut img = RgbaImage::new(self.w as u32 * scale, self.h as u32 * scale);
        for y in 0..self.h {
            for x in 0..self.w {
                let c = self.px[(y * self.w + x) as usize];
                if c.0[3] == 0 {
                    continue;
                }
                let bx = x as u32 * scale;
                let by = y as u32 * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        img.put_pixel(bx + dx, by + dy, c);
                    }
                }
            }
        }
        img
    }
}

// ---------------------------------------------------------------------------
// Pose model. Every frame is the same character with different parameters, so
// the sprite reads as one being instead of a redraw per frame.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Eyes {
    Open,
    Blink,
    Happy,
    Squeezed,
    Squint,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mouth {
    Smile,
    Flat,
    Frown,
    Open,
}

#[derive(Clone, Copy)]
struct Pose {
    /// Lifts the whole character off the ground (jumping).
    whole_dy: i32,
    /// Body bob relative to the ground; legs compensate so feet stay planted.
    body_dy: i32,
    /// Vertical scale of the body (breathing / squash / stretch).
    squash: f32,
    /// Horizontal scale of the body (squash widens).
    width: f32,
    /// Lean: positive leans the top of the body to the right.
    shear: f32,
    eyes: Eyes,
    eye_dx: i32,
    eye_dy: i32,
    mouth: Mouth,
    blush: bool,
    antenna_bend: f32,
    antenna_droop: i32,
    antenna_pulse: bool,
    arm_l: (f32, f32),
    arm_r: (f32, f32),
    arms_front: bool,
    leg_l: (i32, i32),
    leg_r: (i32, i32),
    body_color: Rgba<u8>,
    tear: Option<(f32, f32)>,
}

impl Pose {
    fn base() -> Self {
        Self {
            whole_dy: 0,
            body_dy: 0,
            squash: 1.0,
            width: 1.0,
            shear: 0.0,
            eyes: Eyes::Open,
            eye_dx: 0,
            eye_dy: 0,
            mouth: Mouth::Smile,
            blush: true,
            antenna_bend: 0.0,
            antenna_droop: 0,
            antenna_pulse: false,
            arm_l: (8.0, 38.0),
            arm_r: (40.0, 38.0),
            arms_front: false,
            leg_l: (0, 0),
            leg_r: (0, 0),
            body_color: BODY,
            tear: None,
        }
    }
}

struct Geom {
    cy: f32,
    rx: f32,
    ry: f32,
    top: f32,
}

fn geometry(p: &Pose) -> Geom {
    let bottom = BODY_BOTTOM + p.body_dy as f32 + p.whole_dy as f32;
    let ry = BASE_RY * p.squash;
    let rx = BASE_RX * p.width;
    let cy = bottom - ry;
    Geom {
        cy,
        rx,
        ry,
        top: cy - ry,
    }
}

// ---------------------------------------------------------------------------
// Drawing the character.
// ---------------------------------------------------------------------------

fn draw_leg(c: &mut Canvas, p: &Pose, base_x: f32, leg: (i32, i32)) {
    let (dx, lift) = leg;
    let x = base_x + dx as f32;
    let top = BODY_BOTTOM + p.body_dy as f32 + p.whole_dy as f32 - 2.5;
    let by = (BASELINE + p.whole_dy as f32 - lift as f32).round() as i32;
    if (by as f32) <= top {
        return;
    }
    c.thick_line(x, top, x, (by - 3) as f32, 1.0, p.body_color);
    let bx = x.round() as i32;
    // Foot: 5px wide, two rows tall, bottom edge exactly on the baseline.
    c.fill_rect(bx - 3, by - 4, bx + 3, by - 1, OUTLINE);
    c.fill_rect(bx - 2, by - 3, bx + 2, by - 2, p.body_color);
}

fn draw_arm(c: &mut Canvas, p: &Pose, side: f32, hand: (f32, f32), g: &Geom) {
    let sy = g.cy - 1.0;
    let sx = 24.0 + side * (g.rx - 2.0) + p.shear;
    c.thick_line(sx, sy, hand.0, hand.1, 0.9, p.body_color);
    c.disc(hand.0, hand.1, 1.5, p.body_color);
}

fn draw_eye(c: &mut Canvas, x: f32, y: f32, mode: Eyes, side: f32) {
    let xi = x.round() as i32;
    let yi = y.round() as i32;
    match mode {
        Eyes::Open => {
            c.fill_ellipse(x, y, 2.6, 3.2, 0.0, EYE);
            c.px(xi - 1, yi - 2, WHITE);
        }
        Eyes::Squint => {
            c.fill_ellipse(x, y + 0.5, 2.6, 1.9, 0.0, EYE);
            c.px(xi - 1, yi, WHITE);
        }
        Eyes::Blink => {
            c.fill_rect(xi - 2, yi, xi + 1, yi, EYE);
        }
        Eyes::Happy => {
            c.px(xi - 2, yi + 1, EYE);
            c.px(xi - 1, yi, EYE);
            c.px(xi, yi, EYE);
            c.px(xi + 1, yi, EYE);
            c.px(xi + 2, yi + 1, EYE);
        }
        Eyes::Squeezed => {
            if side < 0.0 {
                // ">" squeezed toward the centre.
                c.px(xi - 1, yi - 1, EYE);
                c.px(xi, yi, EYE);
                c.px(xi - 1, yi + 1, EYE);
            } else {
                // "<"
                c.px(xi + 1, yi - 1, EYE);
                c.px(xi, yi, EYE);
                c.px(xi + 1, yi + 1, EYE);
            }
        }
    }
}

fn draw_mouth(c: &mut Canvas, x: f32, y: f32, mode: Mouth) {
    let xi = x.round() as i32;
    let yi = y.round() as i32;
    match mode {
        Mouth::Smile => {
            c.px(xi - 1, yi, EYE);
            c.px(xi, yi + 1, EYE);
            c.px(xi + 1, yi, EYE);
        }
        Mouth::Flat => {
            c.px(xi - 1, yi, EYE);
            c.px(xi, yi, EYE);
            c.px(xi + 1, yi, EYE);
        }
        Mouth::Frown => {
            c.px(xi - 1, yi + 1, EYE);
            c.px(xi, yi, EYE);
            c.px(xi + 1, yi + 1, EYE);
        }
        Mouth::Open => {
            c.fill_ellipse(x, y + 0.5, 1.7, 1.5, 0.0, EYE);
            c.px(xi, yi + 1, BLUSH);
        }
    }
}

fn draw_bot(c: &mut Canvas, p: &Pose) {
    let g = geometry(p);

    // Limbs first so the body covers their roots.
    draw_leg(c, p, 19.0, p.leg_l);
    draw_leg(c, p, 29.0, p.leg_r);
    if !p.arms_front {
        draw_arm(c, p, -1.0, p.arm_l, &g);
        draw_arm(c, p, 1.0, p.arm_r, &g);
    }

    // Body + belly panel.
    c.ellipse(24.0, g.cy, g.rx, g.ry, p.shear, p.body_color);
    let b_cy = g.cy + 8.0 * p.squash;
    let b_cx = 24.0 + p.shear * (g.cy - b_cy);
    c.ellipse(b_cx, b_cy, 7.0 * p.width, 3.4 * p.squash, p.shear, BELLY);
    let bxi = b_cx.round() as i32;
    let byi = b_cy.round() as i32;
    c.px(bxi - 2, byi, BODY_SHADE);
    c.px(bxi + 2, byi, BODY_SHADE);

    // Face. Gaze shifts both eyes together (never their separation).
    let eye_y = g.cy - 1.5 + p.eye_dy as f32;
    let eye_off = 4.6 * p.width;
    let gaze = p.eye_dx as f32 + p.shear * (g.cy - eye_y);
    draw_eye(c, 24.0 - eye_off + gaze, eye_y, p.eyes, -1.0);
    draw_eye(c, 24.0 + eye_off + gaze, eye_y, p.eyes, 1.0);

    if p.blush {
        let by = (g.cy + 4.5).round() as i32;
        let bx = (10.5 * p.width).round() as i32;
        c.fill_rect(24 - bx, by, 24 - bx + 2, by, BLUSH);
        c.fill_rect(24 + bx - 2, by, 24 + bx, by, BLUSH);
    }

    let m_cy = g.cy + 3.0;
    draw_mouth(c, 24.0 + p.shear * (g.cy - m_cy), m_cy, p.mouth);

    // Antenna: stalk from the crown to a glowing amber tip.
    let base_x = 24.0 + p.shear * g.ry;
    let tip_x = 24.0 + p.antenna_bend;
    let tip_y = g.top - 5.0 + p.antenna_droop as f32;
    c.thick_line(base_x, g.top + 1.0, tip_x, tip_y, 0.5, p.body_color);
    c.disc(tip_x, tip_y - 1.0, 1.8, AMBER);
    if p.antenna_pulse {
        let ox = tip_x.round() as i32;
        let oy = (tip_y - 1.0).round() as i32;
        c.px(ox, oy - 3, AMBER);
        c.px(ox, oy + 3, AMBER);
        c.px(ox - 3, oy, AMBER);
        c.px(ox + 3, oy, AMBER);
    }

    if p.arms_front {
        draw_arm(c, p, -1.0, p.arm_l, &g);
        draw_arm(c, p, 1.0, p.arm_r, &g);
    }

    if let Some((tx, ty)) = p.tear {
        c.px(tx.round() as i32, ty.round() as i32, WHITE);
        c.px(tx.round() as i32, ty.round() as i32 + 1, WHITE);
    }
}

// ---------------------------------------------------------------------------
// Row 0 - idle: breathing bob, blink on frames 3-4, antenna tip pulses.
// ---------------------------------------------------------------------------
fn idle_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    match i {
        0 => {}
        1 => {
            p.body_dy = -1;
            p.squash = 1.02;
            p.antenna_pulse = true;
            p.arm_l.1 -= 1.0;
            p.arm_r.1 -= 1.0;
        }
        2 => {}
        3 => {
            p.eyes = Eyes::Blink;
            p.mouth = Mouth::Flat;
        }
        4 => {
            p.body_dy = -1;
            p.squash = 1.02;
            p.eyes = Eyes::Blink;
            p.antenna_pulse = true;
            p.arm_l.1 -= 1.0;
            p.arm_r.1 -= 1.0;
        }
        5 => {}
        _ => {}
    }
    p
}

// ---------------------------------------------------------------------------
// Rows 1/2 - running right/left: 8-phase leg cycle, double bob, forward lean,
// antenna trailing. Row 2 is the horizontal mirror of row 1.
// ---------------------------------------------------------------------------
const RUN_BOB: [i32; 8] = [0, -1, -2, -1, 0, -1, -2, -1];
/// One leg's 8-phase cycle: forward contact -> stance -> push-off -> lift ->
/// swing through -> reach. The other leg is this shifted by four phases.
const RUN_LEG: [(i32, i32); 8] = [
    (1, 0),
    (0, 0),
    (-1, 0),
    (-1, 1),
    (-1, 2),
    (0, 2),
    (1, 1),
    (1, 0),
];

fn run_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    let leg_a = RUN_LEG[i];
    let leg_b = RUN_LEG[(i + 4) % 8];
    p.body_dy = RUN_BOB[i];
    p.shear = 0.10;
    p.leg_l = leg_a;
    p.leg_r = leg_b;
    p.arm_l = (8.0 - leg_a.0 as f32, 38.0 - leg_a.1 as f32 * 0.8);
    p.arm_r = (40.0 - leg_b.0 as f32, 38.0 - leg_b.1 as f32 * 0.8);
    p.eye_dx = 1;
    p.antenna_bend = -2.0 + 0.5 * p.body_dy as f32;
    p.antenna_droop = 1;
    p.mouth = Mouth::Smile;
    p
}

// ---------------------------------------------------------------------------
// Row 3 - waving: right arm raises and waves out/in, body leans away.
// ---------------------------------------------------------------------------
fn wave_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    match i {
        0 => {}
        1 => {
            p.body_dy = -1;
            p.shear = -0.05;
            p.arm_r = (42.0, 25.0);
            p.antenna_bend = 0.6;
            p.antenna_pulse = true;
            p.mouth = Mouth::Smile;
        }
        2 => {
            p.body_dy = -1;
            p.shear = -0.06;
            p.arm_r = (43.0, 15.0);
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.antenna_bend = 1.2;
            p.antenna_pulse = true;
            p.arm_l = (7.0, 39.0);
        }
        3 => {
            p.body_dy = -1;
            p.shear = -0.04;
            p.arm_r = (35.0, 14.0);
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.antenna_bend = 0.4;
            p.antenna_pulse = true;
            p.arm_l = (7.0, 39.0);
        }
        _ => {}
    }
    p.arms_front = true;
    p
}

// ---------------------------------------------------------------------------
// Row 4 - jumping: anticipation squash, stretch, peak, descent, landing squash.
// ---------------------------------------------------------------------------
fn jump_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    match i {
        0 => {
            p.squash = 0.78;
            p.width = 1.14;
            p.eyes = Eyes::Open;
            p.eye_dy = -1;
            p.mouth = Mouth::Flat;
            p.arm_l = (7.0, 42.0);
            p.arm_r = (41.0, 42.0);
            p.antenna_droop = 1;
            p.antenna_bend = -1.0;
        }
        1 => {
            p.whole_dy = -3;
            p.squash = 1.16;
            p.width = 0.92;
            p.eyes = Eyes::Open;
            p.eye_dy = -1;
            p.mouth = Mouth::Open;
            p.arm_l = (11.0, 20.0);
            p.arm_r = (37.0, 20.0);
            p.leg_l = (0, 1);
            p.leg_r = (0, 1);
            p.antenna_droop = 2;
            p.antenna_bend = -0.5;
        }
        2 => {
            p.whole_dy = -6;
            p.squash = 1.02;
            p.width = 0.95;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (12.0, 18.0);
            p.arm_r = (36.0, 18.0);
            p.leg_l = (-1, 3);
            p.leg_r = (1, 3);
            p.antenna_droop = 2;
            p.antenna_bend = 0.0;
        }
        3 => {
            p.whole_dy = -3;
            p.squash = 1.06;
            p.width = 0.98;
            p.eyes = Eyes::Open;
            p.mouth = Mouth::Open;
            p.arm_l = (5.0, 26.0);
            p.arm_r = (43.0, 26.0);
            p.leg_l = (-1, 0);
            p.leg_r = (1, 0);
            p.antenna_droop = 1;
            p.antenna_bend = -1.0;
        }
        4 => {
            p.squash = 0.74;
            p.width = 1.16;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (6.0, 40.0);
            p.arm_r = (42.0, 40.0);
            p.antenna_droop = 3;
            p.antenna_bend = -2.0;
        }
        _ => {}
    }
    p
}

// ---------------------------------------------------------------------------
// Row 5 - failed: slumps lower, eyes squeezed "><", frown, teardrop, and the
// mint body is swapped for its desaturated shade.
// ---------------------------------------------------------------------------
fn fail_pose(i: usize) -> Pose {
    const DROOP: [i32; 8] = [1, 2, 3, 4, 3, 2, 1, 2];
    const BEND: [f32; 8] = [-0.5, 0.5, 1.0, 1.5, 1.0, 0.5, 0.0, -0.5];
    const SLUMP: [i32; 8] = [1, 1, 1, 1, 1, 0, 0, 1];
    const DEFLATE: [f32; 8] = [0.97, 0.96, 0.95, 0.94, 0.95, 0.96, 0.98, 0.96];
    let mut p = Pose::base();
    p.body_color = BODY_SHADE;
    p.eyes = Eyes::Squeezed;
    p.mouth = Mouth::Frown;
    p.blush = false;
    p.squash = DEFLATE[i];
    p.body_dy = SLUMP[i];
    p.shear = -0.04;
    p.arm_l = (8.0, 41.0);
    p.arm_r = (40.0, 41.0);
    p.antenna_droop = DROOP[i];
    p.antenna_bend = BEND[i];
    // Tear falls from the outer corner of alternating eyes, staying on the
    // teal body so it never disappears against the cream belly panel.
    let tear_y = 34.0 + (i % 4) as f32 * 2.0;
    p.tear = Some(if i < 4 {
        (15.0, tear_y)
    } else {
        (33.0, tear_y)
    });
    p
}

// ---------------------------------------------------------------------------
// Row 6 - waiting: looks left, looks right, tilts, taps a foot.
// ---------------------------------------------------------------------------
fn wait_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    match i {
        0 => {}
        1 => {
            p.eye_dx = -2;
            p.antenna_bend = -1.0;
        }
        2 => {
            p.eye_dx = 2;
            p.antenna_bend = 1.0;
        }
        3 => {
            p.shear = 0.10;
            p.eye_dx = -1;
            p.antenna_bend = -0.5;
            p.antenna_droop = 1;
        }
        4 => {
            p.body_dy = -1;
            p.leg_r = (0, 2);
            p.eye_dy = 1;
            p.arm_r.1 -= 2.0;
            p.antenna_bend = 0.5;
        }
        5 => {
            p.leg_l = (0, 2);
            p.eye_dx = 1;
            p.antenna_pulse = true;
            p.antenna_bend = -0.5;
        }
        _ => {}
    }
    p
}

// ---------------------------------------------------------------------------
// Row 7 - running (working/thinking): leans into a tiny console, types, sparks.
// ---------------------------------------------------------------------------
fn work_pose(i: usize) -> Pose {
    const BOB: [i32; 6] = [0, -1, 0, 0, -1, 0];
    let mut p = Pose::base();
    p.body_dy = BOB[i];
    p.shear = 0.12;
    p.eyes = Eyes::Squint;
    p.eye_dx = 1;
    p.eye_dy = 1;
    p.mouth = Mouth::Smile;
    p.arms_front = true;
    // Left arm rests at the side; the right arm reaches the keyboard and taps.
    p.arm_l = (7.0, 38.0);
    p.arm_r = if i.is_multiple_of(2) {
        (37.0, 46.0)
    } else {
        (36.0, 47.0)
    };
    p.antenna_bend = -0.5;
    p.antenna_droop = 1;
    p
}

fn draw_console(c: &mut Canvas, i: usize) {
    // Code lines grow/shrink per frame so the screen looks like it is being
    // typed on. Shell is the darker mint so it never merges with the cream
    // belly panel; the screen is near-black with amber/white "code".
    const LINE_A: [i32; 6] = [6, 7, 8, 8, 6, 7];
    const LINE_B: [i32; 6] = [4, 5, 4, 6, 7, 5];
    const LINE_C: [i32; 6] = [2, 3, 2, 4, 5, 3];
    c.rect(34, 40, 46, 48, BODY_SHADE);
    c.fill_rect(37, 42, 45, 46, OUTLINE);
    c.fill_rect(38, 43, 38 + LINE_A[i] - 1, 43, WHITE);
    c.fill_rect(38, 45, 38 + LINE_B[i] - 1, 45, AMBER);
    c.fill_rect(38, 47, 38 + LINE_C[i] - 1, 47, WHITE);
    for k in 0..6 {
        c.px(35 + k * 2, 48, BELLY);
    }
}

fn draw_sparks(c: &mut Canvas, i: usize) {
    let spots: &[(i32, i32)] = match i {
        1 => &[(44, 35)],
        2 => &[(43, 34), (46, 36)],
        4 => &[(45, 35)],
        _ => &[],
    };
    for &(x, y) in spots {
        c.px(x, y, WHITE);
        c.px(x - 1, y, AMBER);
        c.px(x + 1, y, AMBER);
        c.px(x, y - 1, AMBER);
        c.px(x, y + 1, AMBER);
    }
}

// ---------------------------------------------------------------------------
// Row 8 - review (done): a check mark draws itself in and the pet does one
// happy bounce.
// ---------------------------------------------------------------------------
fn review_pose(i: usize) -> Pose {
    let mut p = Pose::base();
    match i {
        0 => {}
        1 => {
            p.body_dy = -1;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (10.0, 32.0);
            p.arm_r = (38.0, 32.0);
            p.antenna_pulse = true;
        }
        2 => {
            p.body_dy = -3;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (12.0, 22.0);
            p.arm_r = (36.0, 22.0);
            p.antenna_pulse = true;
            p.antenna_bend = 1.0;
        }
        3 => {
            p.body_dy = -2;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (11.0, 24.0);
            p.arm_r = (37.0, 24.0);
            p.antenna_pulse = true;
        }
        4 => {
            p.squash = 0.94;
            p.width = 1.04;
            p.eyes = Eyes::Happy;
            p.mouth = Mouth::Open;
            p.arm_l = (8.0, 36.0);
            p.arm_r = (40.0, 36.0);
        }
        5 => {}
        _ => {}
    }
    p
}

fn draw_check(c: &mut Canvas, i: usize) {
    // A check mark that draws itself in above the pet's head, clear of the
    // antenna and of the body silhouette during the happy bounce.
    match i {
        0 | 5 => {}
        1 => {
            c.line(31, 8, 34, 11, AMBER);
            c.outline(30, 7, 35, 12);
        }
        2 => {
            c.line(31, 8, 34, 11, AMBER);
            c.line(34, 11, 41, 3, AMBER);
            c.outline(30, 2, 42, 12);
        }
        _ => {
            c.line(31, 8, 34, 11, AMBER);
            c.line(34, 11, 41, 3, AMBER);
            c.outline(30, 2, 42, 12);
            c.px(44, 7, WHITE);
            c.px(43, 7, AMBER);
            c.px(45, 7, AMBER);
            c.px(44, 6, AMBER);
            c.px(44, 8, AMBER);
            c.outline(42, 5, 46, 9);
        }
    }
}

// ---------------------------------------------------------------------------
// Frame + atlas assembly.
// ---------------------------------------------------------------------------

fn render_frame(row: usize, col: usize) -> Canvas {
    let mut c = Canvas::new();
    match row {
        0 => draw_bot(&mut c, &idle_pose(col)),
        1 => draw_bot(&mut c, &run_pose(col)),
        2 => {
            let mut right = Canvas::new();
            draw_bot(&mut right, &run_pose(col));
            c = right.mirror_h();
        }
        3 => draw_bot(&mut c, &wave_pose(col)),
        4 => draw_bot(&mut c, &jump_pose(col)),
        5 => draw_bot(&mut c, &fail_pose(col)),
        6 => draw_bot(&mut c, &wait_pose(col)),
        7 => {
            let mut p = work_pose(col);
            let hands = (p.arm_l, p.arm_r);
            p.arm_l = (7.0, 40.0);
            p.arm_r = (41.0, 40.0);
            p.arms_front = false;
            draw_bot(&mut c, &p);
            draw_console(&mut c, col);
            p.arm_l = hands.0;
            p.arm_r = hands.1;
            p.arms_front = true;
            let g = geometry(&p);
            draw_arm(&mut c, &p, -1.0, p.arm_l, &g);
            draw_arm(&mut c, &p, 1.0, p.arm_r, &g);
            draw_sparks(&mut c, col);
        }
        8 => {
            draw_bot(&mut c, &review_pose(col));
            draw_check(&mut c, col);
        }
        _ => {}
    }
    c
}

fn build_atlas() -> RgbaImage {
    let mut atlas = RgbaImage::new(CELL_W * COLS as u32, CELL_H * ROWS as u32);
    for (row, used) in ROW_FRAMES.iter().enumerate() {
        for col in 0..*used {
            let cell = render_frame(row, col).upscale_to(SCALE);
            imageops::replace(
                &mut atlas,
                &cell,
                (col as u32 * CELL_W) as i64,
                (row as u32 * CELL_H) as i64,
            );
        }
    }
    atlas
}

// ---------------------------------------------------------------------------
// Contact sheets for human review.
// ---------------------------------------------------------------------------

fn checkerboard(w: u32, h: u32, size: u32) -> RgbaImage {
    let light = Rgba([0xf2, 0xf2, 0xf2, 0xff]);
    let dark = Rgba([0x3a, 0x3a, 0x3a, 0xff]);
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let c = if ((x / size) + (y / size)).is_multiple_of(2) {
                light
            } else {
                dark
            };
            img.put_pixel(x, y, c);
        }
    }
    img
}

fn paste(dst: &mut RgbaImage, src: &RgbaImage, ox: u32, oy: u32) {
    for y in 0..src.height() {
        for x in 0..src.width() {
            let p = *src.get_pixel(x, y);
            if p.0[3] != 0 {
                dst.put_pixel(ox + x, oy + y, p);
            }
        }
    }
}

/// One row per state, every frame, at `scale` logical pixels per cell pixel.
fn contact_sheet(scale: u32, max_frames: usize) -> RgbaImage {
    let cell_w = GRID_W as u32 * scale;
    let cell_h = GRID_H as u32 * scale;
    let cols = max_frames as u32;
    let mut img = checkerboard(cell_w * cols, cell_h * ROWS as u32, 8 * scale);
    for (row, used) in ROW_FRAMES.iter().enumerate() {
        let frames = (*used).min(max_frames);
        for col in 0..frames {
            let cell = render_frame(row, col).upscale_to(scale);
            paste(&mut img, &cell, col as u32 * cell_w, row as u32 * cell_h);
        }
        // Row separator.
        let y = (row as u32 + 1) * cell_h - 1;
        if row + 1 < ROWS {
            for x in 0..img.width() {
                img.put_pixel(x, y, Rgba([0x80, 0x80, 0x80, 0xff]));
            }
        }
    }
    img
}

// ---------------------------------------------------------------------------
// Self-check.
// ---------------------------------------------------------------------------

fn verify(path: &Path) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot re-open {}: {e}", path.display()))?
        .to_rgba8();
    let expect = (CELL_W * COLS as u32, CELL_H * ROWS as u32);
    if (img.width(), img.height()) != expect {
        return Err(format!(
            "spritesheet is {}x{} px but must be {}x{}",
            img.width(),
            img.height(),
            expect.0,
            expect.1
        ));
    }
    for (row, used) in ROW_FRAMES.iter().enumerate() {
        for col in *used..COLS {
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    let px = img.get_pixel(col as u32 * CELL_W + x, row as u32 * CELL_H + y);
                    if px.0[3] != 0 {
                        return Err(format!(
                            "unused cell (row {row}, col {col}) must be fully transparent, \
                             but pixel ({x},{y}) has alpha {}",
                            px.0[3]
                        ));
                    }
                }
            }
        }
        for col in 0..*used {
            let mut any = false;
            'cell: for y in 0..CELL_H {
                for x in 0..CELL_W {
                    if img
                        .get_pixel(col as u32 * CELL_W + x, row as u32 * CELL_H + y)
                        .0[3]
                        != 0
                    {
                        any = true;
                        break 'cell;
                    }
                }
            }
            if !any {
                return Err(format!(
                    "used cell (row {row}, col {col}) is completely empty"
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------------

const PET_JSON: &str = "{\n  \"id\": \"bytepet-default\",\n  \"displayName\": \"ByteBot\",\n  \"description\": \"BytePet 内置的像素小机器人，开箱即用。\",\n  \"spritesheetPath\": \"spritesheet.png\"\n}\n";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut preview: Option<PathBuf> = None;
    let mut zoom: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--preview" => {
                preview =
                    Some(PathBuf::from(args.get(i + 1).cloned().unwrap_or_else(
                        || "/tmp/default-pet-preview.png".to_string(),
                    )));
                i += 2;
            }
            "--zoom" => {
                zoom = Some(PathBuf::from(
                    args.get(i + 1)
                        .cloned()
                        .unwrap_or_else(|| "/tmp/default-pet-zoom.png".to_string()),
                ));
                i += 2;
            }
            other => {
                eprintln!("unknown argument '{other}' (expected --preview/--zoom <path>)");
                std::process::exit(2);
            }
        }
    }

    let started = Instant::now();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("default-pet");
    std::fs::create_dir_all(&dir).expect("cannot create assets/default-pet");

    let atlas = build_atlas();
    let sheet = dir.join("spritesheet.png");
    atlas.save(&sheet).expect("cannot write spritesheet.png");
    std::fs::write(dir.join("pet.json"), PET_JSON).expect("cannot write pet.json");

    if let Err(err) = verify(&sheet) {
        panic!("self-check failed: {err}");
    }

    if let Some(path) = &preview {
        contact_sheet(2, COLS)
            .save(path)
            .unwrap_or_else(|e| panic!("cannot write preview {}: {e}", path.display()));
    }
    if let Some(path) = &zoom {
        contact_sheet(SCALE, 4)
            .save(path)
            .unwrap_or_else(|e| panic!("cannot write zoom sheet {}: {e}", path.display()));
    }

    let sheet_bytes = std::fs::metadata(&sheet).map(|m| m.len()).unwrap_or(0);
    println!("default pet written");
    println!("  dir        {}", dir.display());
    println!("  pet.json   {} bytes", PET_JSON.len());
    println!("  sheet      {} bytes", sheet_bytes);
    println!(
        "  size       {}x{} px RGBA ({}x{} cells of {}x{})",
        CELL_W * COLS as u32,
        CELL_H * ROWS as u32,
        COLS,
        ROWS,
        CELL_W,
        CELL_H
    );
    println!(
        "  frames     {} used cells, {} empty cells",
        ROW_FRAMES.iter().sum::<usize>(),
        COLS * ROWS - ROW_FRAMES.iter().sum::<usize>()
    );
    if let Some(path) = &preview {
        println!("  preview    {}", path.display());
    }
    if let Some(path) = &zoom {
        println!("  zoom       {}", path.display());
    }
    println!(
        "  elapsed    {:.1} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
}
