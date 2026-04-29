//! Generates PNG assets for Solitaire Quest.
//!
//! Produces:
//! - 52 card face PNGs (120×168) — one per card, with rank, suit symbol, and
//!   pip or face-letter layout baked in.
//! - 5 card back PNGs (120×168) with distinctive coloured patterns.
//! - 5 background PNGs (120×168) with textured felt/wood patterns.
//!
//! Run with: `cargo run -p solitaire_assetgen --bin gen_art`

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

// ---------------------------------------------------------------------------
// Card dimensions and palette
// ---------------------------------------------------------------------------

const W: u32 = 120;
const H: u32 = 168;

const BG: [u8; 4] = [0xFE, 0xFE, 0xF2, 0xFF];
const BORDER: [u8; 4] = [0x99, 0x99, 0x99, 0xFF];
const RED: [u8; 4] = [0xCC, 0x11, 0x11, 0xFF];
const DARK: [u8; 4] = [0x11, 0x11, 0x11, 0xFF];

fn suit_color(suit: u8) -> [u8; 4] {
    if suit == 1 || suit == 2 { RED } else { DARK }
}

fn rank_str(rank: u8) -> &'static str {
    ["A","2","3","4","5","6","7","8","9","10","J","Q","K"][rank as usize]
}

// ---------------------------------------------------------------------------
// Pixel canvas (120×168 RGBA)
// ---------------------------------------------------------------------------

struct Canvas {
    data: Vec<u8>,
}

impl Canvas {
    fn new() -> Self {
        let mut data = vec![0u8; (W * H * 4) as usize];
        for i in 0..(W * H) as usize {
            data[i * 4..i * 4 + 4].copy_from_slice(&BG);
        }
        Self { data }
    }

    /// Fill every pixel with a solid colour, erasing whatever was there before.
    fn fill_solid(&mut self, c: [u8; 4]) {
        for i in 0..(W * H) as usize {
            self.data[i * 4..i * 4 + 4].copy_from_slice(&c);
        }
    }

    /// Draw a 1-pixel-wide axis-aligned horizontal line.
    fn hline(&mut self, y: i32, x0: i32, x1: i32, c: [u8; 4]) {
        for x in x0..=x1 {
            self.set(x, y, c);
        }
    }

    /// Draw a 1-pixel-wide axis-aligned vertical line.
    fn vline(&mut self, x: i32, y0: i32, y1: i32, c: [u8; 4]) {
        for y in y0..=y1 {
            self.set(x, y, c);
        }
    }

