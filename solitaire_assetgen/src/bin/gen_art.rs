//! Generates PNG assets for Solitaire Quest.
//!
//! Produces:
//! - 52 card face PNGs (120×168) — one per card, with rank, suit symbol, and
//!   pip or face-letter layout baked in.
//! - 5 card back PNGs (16×16 placeholder patterns).
//! - 5 background PNGs (16×16 placeholder patterns).
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

fn save_small_png(path: &Path, pixels: &[u8; 1024]) {
    save_png_wh(path, pixels, 16, 16);
}

fn make_small<F: Fn(u32, u32) -> [u8; 4]>(f: F) -> [u8; 1024] {
    let mut out = [0u8; 1024];
    for y in 0u32..16 {
        for x in 0u32..16 {
            let c = f(x, y);
            let i = ((y * 16 + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Card backs (16×16 placeholder patterns)
// ---------------------------------------------------------------------------

fn make_back_0() -> [u8; 1024] {
    make_small(|_, y| if y % 4 < 2 { [0xFF, 0xFF, 0xFF, 40] } else { [0x26, 0x4D, 0x8C, 0xFF] })
}
fn make_back_1() -> [u8; 1024] {
    make_small(|x, y| if (x + y) % 4 < 2 { [0xFF, 0xFF, 0xFF, 40] } else { [0x8C, 0x1A, 0x1A, 0xFF] })
}
fn make_back_2() -> [u8; 1024] {
    make_small(|x, y| if x.is_multiple_of(4) && y.is_multiple_of(4) { [0xFF, 0xFF, 0xFF, 0xFF] } else { [0x0D, 0x66, 0x1A, 0xFF] })
}
fn make_back_3() -> [u8; 1024] {
    make_small(|x, y| {
        let dx = (x as i32 - 8).unsigned_abs();
        let dy = (y as i32 - 8).unsigned_abs();
        if dx + dy <= 4 { [0xFF, 0xFF, 0xFF, 0xFF] } else { [0x59, 0x14, 0x85, 0xFF] }
    })
}
fn make_back_4() -> [u8; 1024] {
    make_small(|x, y| if x == 0 || x == 15 || y == 0 || y == 15 { [0xFF, 0xFF, 0xFF, 0xFF] } else { [0x0D, 0x66, 0x6B, 0xFF] })
}

// ---------------------------------------------------------------------------
// Backgrounds (16×16 placeholder patterns)
// ---------------------------------------------------------------------------

fn make_bg_0() -> [u8; 1024] {
    make_small(|x, y| if x.is_multiple_of(8) || y.is_multiple_of(8) { [0xFF, 0xFF, 0xFF, 30] } else { [0x1A, 0x4D, 0x1A, 0xFF] })
}
fn make_bg_1() -> [u8; 1024] {
    make_small(|_, y| if y.is_multiple_of(2) { [0xFF, 0xFF, 0xFF, 20] } else { [0x40, 0x2D, 0x1A, 0xFF] })
}
fn make_bg_2() -> [u8; 1024] {
    make_small(|x, y| {
        let off: u32 = if (y / 4).is_multiple_of(2) { 0 } else { 4 };
        if (x + off).is_multiple_of(8) && y.is_multiple_of(8) { [0xFF, 0xFF, 0xFF, 0xFF] } else { [0x0D, 0x14, 0x38, 0xFF] }
    })
}
fn make_bg_3() -> [u8; 1024] {
    make_small(|x, y| if (x + y).is_multiple_of(8) { [0xFF, 0xFF, 0xFF, 30] } else { [0x4D, 0x0D, 0x14, 0xFF] })
}
fn make_bg_4() -> [u8; 1024] {
    make_small(|x, y| if (x + y).is_multiple_of(2) && x.is_multiple_of(3) { [0xFF, 0xFF, 0xFF, 20] } else { [0x1F, 0x1F, 0x24, 0xFF] })
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
    for (i, pixels) in [make_back_0(), make_back_1(), make_back_2(), make_back_3(), make_back_4()].iter().enumerate() {
        let path = root.join(format!("assets/cards/backs/back_{i}.png"));
        save_small_png(&path, pixels);
        println!("wrote {}", path.display());
    }

    // Backgrounds
    for (i, pixels) in [make_bg_0(), make_bg_1(), make_bg_2(), make_bg_3(), make_bg_4()].iter().enumerate() {
        let path = root.join(format!("assets/backgrounds/bg_{i}.png"));
        save_small_png(&path, pixels);
        println!("wrote {}", path.display());
    }

    println!("gen_art: all assets generated successfully.");
}