    /// Draw a filled diamond outline (ring) of given half-extents and line thickness.
    fn diamond_ring(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, thickness: f32, c: [u8; 4]) {
        for y in (cy - ry - 2.0) as i32..=(cy + ry + 2.0) as i32 {
            for x in (cx - rx - 2.0) as i32..=(cx + rx + 2.0) as i32 {
                let nx = (x as f32 - cx).abs() / rx;
                let ny = (y as f32 - cy).abs() / ry;
                let dist = nx + ny;
                if dist <= 1.0 && dist >= 1.0 - (thickness / rx.min(ry)) {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn set(&mut self, x: i32, y: i32, c: [u8; 4]) {
        if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 { return; }
        let i = (y as u32 * W + x as u32) as usize * 4;
        let a = c[3] as f32 / 255.0;
        if a >= 0.99 {
            self.data[i..i + 4].copy_from_slice(&c);
        } else if a > 0.01 {
            self.data[i]     = (self.data[i]     as f32 * (1.0 - a) + c[0] as f32 * a) as u8;
            self.data[i + 1] = (self.data[i + 1] as f32 * (1.0 - a) + c[1] as f32 * a) as u8;
            self.data[i + 2] = (self.data[i + 2] as f32 * (1.0 - a) + c[2] as f32 * a) as u8;
            self.data[i + 3] = 255;
        }
    }

    fn circle(&mut self, cx: f32, cy: f32, r: f32, c: [u8; 4]) {
        for y in (cy - r - 1.0) as i32..=(cy + r + 1.0) as i32 {
            for x in (cx - r - 1.0) as i32..=(cx + r + 1.0) as i32 {
                if (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2) <= r * r {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
        for ry in y..y + h {
            for rx in x..x + w {
                self.set(rx, ry, c);
            }
        }
    }

    fn triangle(&mut self, pts: [(f32, f32); 3], c: [u8; 4]) {
        let min_x = pts.iter().map(|p| p.0).fold(f32::INFINITY, f32::min) as i32;
        let max_x = pts.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max) as i32;
        let min_y = pts.iter().map(|p| p.1).fold(f32::INFINITY, f32::min) as i32;
        let max_y = pts.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max) as i32;
        let (ax, ay) = pts[0];
        let (bx, by) = pts[1];
        let (ex, ey) = pts[2];
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let d0 = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
                let d1 = (ex - bx) * (py - by) - (ey - by) * (px - bx);
                let d2 = (ax - ex) * (py - ey) - (ay - ey) * (px - ex);
                let neg = d0 < 0.0 || d1 < 0.0 || d2 < 0.0;
                let pos = d0 > 0.0 || d1 > 0.0 || d2 > 0.0;
                if !(neg && pos) {
                    self.set(x, y, c);
                }
            }
        }
    }

    fn diamond(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: [u8; 4]) {
        for y in (cy - ry - 1.0) as i32..=(cy + ry + 1.0) as i32 {
            for x in (cx - rx - 1.0) as i32..=(cx + rx + 1.0) as i32 {
                let nx = (x as f32 - cx).abs() / rx;
                let ny = (y as f32 - cy).abs() / ry;
                if nx + ny <= 1.0 {
                    self.set(x, y, c);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Suit symbol drawing
// ---------------------------------------------------------------------------

fn draw_suit(cv: &mut Canvas, cx: f32, cy: f32, sz: f32, suit: u8, c: [u8; 4]) {
    match suit {
        0 => draw_club(cv, cx, cy, sz, c),
        1 => draw_diamond_sym(cv, cx, cy, sz, c),
        2 => draw_heart(cv, cx, cy, sz, c),
        _ => draw_spade(cv, cx, cy, sz, c),
    }
}

fn draw_heart(cv: &mut Canvas, cx: f32, cy: f32, sz: f32, c: [u8; 4]) {
    let r = sz * 0.33;
    let oy = cy - sz * 0.04;
    cv.circle(cx - sz * 0.22, oy, r, c);
    cv.circle(cx + sz * 0.22, oy, r, c);
    cv.triangle([
        (cx - sz * 0.52, oy + r * 0.4),
        (cx + sz * 0.52, oy + r * 0.4),
        (cx, cy + sz * 0.52),
    ], c);
}

fn draw_spade(cv: &mut Canvas, cx: f32, cy: f32, sz: f32, c: [u8; 4]) {
    cv.triangle([
        (cx, cy - sz * 0.52),
        (cx - sz * 0.52, cy + sz * 0.1),
        (cx + sz * 0.52, cy + sz * 0.1),
    ], c);
    cv.circle(cx - sz * 0.22, cy + sz * 0.06, sz * 0.3, c);
    cv.circle(cx + sz * 0.22, cy + sz * 0.06, sz * 0.3, c);
    // stem + base
    cv.triangle([
        (cx, cy + sz * 0.12),
        (cx - sz * 0.13, cy + sz * 0.5),
        (cx + sz * 0.13, cy + sz * 0.5),
    ], c);
    cv.fill_rect(
        (cx - sz * 0.26) as i32,
        (cy + sz * 0.43) as i32,
        (sz * 0.52) as i32,
        (sz * 0.1) as i32,
        c,
    );
}

fn draw_diamond_sym(cv: &mut Canvas, cx: f32, cy: f32, sz: f32, c: [u8; 4]) {
    cv.diamond(cx, cy, sz * 0.44, sz * 0.57, c);
}

fn draw_club(cv: &mut Canvas, cx: f32, cy: f32, sz: f32, c: [u8; 4]) {
    let r = sz * 0.29;
    cv.circle(cx, cy - sz * 0.22, r, c);
    cv.circle(cx - sz * 0.28, cy + sz * 0.1, r, c);
    cv.circle(cx + sz * 0.28, cy + sz * 0.1, r, c);
    cv.fill_rect(
        (cx - sz * 0.08) as i32,
        (cy + sz * 0.22) as i32,
        (sz * 0.16) as i32 + 1,
        (sz * 0.27) as i32,
        c,
    );
    cv.fill_rect(
        (cx - sz * 0.26) as i32,
        (cy + sz * 0.45) as i32,
        (sz * 0.52) as i32,
        (sz * 0.09) as i32,
        c,
    );
}

// ---------------------------------------------------------------------------
// Text rendering via ab_glyph
// ---------------------------------------------------------------------------

fn draw_text(cv: &mut Canvas, font: &FontRef<'_>, text: &str, px: f32, left: f32, top: f32, c: [u8; 4]) {
    let scale = PxScale::from(px);
    let baseline = top + font.as_scaled(scale).ascent();
    let mut x = left;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(x, baseline));
        let adv = font.as_scaled(scale).h_advance(gid);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                if cov > 0.02 {
                    let alpha = (cov * c[3] as f32) as u8;
                    cv.set(
                        (bounds.min.x + gx as f32) as i32,
                        (bounds.min.y + gy as f32) as i32,
                        [c[0], c[1], c[2], alpha],
                    );
                }
            });
        }
        x += adv;
    }
}

fn text_w(font: &FontRef<'_>, text: &str, px: f32) -> f32 {
    let scale = PxScale::from(px);
    let sf = font.as_scaled(scale);
    text.chars().map(|c| sf.h_advance(font.glyph_id(c))).sum()
}

fn text_h(font: &FontRef<'_>, px: f32) -> f32 {
    let scale = PxScale::from(px);
    let sf = font.as_scaled(scale);
    sf.ascent() - sf.descent()
}

// ---------------------------------------------------------------------------
// Pip layout (rank 0=Ace … 9=Ten; rank 10-12 are face cards)
// ---------------------------------------------------------------------------

fn pip_positions(rank: u8) -> &'static [(f32, f32)] {
    match rank {
        0 => &[(0.5, 0.5)],
        1 => &[(0.5, 0.2), (0.5, 0.8)],
        2 => &[(0.5, 0.12), (0.5, 0.5), (0.5, 0.88)],
        3 => &[(0.25, 0.18), (0.75, 0.18), (0.25, 0.82), (0.75, 0.82)],
        4 => &[(0.25, 0.18), (0.75, 0.18), (0.5, 0.5), (0.25, 0.82), (0.75, 0.82)],
        5 => &[(0.25, 0.12), (0.75, 0.12), (0.25, 0.5), (0.75, 0.5), (0.25, 0.88), (0.75, 0.88)],
        6 => &[(0.25, 0.1), (0.75, 0.1), (0.5, 0.31), (0.25, 0.5), (0.75, 0.5), (0.25, 0.9), (0.75, 0.9)],
        7 => &[(0.25, 0.1), (0.75, 0.1), (0.5, 0.28), (0.25, 0.48), (0.75, 0.48), (0.5, 0.70), (0.25, 0.9), (0.75, 0.9)],
        8 => &[(0.25, 0.1), (0.75, 0.1), (0.25, 0.35), (0.75, 0.35), (0.5, 0.5), (0.25, 0.65), (0.75, 0.65), (0.25, 0.9), (0.75, 0.9)],
        9 => &[(0.25, 0.09), (0.75, 0.09), (0.5, 0.27), (0.25, 0.44), (0.75, 0.44), (0.25, 0.56), (0.75, 0.56), (0.5, 0.73), (0.25, 0.91), (0.75, 0.91)],
        _ => &[],
    }
}

// Pip area within the card (avoids the corner labels).
const PIP_X: f32 = 22.0;
const PIP_Y: f32 = 46.0;
const PIP_W: f32 = 76.0;
const PIP_H: f32 = 80.0;

// ---------------------------------------------------------------------------
// Card face generation
// ---------------------------------------------------------------------------

fn make_card_face(font: &FontRef<'_>, rank: u8, suit: u8) -> Canvas {
    let mut cv = Canvas::new();
    let sc = suit_color(suit);

    // Border (2 px)
    for x in 0..W as i32 {
        cv.set(x, 0, BORDER);
        cv.set(x, 1, BORDER);
        cv.set(x, H as i32 - 2, BORDER);
        cv.set(x, H as i32 - 1, BORDER);
    }
    for y in 0..H as i32 {
        cv.set(0, y, BORDER);
        cv.set(1, y, BORDER);
        cv.set(W as i32 - 2, y, BORDER);
        cv.set(W as i32 - 1, y, BORDER);
    }

    let rank_s = rank_str(rank);
    let rank_px = 18.0f32;
    let suit_sz = 11.0f32;
    let rh = text_h(font, rank_px);
    let rw = text_w(font, rank_s, rank_px);
    let corner_h = rh + 2.0 + suit_sz * 1.5;

    // Top-left corner
    let tl_x = 6.0f32;
    let tl_y = 5.0f32;
    draw_text(&mut cv, font, rank_s, rank_px, tl_x, tl_y, sc);
    draw_suit(&mut cv, tl_x + suit_sz * 0.62, tl_y + rh + 2.0 + suit_sz * 0.75, suit_sz, suit, sc);

    // Bottom-right corner (right-aligned rank, suit above it)
    let br_rx = W as f32 - 6.0;
    let br_by = H as f32 - 5.0;
    let br_ty = br_by - corner_h;
    draw_text(&mut cv, font, rank_s, rank_px, br_rx - rw, br_ty, sc);
    draw_suit(&mut cv, br_rx - suit_sz * 0.62, br_ty + rh + 2.0 + suit_sz * 0.75, suit_sz, suit, sc);

    // Center content
    if rank >= 10 {
        // Face cards: large rank letter + suit symbol below
        let big_px = 52.0f32;
        let big_w = text_w(font, rank_s, big_px);
        let big_h = text_h(font, big_px);
        let big_x = (W as f32 - big_w) / 2.0;
        let big_y = H as f32 * 0.28;
        draw_text(&mut cv, font, rank_s, big_px, big_x, big_y, sc);
        let sym_sz = 22.0f32;
        draw_suit(&mut cv, W as f32 * 0.5, big_y + big_h + sym_sz * 1.0, sym_sz, suit, sc);
    } else {
        // Pip cards
        let pip_sz = if rank == 0 {
            24.0f32 // Ace: large single pip
        } else if rank <= 5 {
            14.0
        } else {
            12.0
        };
        for &(nx, ny) in pip_positions(rank) {
            let cx = PIP_X + nx * PIP_W;
            let cy = PIP_Y + ny * PIP_H;
            draw_suit(&mut cv, cx, cy, pip_sz, suit, sc);
        }
    }

    cv
}

// ---------------------------------------------------------------------------
// PNG encoding helpers
// ---------------------------------------------------------------------------

fn save_card_png(path: &Path, cv: &Canvas) {
    save_png_wh(path, &cv.data, W, H);
}

fn save_png_wh(path: &Path, data: &[u8], w: u32, h: u32) {
    let file = File::create(path)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", path.display()));
    let mut bw = BufWriter::new(file);
    let mut enc = png::Encoder::new(&mut bw, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header()
        .unwrap_or_else(|e| panic!("png header error for {}: {e}", path.display()));
    writer.write_image_data(data)
        .unwrap_or_else(|e| panic!("png data error for {}: {e}", path.display()));
}

// ---------------------------------------------------------------------------
// Card backs (120×168 with distinctive patterns)
// ---------------------------------------------------------------------------

/// back_0 – blue: repeating diamond grid pattern
fn make_back_0() -> Canvas {
    const BASE: [u8; 4] = [0x26, 0x4D, 0x8C, 0xFF];
    const LIGHT: [u8; 4] = [0x5A, 0x80, 0xBF, 0xFF];
    const HIGHLIGHT: [u8; 4] = [0xA0, 0xC0, 0xFF, 0xB0];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);

    // 2-pixel border
    let bw = 4i32;
    for x in 0..W as i32 { for t in 0..bw { cv.set(x, t, LIGHT); cv.set(x, H as i32 - 1 - t, LIGHT); } }
    for y in 0..H as i32 { for t in 0..bw { cv.set(t, y, LIGHT); cv.set(W as i32 - 1 - t, y, LIGHT); } }

    // Diamond grid: row/col spacing
    let gx = 18.0f32;
    let gy = 18.0f32;
    let rx = gx * 0.45;
    let ry = gy * 0.45;
    let mut row = 0;
    let mut cy = 6.0f32 + gy * 0.5;
    while cy < H as f32 - 4.0 {
        let offset = if row % 2 == 0 { 0.0 } else { gx * 0.5 };
        let mut cx = 6.0f32 + gx * 0.5 + offset;
        while cx < W as f32 - 4.0 {
            cv.diamond_ring(cx, cy, rx, ry, 1.5, LIGHT);
            // tiny highlight dot at centre of each diamond
            cv.circle(cx, cy, 1.5, HIGHLIGHT);
            cx += gx;
        }
        cy += gy;
        row += 1;
    }
    cv
}

/// back_1 – red: diagonal crosshatch
fn make_back_1() -> Canvas {
    const BASE: [u8; 4] = [0x8C, 0x1A, 0x1A, 0xFF];
    const LINE: [u8; 4] = [0xCC, 0x55, 0x55, 0xC0];
    const BORDER: [u8; 4] = [0xDD, 0x88, 0x88, 0xFF];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);

    // Diagonal lines every 12 px (NW→SE)
    let spacing = 12i32;
    for k in (-(H as i32)..W as i32).step_by(spacing as usize) {
        for t in 0..W as i32 {
            let y = t + k;
            cv.set(t, y, LINE);
            // 1 px thick — also set neighbour for slightly bolder line
            cv.set(t, y + 1, LINE);
        }
    }
    // Diagonal lines (NE→SW)
    for k in (0..(W as i32 + H as i32)).step_by(spacing as usize) {
        for t in 0..W as i32 {
            let y = k - t;
            cv.set(t, y, LINE);
            cv.set(t, y + 1, LINE);
        }
    }

    // 4-pixel border
    let bw = 4i32;
    for x in 0..W as i32 { for t in 0..bw { cv.set(x, t, BORDER); cv.set(x, H as i32 - 1 - t, BORDER); } }
    for y in 0..H as i32 { for t in 0..bw { cv.set(t, y, BORDER); cv.set(W as i32 - 1 - t, y, BORDER); } }
    cv
}

/// back_2 – green: evenly spaced small circle array
fn make_back_2() -> Canvas {
    const BASE: [u8; 4] = [0x0D, 0x66, 0x1A, 0xFF];
    const DOT: [u8; 4] = [0x40, 0xCC, 0x55, 0xE0];
    const BORDER: [u8; 4] = [0x55, 0xDD, 0x66, 0xFF];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);

    // 4-pixel border
    let bw = 4i32;
    for x in 0..W as i32 { for t in 0..bw { cv.set(x, t, BORDER); cv.set(x, H as i32 - 1 - t, BORDER); } }
    for y in 0..H as i32 { for t in 0..bw { cv.set(t, y, BORDER); cv.set(W as i32 - 1 - t, y, BORDER); } }

    // Circle array (staggered rows)
    let gx = 16.0f32;
    let gy = 16.0f32;
    let r = 3.5f32;
    let mut row = 0;
    let mut cy = 8.0f32 + gy * 0.5;
    while cy < H as f32 - 6.0 {
        let offset = if row % 2 == 0 { 0.0 } else { gx * 0.5 };
        let mut cx = 8.0f32 + gx * 0.5 + offset;
        while cx < W as f32 - 6.0 {
            cv.circle(cx, cy, r, DOT);
            cx += gx;
        }
        cy += gy;
        row += 1;
    }
    cv
}

/// back_3 – purple: concentric diamond outlines
fn make_back_3() -> Canvas {
    const BASE: [u8; 4] = [0x59, 0x14, 0x85, 0xFF];
    const RING: [u8; 4] = [0xA0, 0x60, 0xDD, 0xD0];
    const BORDER: [u8; 4] = [0xBB, 0x77, 0xFF, 0xFF];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);

    // Concentric diamonds from centre
    let cx = W as f32 * 0.5;
    let cy = H as f32 * 0.5;
    let mut rx = 8.0f32;
    let step = 12.0f32;
    while rx < (W as f32).max(H as f32) {
        let ry = rx * (H as f32 / W as f32);
        cv.diamond_ring(cx, cy, rx, ry, 1.5, RING);
        rx += step;
    }

    // 4-pixel border
    let bw = 4i32;
    for x in 0..W as i32 { for t in 0..bw { cv.set(x, t, BORDER); cv.set(x, H as i32 - 1 - t, BORDER); } }
    for y in 0..H as i32 { for t in 0..bw { cv.set(t, y, BORDER); cv.set(W as i32 - 1 - t, y, BORDER); } }
    cv
}

/// back_4 – teal: horizontal stripes with thin decorative lines
fn make_back_4() -> Canvas {
    const BASE: [u8; 4] = [0x0D, 0x66, 0x6B, 0xFF];
    const STRIPE: [u8; 4] = [0x1A, 0x99, 0xA0, 0x90];
    const DECO: [u8; 4] = [0x55, 0xCC, 0xD4, 0xA0];
    const BORDER: [u8; 4] = [0x44, 0xBB, 0xC4, 0xFF];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);

    // Horizontal stripes every 10 px (2 px wide)
    let mut y = 6i32;
    while y < H as i32 - 4 {
        cv.hline(y, 5, W as i32 - 6, STRIPE);
        cv.hline(y + 1, 5, W as i32 - 6, STRIPE);
        y += 10;
    }
    // Thin decorative horizontal lines between stripes
    let mut y = 10i32;
    while y < H as i32 - 4 {
        cv.hline(y, 14, W as i32 - 15, DECO);
        y += 10;
    }

    // 4-pixel border
    let bw = 4i32;
    for x in 0..W as i32 { for t in 0..bw { cv.set(x, t, BORDER); cv.set(x, H as i32 - 1 - t, BORDER); } }
    for y in 0..H as i32 { for t in 0..bw { cv.set(t, y, BORDER); cv.set(W as i32 - 1 - t, y, BORDER); } }
    cv
}

// ---------------------------------------------------------------------------
// Backgrounds (120×168 textured patterns)
// ---------------------------------------------------------------------------

/// bg_0 – dark green felt: subtle grid of faint lines giving a woven texture
fn make_bg_0() -> Canvas {
    const BASE: [u8; 4] = [0x1A, 0x4D, 0x1A, 0xFF];
    const WARP: [u8; 4] = [0x22, 0x60, 0x22, 0x90]; // slightly lighter horizontal threads
    const WEFT: [u8; 4] = [0x15, 0x40, 0x15, 0x90]; // slightly darker vertical threads
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);
    // Horizontal warp lines every 4 px
    for y in (0..H as i32).step_by(4) {
        cv.hline(y, 0, W as i32 - 1, WARP);
    }
    // Vertical weft lines every 4 px
    for x in (0..W as i32).step_by(4) {
        cv.vline(x, 0, H as i32 - 1, WEFT);
    }
    cv
}

/// bg_1 – wood brown: horizontal planks with grain lines
fn make_bg_1() -> Canvas {
    const BASE: [u8; 4] = [0x40, 0x2D, 0x1A, 0xFF];
    const PLANK_EDGE: [u8; 4] = [0x28, 0x1A, 0x0A, 0xFF]; // dark plank separator
    const GRAIN: [u8; 4] = [0x55, 0x3D, 0x28, 0xA0];      // lighter grain streak
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);
    // Horizontal plank edges every 24 px
    for y in (0..H as i32).step_by(24) {
        cv.hline(y, 0, W as i32 - 1, PLANK_EDGE);
        cv.hline(y + 1, 0, W as i32 - 1, PLANK_EDGE);
    }
    // Grain lines within each plank (every 3 px between plank edges)
    for y in (0..H as i32).step_by(3) {
        // Skip the plank edge rows
        if y % 24 < 2 { continue; }
        cv.hline(y, 2, W as i32 - 3, GRAIN);
    }
    cv
}

/// bg_2 – navy: star-field dots scattered in a regular grid
fn make_bg_2() -> Canvas {
    const BASE: [u8; 4] = [0x0D, 0x14, 0x38, 0xFF];
    const STAR_A: [u8; 4] = [0xCC, 0xDD, 0xFF, 0xD0];
    const STAR_B: [u8; 4] = [0x80, 0xA0, 0xDD, 0x80];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);
    // Bright small stars on a staggered grid
    let gx = 14.0f32;
    let gy = 16.0f32;
    let mut row = 0u32;
    let mut cy = gy * 0.5;
    while cy < H as f32 {
        let offset = if row.is_multiple_of(2) { 0.0 } else { gx * 0.5 };
        let mut cx = gx * 0.5 + offset;
        while cx < W as f32 {
            // alternate bright/dim to give depth
            let c = if (row + (cx / gx) as u32).is_multiple_of(3) { STAR_A } else { STAR_B };
            cv.circle(cx, cy, 1.0, c);
            cx += gx;
        }
        cy += gy;
        row += 1;
    }
    cv
}

/// bg_3 – burgundy: diagonal tile pattern
fn make_bg_3() -> Canvas {
    const BASE: [u8; 4] = [0x4D, 0x0D, 0x14, 0xFF];
    const LINE: [u8; 4] = [0x77, 0x22, 0x30, 0xB0];
    const ACCENT: [u8; 4] = [0x99, 0x33, 0x44, 0x80];
    let mut cv = Canvas::new();
    cv.fill_solid(BASE);
    // Diagonal lines in one direction every 16 px
    let spacing = 16i32;
    for k in (-(H as i32)..W as i32 + H as i32).step_by(spacing as usize) {
        for t in 0..W as i32 {
            let y = t + k;
            cv.set(t, y, LINE);
        }
    }
    // Diagonal lines in the other direction every 16 px (accent colour)
    for k in (0..W as i32 + H as i32).step_by(spacing as usize) {
        for t in 0..W as i32 {
            let y = k - t;
            cv.set(t, y, ACCENT);
        }
    }
    cv
}

/// bg_4 – charcoal: subtle checkerboard texture
fn make_bg_4() -> Canvas {
    const DARK: [u8; 4] = [0x1F, 0x1F, 0x24, 0xFF];
    const LIGHT: [u8; 4] = [0x2C, 0x2C, 0x33, 0xFF];
    let mut cv = Canvas::new();
    cv.fill_solid(DARK);
    // 4×4 checkerboard
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            if ((x / 4) + (y / 4)) % 2 == 0 {
                cv.set(x, y, LIGHT);
            }
        }
    }
    cv
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn workspace_root() -> std::path::PathBuf {
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir.parent().unwrap().to_path_buf()
}

fn main() {
    let root = workspace_root();
    std::fs::create_dir_all(root.join("assets/cards/faces")).unwrap();
    std::fs::create_dir_all(root.join("assets/cards/backs")).unwrap();
    std::fs::create_dir_all(root.join("assets/backgrounds")).unwrap();

    // Load font from disk (dev tool — runtime load is fine here).
    let font_path = root.join("assets/fonts/main.ttf");
    let font_bytes = std::fs::read(&font_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", font_path.display()));
    let font = FontRef::try_from_slice(&font_bytes)
        .expect("failed to parse assets/fonts/main.ttf");

    // 52 card faces
    let suits = ["c", "d", "h", "s"];
    let ranks = ["a","2","3","4","5","6","7","8","9","10","j","q","k"];
    for suit in 0u8..4 {
        for rank in 0u8..13 {
            let cv = make_card_face(&font, rank, suit);
            let name = format!("{}_{}.png", ranks[rank as usize], suits[suit as usize]);
            let path = root.join("assets/cards/faces").join(&name);
            save_card_png(&path, &cv);
            println!("wrote {}", path.display());
        }
    }

    // Card backs
    for (i, cv) in [make_back_0(), make_back_1(), make_back_2(), make_back_3(), make_back_4()].iter().enumerate() {
        let path = root.join(format!("assets/cards/backs/back_{i}.png"));
        save_card_png(&path, cv);
        println!("wrote {}", path.display());
    }

    // Backgrounds
    for (i, cv) in [make_bg_0(), make_bg_1(), make_bg_2(), make_bg_3(), make_bg_4()].iter().enumerate() {
        let path = root.join(format!("assets/backgrounds/bg_{i}.png"));
        save_card_png(&path, cv);
        println!("wrote {}", path.display());
    }

    println!("gen_art: all assets generated successfully.");
}
