//! ボックス・マーカー・接続線・テキストのオーバーレイ描画。

use aviutl2::filter::RgbaPixel;

use crate::keyer::Box;
use crate::text;
use crate::wildcard::Rng;
use crate::{
    BoxShape, CornerMode, FillMode, FilterConfig, GradType, LineOrder, LineType, MarkerType,
    StrokePosition,
};

#[derive(Clone, Copy, Debug)]
struct ColorF {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

#[inline]
fn blend_pixel(px: &mut RgbaPixel, c: ColorF) {
    let ia = 1.0 - c.a;
    px.r = (px.r as f32 * ia + c.r * c.a * 255.0).round() as u8;
    px.g = (px.g as f32 * ia + c.g * c.a * 255.0).round() as u8;
    px.b = (px.b as f32 * ia + c.b * c.a * 255.0).round() as u8;
    px.a = (px.a as f32 * ia + c.a * 255.0).round() as u8;
}

#[inline]
fn blend_pixel_cov(px: &mut RgbaPixel, c: ColorF, cov: f32) {
    if cov <= 0.001 {
        return;
    }
    let mut cc = c;
    cc.a *= cov.min(1.0);
    blend_pixel(px, cc);
}

#[inline]
fn color_of(value: (u8, u8, u8), opacity_percent: f64) -> ColorF {
    ColorF {
        r: value.0 as f32 / 255.0,
        g: value.1 as f32 / 255.0,
        b: value.2 as f32 / 255.0,
        a: (opacity_percent as f32 / 100.0).clamp(0.0, 1.0),
    }
}

#[inline]
fn opacity01(percent: f64) -> f32 {
    (percent as f32 / 100.0).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------
// 2x2 スーパーサンプリング
// ---------------------------------------------------------------------
#[inline]
fn ss2x2(x: f64, y: f64, inside: &dyn Fn(f64, f64) -> bool) -> f32 {
    let mut cov = 0.0;
    if inside(x - 0.25, y - 0.25) {
        cov += 0.25;
    }
    if inside(x + 0.25, y - 0.25) {
        cov += 0.25;
    }
    if inside(x - 0.25, y + 0.25) {
        cov += 0.25;
    }
    if inside(x + 0.25, y + 0.25) {
        cov += 0.25;
    }
    cov
}

// ---------------------------------------------------------------------
// 形状ジオメトリ
// ---------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
struct Geom {
    shape: BoxShape,
    cx: f64,
    cy: f64,
    hw: f64,
    hh: f64,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    corner_r: f64,
}

fn make_geom(b: &Box, shape: BoxShape, corner_radius: f64) -> Geom {
    let bw = (b.max_x - b.min_x + 1) as f64;
    let bh = (b.max_y - b.min_y + 1) as f64;
    let cx = (b.min_x + b.max_x + 1) as f64 * 0.5;
    let cy = (b.min_y + b.max_y + 1) as f64 * 0.5;
    let (hw, hh, min_x, min_y, max_x, max_y) = match shape {
        BoxShape::Rectangle => (
            bw * 0.5,
            bh * 0.5,
            b.min_x as f64,
            b.min_y as f64,
            b.max_x as f64,
            b.max_y as f64,
        ),
        BoxShape::Square => {
            let side = bw.max(bh);
            let half = side * 0.5;
            (half, half, cx - half, cy - half, cx + half, cy + half)
        }
        BoxShape::Ellipse => (
            bw * 0.5,
            bh * 0.5,
            b.min_x as f64,
            b.min_y as f64,
            b.max_x as f64,
            b.max_y as f64,
        ),
        BoxShape::Circle => {
            let side = bw.max(bh);
            let half = side * 0.5;
            (half, half, cx - half, cy - half, cx + half, cy + half)
        }
    };
    let mut g = Geom {
        shape,
        cx,
        cy,
        hw,
        hh,
        min_x,
        min_y,
        max_x,
        max_y,
        corner_r: 0.0,
    };
    if matches!(shape, BoxShape::Rectangle | BoxShape::Square) {
        g.corner_r = corner_radius.max(0.0);
        let half = ((g.max_x - g.min_x).min(g.max_y - g.min_y)) * 0.5;
        if g.corner_r > half {
            g.corner_r = half;
        }
    }
    g
}

/// ランダム塗りのコンテンツスワップ元となるボックス領域。
#[derive(Clone, Copy)]
struct RandSource {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

fn rounded_rect_contains(x: f64, y: f64, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> bool {
    if r <= 0.0 {
        return x >= x0 && x <= x1 && y >= y0 && y <= y1;
    }
    let half = ((x1 - x0).min(y1 - y0)) * 0.5;
    let r = if r > half { half } else { r };
    let cx = if x < x0 + r { x0 + r } else if x > x1 - r { x1 - r } else { x };
    let cy = if y < y0 + r { y0 + r } else if y > y1 - r { y1 - r } else { y };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn contains_point(g: &Geom, x: f64, y: f64) -> bool {
    if matches!(g.shape, BoxShape::Rectangle | BoxShape::Square) {
        return rounded_rect_contains(x, y, g.min_x, g.min_y, g.max_x, g.max_y, g.corner_r);
    }
    let dx = (x - g.cx) / g.hw;
    let dy = (y - g.cy) / g.hh;
    dx * dx + dy * dy <= 1.0
}

// ---------------------------------------------------------------------
// プリミティブ描画
// ---------------------------------------------------------------------
fn fill_rect_band(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    c: ColorF,
    aa: bool,
) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let ix0 = (x0.floor() as i32).max(0);
    let ix1 = (x1.ceil() as i32).min(w as i32 - 1);
    let iy0 = (y0.floor() as i32).max(0);
    let iy1 = (y1.ceil() as i32).min(h as i32 - 1);
    for y in iy0..=iy1 {
        for x in ix0..=ix1 {
            let px = &mut dst[(y as usize) * w + x as usize];
            if !aa {
                blend_pixel(px, c);
                continue;
            }
            let ox0 = (x as f64 - 0.5).max(x0);
            let ox1 = (x as f64 + 0.5).min(x1);
            let oy0 = (y as f64 - 0.5).max(y0);
            let oy1 = (y as f64 + 0.5).min(y1);
            if ox1 <= ox0 || oy1 <= oy0 {
                continue;
            }
            let cov = ((ox1 - ox0) * (oy1 - oy0)) as f32;
            blend_pixel_cov(px, c, cov);
        }
    }
}

fn stamp_disc(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    cx: f64,
    cy: f64,
    r: f64,
    c: ColorF,
    aa: bool,
) {
    if r <= 0.0 {
        return;
    }
    let x0 = ((cx - r).ceil() as i32).max(0);
    let x1 = ((cx + r).floor() as i32).min(w as i32 - 1);
    let y0 = ((cy - r).ceil() as i32).max(0);
    let y1 = ((cy + r).floor() as i32).min(h as i32 - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            let px = &mut dst[(y as usize) * w + x as usize];
            if aa {
                let cov = ((r - d + 0.5) as f32).clamp(0.0, 1.0);
                if cov > 0.0 {
                    blend_pixel_cov(px, c, cov);
                }
            } else if d <= r {
                blend_pixel(px, c);
            }
        }
    }
}

fn point_in_quad(
    px: f64,
    py: f64,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
) -> bool {
    let side = |x1: f64, y1: f64, x2: f64, y2: f64| (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1);
    let s1 = side(ax, ay, bx, by);
    let s2 = side(bx, by, cx, cy);
    let s3 = side(cx, cy, dx, dy);
    let s4 = side(dx, dy, ax, ay);
    let has_neg = s1 < 0.0 || s2 < 0.0 || s3 < 0.0 || s4 < 0.0;
    let has_pos = s1 > 0.0 || s2 > 0.0 || s3 > 0.0 || s4 > 0.0;
    !(has_neg && has_pos)
}

fn fill_quad(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
    col: ColorF,
    aa: bool,
) {
    let min_x = ax.min(bx).min(cx).min(dx);
    let max_x = ax.max(bx).max(cx).max(dx);
    let min_y = ay.min(by).min(cy).min(dy);
    let max_y = ay.max(by).max(cy).max(dy);
    let x0 = (min_x.floor() as i32).max(0);
    let x1 = (max_x.ceil() as i32).min(w as i32 - 1);
    let y0 = (min_y.floor() as i32).max(0);
    let y1 = (max_y.ceil() as i32).min(h as i32 - 1);
    let inside = |x: f64, y: f64| point_in_quad(x, y, ax, ay, bx, by, cx, cy, dx, dy);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = &mut dst[(y as usize) * w + x as usize];
            if aa {
                let cov = ss2x2(x as f64, y as f64, &inside);
                if cov > 0.0 {
                    blend_pixel_cov(px, col, cov);
                }
            } else if inside(x as f64, y as f64) {
                blend_pixel(px, col);
            }
        }
    }
}

fn fill_thick_segment(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    th: f64,
    c: ColorF,
    aa: bool,
) {
    let dx = bx - ax;
    let dy = by - ay;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 || th <= 0.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let r = th * 0.5;
    let px = -uy * r;
    let py = ux * r;
    fill_quad(
        dst,
        w,
        h,
        ax - px,
        ay - py,
        ax + px,
        ay + py,
        bx + px,
        by + py,
        bx - px,
        by - py,
        c,
        aa,
    );
}

fn point_in_triangle(
    px: f64,
    py: f64,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
) -> bool {
    let d1 = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
    let d2 = (cx - bx) * (py - by) - (cy - by) * (px - bx);
    let d3 = (ax - cx) * (py - cy) - (ay - cy) * (px - cx);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

fn fill_triangle(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    ax: f64,
    ay: f64,
    bx: f64,
    by: f64,
    cx: f64,
    cy: f64,
    col: ColorF,
    aa: bool,
) {
    let min_x = ax.min(bx).min(cx);
    let max_x = ax.max(bx).max(cx);
    let min_y = ay.min(by).min(cy);
    let max_y = ay.max(by).max(cy);
    let x0 = (min_x.floor() as i32).max(0);
    let x1 = (max_x.ceil() as i32).min(w as i32 - 1);
    let y0 = (min_y.floor() as i32).max(0);
    let y1 = (max_y.ceil() as i32).min(h as i32 - 1);
    let inside = |x: f64, y: f64| point_in_triangle(x, y, ax, ay, bx, by, cx, cy);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let px = &mut dst[(y as usize) * w + x as usize];
            if aa {
                let cov = ss2x2(x as f64, y as f64, &inside);
                if cov > 0.0 {
                    blend_pixel_cov(px, col, cov);
                }
            } else if inside(x as f64, y as f64) {
                blend_pixel(px, col);
            }
        }
    }
}

// ---------------------------------------------------------------------
// 四角形のストローク (角丸対応)
// ---------------------------------------------------------------------
fn stroke_rect(dst: &mut [RgbaPixel], w: usize, h: usize, g: &Geom, cfg: &FilterConfig) {
    let th = (cfg.stroke_width as f64).max(0.01);
    let dotted = cfg.stroke_dotted;
    let corner_mode = cfg.stroke_corner_mode;
    let aa = true;
    let color = color_of(cfg.stroke_color.to_rgb(), cfg.stroke_opacity);
    let (x0, y0, x1, y1) = (g.min_x, g.min_y, g.max_x, g.max_y);
    let r = g.corner_r;

    // 表示する角 (0=右上, 1=右下, 2=左下, 3=左上)
    let corners: [bool; 4] = match corner_mode {
        CornerMode::CornerOnly
        | CornerMode::HorizontalBracket
        | CornerMode::VerticalBracket => [true, true, true, true],
        // 鍵括弧(左): 右下と左上
        CornerMode::BracketLeft => [false, true, false, true],
        // 鍵括弧(右): 右上と左下
        CornerMode::BracketRight => [true, false, true, false],
    };

    let period = if dotted {
        (cfg.stroke_dotted_freq as f64).max(1.0)
    } else {
        0.0
    };
    let on = if dotted {
        period * (cfg.stroke_dotted_ratio / 100.0)
    } else {
        0.0
    };
    let corner_len = (cfg.stroke_corner_length as f64).max(0.0);
    let half = th * 0.5;
    let off = match cfg.stroke_position {
        StrokePosition::Inside => half,
        StrokePosition::Outside => -half,
        StrokePosition::Center => 0.0,
    };
    let mut phase = if period > 0.0 {
        (cfg.stroke_dotted_offset / 360.0) * period
    } else {
        0.0
    };

    // ストローク位置 (内側/中央/外側) に応じて描画パスをオフセットする
    let cx0 = x0 + off;
    let cy0 = y0 + off;
    let cx1 = x1 - off;
    let cy1 = y1 - off;
    let cr = (r - off).max(0.0);

    if cr > 0.0 {
        // ---- 角丸: 各角に弧 + 直線延長 ----
        let pi = std::f64::consts::PI;
        // 90° の弧の長さ。
        let arc_len = pi * cr * 0.5;
        // 隣の角の弧の起点を超えて直線が伸びないよう、各辺の直線部の長さでクランプする
        let lh = ((cx1 - cr) - (cx0 + cr)).max(0.0);
        let lv = ((cy1 - cr) - (cy0 + cr)).max(0.0);
        let ccx = [cx1 - cr, cx1 - cr, cx0 + cr, cx0 + cr];
        let ccy = [cy0 + cr, cy1 - cr, cy1 - cr, cy0 + cr];
        let a0 = [-pi * 0.5, 0.0, pi * 0.5, pi];
        let edge_in = [(-1.0, 0.0), (0.0, -1.0), (1.0, 0.0), (0.0, 1.0)];
        let edge_out = [(0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (1.0, 0.0)];
        for c in 0..4 {
            if !corners[c] {
                continue;
            }
            let (cx, cy) = (ccx[c], ccy[c]);
            // 角の長さは、角が無い場合と同じく「合計 2×角の長さ」になるようにする。
            // 弧は角の長さに含めるため、各辺の直線の延長は (角の長さ - 弧/2) になる。
            let ext = corner_len - arc_len * 0.5;
            if ext < 0.0 {
                // 角の長さが弧の半分にも満たない: 対角線 (45°) を中心にした部分弧のみ描画
                // (弧の長さ = 合計 2×角の長さ)
                let arc_total = 2.0 * corner_len;
                let center_a = a0[c] + pi * 0.25;
                let half_a = (arc_total / cr) * 0.5;
                let a_start = center_a - half_a;
                let a_end = center_a + half_a;
                const ARC_STEPS: usize = 16;
                let mut prev = (cx + cr * a_start.cos(), cy + cr * a_start.sin());
                for i in 1..=ARC_STEPS {
                    let a = a_start + (a_end - a_start) * i as f64 / ARC_STEPS as f64;
                    let q = (cx + cr * a.cos(), cy + cr * a.sin());
                    fill_thick_segment(dst, w, h, prev.0, prev.1, q.0, q.1, th, color, aa);
                    prev = q;
                }
                continue;
            }
            let ext_in = ext.min(if edge_in[c].1 == 0.0 { lh } else { lv });
            let ext_out = ext.min(if edge_out[c].1 == 0.0 { lh } else { lv });
            let a0p = (cx + cr * a0[c].cos(), cy + cr * a0[c].sin());
            if ext_in > 0.0 {
                fill_thick_segment(
                    dst,
                    w,
                    h,
                    a0p.0 + edge_in[c].0 * ext_in,
                    a0p.1 + edge_in[c].1 * ext_in,
                    a0p.0,
                    a0p.1,
                    th,
                    color,
                    aa,
                );
            }
            let mut prev = a0p;
            for i in 1..=16 {
                let a = a0[c] + pi * 0.5 * i as f64 / 16.0;
                let q = (cx + cr * a.cos(), cy + cr * a.sin());
                fill_thick_segment(dst, w, h, prev.0, prev.1, q.0, q.1, th, color, aa);
                prev = q;
            }
            if ext_out > 0.0 {
                fill_thick_segment(
                    dst,
                    w,
                    h,
                    prev.0,
                    prev.1,
                    prev.0 + edge_out[c].0 * ext_out,
                    prev.1 + edge_out[c].1 * ext_out,
                    th,
                    color,
                    aa,
                );
            }
        }
        // 括弧モード: 対応する辺を直線で結ぶ
        match corner_mode {
            CornerMode::HorizontalBracket => {
                fill_thick_segment(dst, w, h, cx0, cy0 + cr, cx0, cy1 - cr, th, color, aa);
                fill_thick_segment(dst, w, h, cx1, cy0 + cr, cx1, cy1 - cr, th, color, aa);
            }
            CornerMode::VerticalBracket => {
                fill_thick_segment(dst, w, h, cx0 + cr, cy0, cx1 - cr, cy0, th, color, aa);
                fill_thick_segment(dst, w, h, cx0 + cr, cy1, cx1 - cr, cy1, th, color, aa);
            }
            _ => {}
        }
    } else {
        // ---- 平角: 各角に直線 + マイター ----
        // 辺の両端にある角 (上: TL(3)起点/TR(0)終点, ...)
        let edges = [
            (cx0, cy0, cx1, cy0, 3, 0), // 上
            (cx1, cy0, cx1, cy1, 0, 1), // 右
            (cx1, cy1, cx0, cy1, 1, 2), // 下
            (cx0, cy1, cx0, cy0, 2, 3), // 左
        ];
        for &(ex0, ey0, ex1, ey1, c_start, c_end) in &edges {
            let len = ((ex1 - ex0).powi(2) + (ey1 - ey0).powi(2)).sqrt();
            if len <= 0.0 {
                continue;
            }
            let cf = (corner_len / len).min(1.0);
            if corners[c_start] {
                draw_thick_segment_range(
                    dst,
                    w,
                    h,
                    ex0,
                    ey0,
                    ex1,
                    ey1,
                    th,
                    color,
                    0.0,
                    cf,
                    period,
                    on,
                    &mut phase,
                    aa,
                );
            }
            if corners[c_end] {
                draw_thick_segment_range(
                    dst,
                    w,
                    h,
                    ex0,
                    ey0,
                    ex1,
                    ey1,
                    th,
                    color,
                    1.0 - cf,
                    1.0,
                    period,
                    on,
                    &mut phase,
                    aa,
                );
            }
        }
        // マイターコーナー埋め
        let hs = th * 0.5;
        let miter = [
            (cx1, cy0), // 右上 (0)
            (cx1, cy1), // 右下 (1)
            (cx0, cy1), // 左下 (2)
            (cx0, cy0), // 左上 (3)
        ];
        for c in 0..4 {
            if !corners[c] {
                continue;
            }
            fill_rect_band(
                dst,
                w,
                h,
                miter[c].0 - hs,
                miter[c].1 - hs,
                miter[c].0 + hs,
                miter[c].1 + hs,
                color,
                aa,
            );
        }
        // 括弧モード: 対応する辺を直線で結ぶ
        match corner_mode {
            CornerMode::HorizontalBracket => {
                fill_thick_segment(dst, w, h, cx0, cy0, cx0, cy1, th, color, aa);
                fill_thick_segment(dst, w, h, cx1, cy0, cx1, cy1, th, color, aa);
            }
            CornerMode::VerticalBracket => {
                fill_thick_segment(dst, w, h, cx0, cy0, cx1, cy0, th, color, aa);
                fill_thick_segment(dst, w, h, cx0, cy1, cx1, cy1, th, color, aa);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// 楕円・円のストローク
// ---------------------------------------------------------------------
fn stroke_ellipse(dst: &mut [RgbaPixel], w: usize, h: usize, g: &Geom, cfg: &FilterConfig) {
    let th = (cfg.stroke_width as f64).max(0.01);
    let a = g.hw;
    let b = g.hh;
    if a <= 0.5 || b <= 0.5 {
        return;
    }
    // ストローク位置 (内側/中央/外側) に応じた中心線の半径。
    // 角の描画はこの中心線上で行う。
    let (ao, bo) = match cfg.stroke_position {
        StrokePosition::Inside => ((a - th * 0.5).max(0.0), (b - th * 0.5).max(0.0)),
        StrokePosition::Outside => (a + th * 0.5, b + th * 0.5),
        StrokePosition::Center => (a, b),
    };
    let color = color_of(cfg.stroke_color.to_rgb(), cfg.stroke_opacity);
    let aa = true;
    let dotted = cfg.stroke_dotted;
    let period = if dotted {
        (cfg.stroke_dotted_freq as f64).max(1.0)
    } else {
        0.0
    };
    let on = if dotted {
        period * (cfg.stroke_dotted_ratio / 100.0)
    } else {
        0.0
    };

    let corner_mode = cfg.stroke_corner_mode;
    // 表示する角 (0=右上, 1=右下, 2=左下, 3=左上)
    let corners: [bool; 4] = match corner_mode {
        CornerMode::CornerOnly
        | CornerMode::HorizontalBracket
        | CornerMode::VerticalBracket => [true, true, true, true],
        CornerMode::BracketLeft => [false, true, false, true],
        CornerMode::BracketRight => [true, false, true, false],
    };

    let pi = std::f64::consts::PI;
    let corner_len = (cfg.stroke_corner_length as f64).max(0.0);
    let phase = if period > 0.0 {
        (cfg.stroke_dotted_offset / 360.0) * period
    } else {
        0.0
    };
    let (cx, cy) = (g.cx, g.cy);
    // 角に対応する楕円周上の角度 (画面座標: 0°=右, 90°=下)
    // 0=右上(315°), 1=右下(45°), 2=左下(135°), 3=左上(225°)
    let corner_angs = [
        7.0 * pi / 4.0,
        pi / 4.0,
        3.0 * pi / 4.0,
        5.0 * pi / 4.0,
    ];

    // a_start から a_end までの楕円弧を描画する
    let mut draw_arc_range = |a_start: f64, a_end: f64| {
        let steps = 16;
        let mut arc: Vec<Pt> = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let a = a_start + (a_end - a_start) * i as f64 / steps as f64;
            arc.push(Pt {
                x: cx + ao * a.cos(),
                y: cy + bo * a.sin(),
            });
        }
        if dotted {
            let mut dash_phase = phase;
            draw_dashed_polyline(
                dst, w, h, &arc, th, color, period, on, &mut dash_phase, aa,
            );
        } else {
            for i in 0..arc.len().saturating_sub(1) {
                fill_thick_segment(
                    dst,
                    w,
                    h,
                    arc[i].x,
                    arc[i].y,
                    arc[i + 1].x,
                    arc[i + 1].y,
                    th,
                    color,
                    aa,
                );
            }
        }
    };

    match corner_mode {
        CornerMode::HorizontalBracket => {
            // 全ての角 + 左右の辺: 左右の弧全体を描画 (135°→225°, 315°→405°)
            draw_arc_range(3.0 * pi / 4.0, 5.0 * pi / 4.0);
            draw_arc_range(7.0 * pi / 4.0, 9.0 * pi / 4.0);
        }
        CornerMode::VerticalBracket => {
            // 全ての角 + 上下の辺: 上下の弧全体を描画 (225°→315°, 45°→135°)
            draw_arc_range(5.0 * pi / 4.0, 7.0 * pi / 4.0);
            draw_arc_range(pi / 4.0, 3.0 * pi / 4.0);
        }
        CornerMode::CornerOnly | CornerMode::BracketLeft | CornerMode::BracketRight => {
            // 角のみ / 鍵括弧: 選択された角の弧のみ描画
            for (i, &ca) in corner_angs.iter().enumerate() {
                if !corners[i] {
                    continue;
                }
                let rad = ((ao * ao * ca.sin() * ca.sin()) + (bo * bo * ca.cos() * ca.cos()))
                    .sqrt();
                let extent = if rad > 0.0 { corner_len / rad } else { 0.0 };
                let a0 = ca - extent * 0.5;
                draw_arc_range(a0, a0 + extent);
            }
        }
    }
    return;
}

// ---------------------------------------------------------------------
// 塗り
// ---------------------------------------------------------------------
fn draw_shape_fill(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    src: &[RgbaPixel],
    g: &Geom,
    cfg: &FilterConfig,
    rand_source: Option<&RandSource>,
) {
    if !cfg.fill_enabled {
        return;
    }
    let fill_op = opacity01(cfg.fill_opacity);
    if fill_op <= 0.0 {
        return;
    }
    let x0 = (g.min_x.floor() as i32).max(0);
    let x1 = (g.max_x.ceil() as i32).min(w as i32 - 1);
    let y0 = (g.min_y.floor() as i32).max(0);
    let y1 = (g.max_y.ceil() as i32).min(h as i32 - 1);

    let solid = color_of(cfg.fill_color.to_rgb(), cfg.fill_opacity);
    let (mut g_start, mut g_end) = (cfg.grad_start / 100.0, cfg.grad_end / 100.0);
    if g_end < g_start {
        std::mem::swap(&mut g_start, &mut g_end);
    }
    let grad_span = (g_end - g_start).max(1e-6);
    let gs = color_of(cfg.grad_start_color.to_rgb(), 100.0);
    let ge = color_of(cfg.grad_end_color.to_rgb(), 100.0);
    let ang = cfg.grad_angle * std::f64::consts::PI / 180.0;
    let gdx = ang.cos();
    let gdy = ang.sin();
    let ha = g.hw.max(1.0);
    let hb = g.hh.max(1.0);
    let grad_len = (gdx.abs() * ha + gdy.abs() * hb).max(1e-6);

    let hang = cfg.hatch_angle * std::f64::consts::PI / 180.0;
    let hdx = hang.cos();
    let hdy = hang.sin();
    let hfreq = (cfg.hatch_freq as f64).max(1.0);
    let hratio = cfg.hatch_ratio / 100.0;

    let inside_shape = |x: f64, y: f64| contains_point(g, x, y);

    for y in y0..=y1 {
        for x in x0..=x1 {
            let cov = ss2x2(x as f64, y as f64, &inside_shape);
            if cov <= 0.0 {
                continue;
            }
            let mut c = solid;
            match cfg.fill_mode {
                FillMode::Solid => {}
                FillMode::Gradient => {
                    let t = if cfg.grad_type == GradType::Linear {
                        let proj = (x as f64 - g.cx) * gdx + (y as f64 - g.cy) * gdy;
                        (proj + grad_len * 0.5) / grad_len
                    } else {
                        let dx = (x as f64 - g.cx) / ha;
                        let dy = (y as f64 - g.cy) / hb;
                        (dx * dx + dy * dy).sqrt()
                    };
                    let t = ((t - g_start) / grad_span).clamp(0.0, 1.0) as f32;
                    c.r = gs.r + (ge.r - gs.r) * t;
                    c.g = gs.g + (ge.g - gs.g) * t;
                    c.b = gs.b + (ge.b - gs.b) * t;
                    c.a = fill_op;
                }
                FillMode::Hatch => {
                    let v = (x as f64 - g.cx) * hdx + (y as f64 - g.cy) * hdy;
                    let mut idx = (v / hfreq) % 1.0;
                    if idx < 0.0 {
                        idx += 1.0;
                    }
                    let hcov = (((hratio - idx) * hfreq) as f32).clamp(0.0, 1.0);
                    if hcov <= 0.0 {
                        continue;
                    }
                    c = color_of(cfg.fill_color.to_rgb(), cfg.fill_opacity);
                    blend_pixel_cov(&mut dst[(y as usize) * w + x as usize], c, cov * hcov);
                    continue;
                }
                FillMode::Invert => {
                    let p = &src[(y as usize) * w + x as usize];
                    c = ColorF {
                        r: 1.0 - p.r as f32 / 255.0,
                        g: 1.0 - p.g as f32 / 255.0,
                        b: 1.0 - p.b as f32 / 255.0,
                        a: fill_op,
                    };
                }
                FillMode::Random => {
                    // コンテンツスワップ: 他のボックスの画像を引き伸ばして表示
                    let Some(rs) = rand_source else {
                        continue;
                    };
                    let span_x = (g.max_x - g.min_x + 1.0).max(1e-6);
                    let span_y = (g.max_y - g.min_y + 1.0).max(1e-6);
                    let t = ((x as f64 - g.min_x) / span_x).clamp(0.0, 1.0);
                    let u = ((y as f64 - g.min_y) / span_y).clamp(0.0, 1.0);
                    let sx = rs.x0 + (t * (rs.x1 - rs.x0) as f64).round() as i32;
                    let sy = rs.y0 + (u * (rs.y1 - rs.y0) as f64).round() as i32;
                    let sx = sx.clamp(0, w as i32 - 1);
                    let sy = sy.clamp(0, h as i32 - 1);
                    let p = &src[sy as usize * w + sx as usize];
                    c = ColorF {
                        r: p.r as f32 / 255.0,
                        g: p.g as f32 / 255.0,
                        b: p.b as f32 / 255.0,
                        a: fill_op,
                    };
                }
                FillMode::Binarize => {
                    let p = &src[(y as usize) * w + x as usize];
                    let luma =
                        (0.299 * p.r as f32 + 0.587 * p.g as f32 + 0.114 * p.b as f32) / 255.0;
                    let thr = (cfg.fill_binarize_threshold / 255.0) as f32;
                    let mut white = luma >= thr;
                    if cfg.fill_binarize_invert {
                        white = !white;
                    }
                    c = if white {
                        ColorF {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: fill_op,
                        }
                    } else {
                        ColorF {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: fill_op,
                        }
                    };
                }
                FillMode::ColorNoise | FillMode::Noise => {
                    let mut h = (x as u32).wrapping_mul(73856093)
                        ^ (y as u32).wrapping_mul(19349663)
                        ^ 0x9E37_79B9u32;
                    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
                    h ^= h >> 16;
                    if cfg.fill_mode == FillMode::ColorNoise {
                        c = ColorF {
                            r: (h & 0xFF) as f32 / 255.0,
                            g: ((h >> 8) & 0xFF) as f32 / 255.0,
                            b: ((h >> 16) & 0xFF) as f32 / 255.0,
                            a: fill_op,
                        };
                    } else {
                        let v = (h & 0xFF) as f32 / 255.0;
                        c = ColorF {
                            r: v,
                            g: v,
                            b: v,
                            a: fill_op,
                        };
                    }
                }
            }
            blend_pixel_cov(&mut dst[(y as usize) * w + x as usize], c, cov);
        }
    }
}

// ---------------------------------------------------------------------
// マーカー
// ---------------------------------------------------------------------
fn draw_marker(dst: &mut [RgbaPixel], w: usize, h: usize, b: &Box, cfg: &FilterConfig) {
    let color = color_of(cfg.marker_color.to_rgb(), cfg.marker_opacity);
    let (cx, cy) = (b.center_x as f64, b.center_y as f64);
    let size = cfg.marker_size as f64;
    let angle = cfg.marker_angle * std::f64::consts::PI / 180.0;
    let aa = true;

    match cfg.marker_type {
        MarkerType::Dot => {
            stamp_disc(dst, w, h, cx, cy, size * 0.5, color, aa);
        }
        MarkerType::Square => {
            let half = size * 0.5;
            let s = angle.sin();
            let c = angle.cos();
            let extent = half * (c.abs() + s.abs());
            let xa = ((cx - extent).floor() as i32).max(0);
            let xb = ((cx + extent).ceil() as i32).min(w as i32 - 1);
            let ya = ((cy - extent).floor() as i32).max(0);
            let yb = ((cy + extent).ceil() as i32).min(h as i32 - 1);
            let inside = |x: f64, y: f64| -> bool {
                let dx = x - cx;
                let dy = y - cy;
                let rx = dx * c + dy * s;
                let ry = -dx * s + dy * c;
                rx.abs() <= half && ry.abs() <= half
            };
            for y in ya..=yb {
                for x in xa..=xb {
                    let cov = ss2x2(x as f64, y as f64, &inside);
                    if cov > 0.0 {
                        blend_pixel_cov(&mut dst[(y as usize) * w + x as usize], color, cov);
                    }
                }
            }
        }
        MarkerType::Cross => {
            let half = size * 0.5;
            let s = angle.sin();
            let c = angle.cos();
            let dx1 = c * half;
            let dy1 = s * half;
            let px1 = -s * half;
            let py1 = c * half;
            let th = (cfg.marker_width as f64).max(0.01);
            fill_thick_segment(
                dst, w, h, cx - dx1, cy - dy1, cx + dx1, cy + dy1, th, color, aa,
            );
            fill_thick_segment(
                dst, w, h, cx - px1, cy - py1, cx + px1, cy + py1, th, color, aa,
            );
        }
    }
}

// ---------------------------------------------------------------------
// 接続線
// ---------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Pt {
    x: f64,
    y: f64,
}

fn catmull_sample(pts: &[Pt], sub: usize) -> Vec<Pt> {
    let mut out = Vec::new();
    let n = pts.len();
    if n == 0 {
        return out;
    }
    if n == 1 {
        out.push(pts[0]);
        return out;
    }
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[(i + 1).min(n - 1)];
        let p3 = pts[(i + 2).min(n - 1)];
        for step in 0..sub {
            let t = step as f64 / sub as f64;
            let t2 = t * t;
            let t3 = t2 * t;
            let x = 0.5
                * ((2.0 * p1.x)
                    + (-p0.x + p2.x) * t
                    + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
                    + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3);
            let y = 0.5
                * ((2.0 * p1.y)
                    + (-p0.y + p2.y) * t
                    + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
                    + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3);
            out.push(Pt { x, y });
        }
    }
    out.push(pts[n - 1]);
    out
}

fn quad_bezier(a: Pt, c: Pt, b: Pt, sub: usize) -> Vec<Pt> {
    let mut out = Vec::with_capacity(sub + 1);
    for i in 0..=sub {
        let t = i as f64 / sub as f64;
        let it = 1.0 - t;
        let x = it * it * a.x + 2.0 * it * t * c.x + t * t * b.x;
        let y = it * it * a.y + 2.0 * it * t * c.y + t * t * b.y;
        out.push(Pt { x, y });
    }
    out
}

/// 破線を引く。phase は累積アーク距離。
#[allow(clippy::too_many_arguments)]
fn draw_thick_segment_range(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    th: f64,
    c: ColorF,
    u0: f64,
    u1: f64,
    dash_period: f64,
    dash_on: f64,
    phase: &mut f64,
    aa: bool,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 1e-6 || th <= 0.0 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let start = u0 * len;
    let end = u1 * len;
    let pt = |t: f64| Pt {
        x: x0 + ux * t,
        y: y0 + uy * t,
    };

    if dash_period <= 0.0 || dash_on <= 0.0 {
        let a = pt(start);
        let b = pt(end);
        fill_thick_segment(dst, w, h, a.x, a.y, b.x, b.y, th, c, aa);
        *phase += end - start;
        return;
    }

    let step = 0.5;
    let mut in_dash = false;
    let mut dash_start = 0.0;
    let mut t = start;
    while t <= end + 1e-6 {
        let d = *phase + (t - start);
        let mut f = d % dash_period;
        if f < 0.0 {
            f += dash_period;
        }
        let on = f < dash_on;
        if on && !in_dash {
            in_dash = true;
            dash_start = t;
        } else if !on && in_dash {
            let t_end = (t - step).max(start);
            let a = pt(dash_start);
            let b = pt(t_end);
            fill_thick_segment(dst, w, h, a.x, a.y, b.x, b.y, th, c, aa);
            in_dash = false;
        }
        t += step;
    }
    if in_dash {
        let a = pt(dash_start);
        let b = pt(end);
        fill_thick_segment(dst, w, h, a.x, a.y, b.x, b.y, th, c, aa);
    }
    *phase += end - start;
}

fn draw_polyline(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    pts: &[Pt],
    cfg: &FilterConfig,
    dash_period: f64,
    dash_on: f64,
) {
    if pts.len() < 2 {
        return;
    }
    let color = color_of(cfg.line_color.to_rgb(), cfg.line_opacity);
    let th = (cfg.line_width as f64).max(0.01);
    let aa = true;

    if dash_period > 0.0 && dash_on > 0.0 {
        let mut phase = (cfg.line_dotted_offset / 360.0) * dash_period;
        for i in 0..pts.len() - 1 {
            draw_thick_segment_range(
                dst,
                w,
                h,
                pts[i].x,
                pts[i].y,
                pts[i + 1].x,
                pts[i + 1].y,
                th,
                color,
                0.0,
                1.0,
                dash_period,
                dash_on,
                &mut phase,
                aa,
            );
        }
        return;
    }

    let r = th * 0.5;
    let n = pts.len();
    for i in 0..n - 1 {
        fill_thick_segment(
            dst,
            w,
            h,
            pts[i].x,
            pts[i].y,
            pts[i + 1].x,
            pts[i + 1].y,
            th,
            color,
            aa,
        );
    }
    for i in 1..n - 1 {
        let mut d_in = Pt {
            x: pts[i].x - pts[i - 1].x,
            y: pts[i].y - pts[i - 1].y,
        };
        let mut d_out = Pt {
            x: pts[i + 1].x - pts[i].x,
            y: pts[i + 1].y - pts[i].y,
        };
        let l_in = (d_in.x * d_in.x + d_in.y * d_in.y).sqrt();
        if l_in > 1e-9 {
            d_in.x /= l_in;
            d_in.y /= l_in;
        } else {
            d_in = Pt { x: 1.0, y: 0.0 };
        }
        let l_out = (d_out.x * d_out.x + d_out.y * d_out.y).sqrt();
        if l_out > 1e-9 {
            d_out.x /= l_out;
            d_out.y /= l_out;
        } else {
            d_out = d_in;
        }
        let n_in = Pt {
            x: -d_in.y,
            y: d_in.x,
        };
        let n_out = Pt {
            x: -d_out.y,
            y: d_out.x,
        };
        fill_triangle(
            dst,
            w,
            h,
            pts[i].x,
            pts[i].y,
            pts[i].x + n_in.x * r,
            pts[i].y + n_in.y * r,
            pts[i].x + n_out.x * r,
            pts[i].y + n_out.y * r,
            color,
            aa,
        );
        fill_triangle(
            dst,
            w,
            h,
            pts[i].x,
            pts[i].y,
            pts[i].x - n_in.x * r,
            pts[i].y - n_in.y * r,
            pts[i].x - n_out.x * r,
            pts[i].y - n_out.y * r,
            color,
            aa,
        );
    }
}

fn point_at_arc_back(pts: &[Pt], ref_x: f64, ref_y: f64, back: f64) -> (Pt, Pt) {
    if pts.len() < 2 {
        return (Pt { x: ref_x, y: ref_y }, Pt { x: 1.0, y: 0.0 });
    }
    let n = pts.len();
    let mut cum = vec![0.0f64; n];
    for i in 0..n - 1 {
        let dx = pts[i + 1].x - pts[i].x;
        let dy = pts[i + 1].y - pts[i].y;
        cum[i + 1] = cum[i] + (dx * dx + dy * dy).sqrt();
    }
    let mut best_d = 1e30;
    let mut best_i = 0usize;
    let mut best_t = 0.0;
    for i in 0..n - 1 {
        let dx = pts[i + 1].x - pts[i].x;
        let dy = pts[i + 1].y - pts[i].y;
        let len2 = dx * dx + dy * dy;
        let mut t = 0.0;
        if len2 > 1e-12 {
            t = (((ref_x - pts[i].x) * dx + (ref_y - pts[i].y) * dy) / len2).clamp(0.0, 1.0);
        }
        let px = pts[i].x + dx * t;
        let py = pts[i].y + dy * t;
        let dd = (px - ref_x).powi(2) + (py - ref_y).powi(2);
        if dd < best_d {
            best_d = dd;
            best_i = i;
            best_t = t;
        }
    }
    let seg_len = cum[best_i + 1] - cum[best_i];
    let arc_pos = cum[best_i] + seg_len * best_t;
    let target = (arc_pos - back).max(0.0);
    let mut i = 0usize;
    while i + 1 < n && cum[i + 1] < target {
        i += 1;
    }
    let seg = cum[i + 1] - cum[i];
    let mut t = if seg > 1e-9 {
        (target - cum[i]) / seg
    } else {
        0.0
    };
    t = t.clamp(0.0, 1.0);
    let dx = pts[i + 1].x - pts[i].x;
    let dy = pts[i + 1].y - pts[i].y;
    let il = if seg > 1e-9 { 1.0 / seg } else { 0.0 };
    (
        Pt {
            x: pts[i].x + dx * t,
            y: pts[i].y + dy * t,
        },
        Pt {
            x: dx * il,
            y: dy * il,
        },
    )
}

fn draw_arrow_at(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    cx: f64,
    cy: f64,
    dx: f64,
    dy: f64,
    cfg: &FilterConfig,
) {
    if !cfg.arrow_enabled {
        return;
    }
    let color = color_of(cfg.arrow_color.to_rgb(), cfg.arrow_opacity);
    let size = (cfg.arrow_size as f64).max(1.0);
    let dl = (dx * dx + dy * dy).sqrt();
    if dl < 1e-6 {
        return;
    }
    let ux = dx / dl;
    let uy = dy / dl;
    let ang_off = cfg.arrow_angle_offset * std::f64::consts::PI / 180.0;
    let base_ang = uy.atan2(ux) + ang_off;
    let uxa = base_ang.cos();
    let uya = base_ang.sin();
    let pxa = -uya;
    let pya = uxa;
    let fwd = size * (2.0 / 3.0);
    let back = size * (1.0 / 3.0);
    let half = size * (1.0 / 3.0);
    let tip = (cx + uxa * fwd, cy + uya * fwd);
    let b1 = (cx - uxa * back + pxa * half, cy - uya * back + pya * half);
    let b2 = (cx - uxa * back - pxa * half, cy - uya * back - pya * half);
    fill_triangle(dst, w, h, tip.0, tip.1, b1.0, b1.1, b2.0, b2.1, color, true);
}

#[allow(clippy::too_many_arguments)]
fn draw_dashed_polyline(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    pts: &[Pt],
    th: f64,
    color: ColorF,
    period: f64,
    on: f64,
    phase: &mut f64,
    aa: bool,
) {
    if pts.len() < 2 {
        return;
    }
    let mut cums = Vec::with_capacity(pts.len());
    cums.push(0.0);
    let mut total = 0.0;
    for i in 0..pts.len() - 1 {
        let dx = pts[i + 1].x - pts[i].x;
        let dy = pts[i + 1].y - pts[i].y;
        total += (dx * dx + dy * dy).sqrt();
        cums.push(total);
    }
    if total <= 1e-6 {
        return;
    }
    let point_at = |s: f64| -> Pt {
        for i in 0..pts.len() - 1 {
            if s <= cums[i + 1] || i + 2 == pts.len() {
                let seg = cums[i + 1] - cums[i];
                let mut t = if seg > 1e-9 { (s - cums[i]) / seg } else { 0.0 };
                t = t.clamp(0.0, 1.0);
                return Pt {
                    x: pts[i].x + (pts[i + 1].x - pts[i].x) * t,
                    y: pts[i].y + (pts[i + 1].y - pts[i].y) * t,
                };
            }
        }
        pts[pts.len() - 1]
    };
    let seg_index = |s: f64| -> usize {
        for i in 0..pts.len() - 1 {
            if s <= cums[i + 1] {
                return i;
            }
        }
        pts.len() - 2
    };
    let mut draw_dash = |s1: f64, s2: f64| {
        let mut s = s1;
        let mut guard = 0;
        while s < s2 - 1e-6 && guard < 100000 {
            guard += 1;
            let seg = seg_index(s);
            let mut seg_end = cums[seg + 1];
            if seg_end > s2 {
                seg_end = s2;
            }
            if seg_end <= s + 1e-9 {
                break;
            }
            let a = point_at(s);
            let b = point_at(seg_end);
            fill_thick_segment(dst, w, h, a.x, a.y, b.x, b.y, th, color, aa);
            s = seg_end;
        }
    };
    let step = 0.5;
    let mut in_dash = false;
    let mut dash_start = 0.0;
    let mut s = 0.0;
    while s <= total + 1e-6 {
        let d = *phase + s;
        let mut f = d % period;
        if f < 0.0 {
            f += period;
        }
        let onb = f < on;
        if onb && !in_dash {
            in_dash = true;
            dash_start = s;
        } else if !onb && in_dash {
            let s_end = (s - step).max(0.0);
            draw_dash(dash_start, s_end);
            in_dash = false;
        }
        s += step;
    }
    if in_dash {
        draw_dash(dash_start, total);
    }
    *phase += total;
}

fn draw_connection_lines(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    boxes: &[Box],
    cfg: &FilterConfig,
) {
    if !cfg.line_enabled || boxes.len() < 2 {
        return;
    }
    let mut centers: Vec<Pt> = boxes
        .iter()
        .map(|b| Pt {
            x: b.center_x as f64,
            y: b.center_y as f64,
        })
        .collect();

    match cfg.line_order {
        LineOrder::TopToBottom => centers.sort_by(|a, b| a.y.partial_cmp(&b.y).unwrap()),
        LineOrder::BottomToTop => centers.sort_by(|a, b| b.y.partial_cmp(&a.y).unwrap()),
        LineOrder::LeftToRight => centers.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap()),
        LineOrder::RightToLeft => centers.sort_by(|a, b| b.x.partial_cmp(&a.x).unwrap()),
        LineOrder::DistanceFromCenter => {
            let (cxm, cym) = (w as f64 * 0.5, h as f64 * 0.5);
            centers.sort_by(|a, b| {
                let da = (a.x - cxm).powi(2) + (a.y - cym).powi(2);
                let db = (b.x - cxm).powi(2) + (b.y - cym).powi(2);
                da.partial_cmp(&db).unwrap()
            });
        }
        LineOrder::NearestNeighbor => {
            // 貪欲な最近傍巡回路
            let mut ordered: Vec<Pt> = Vec::with_capacity(centers.len());
            let mut visited = vec![false; centers.len()];
            ordered.push(centers[0]);
            visited[0] = true;
            while ordered.len() < centers.len() {
                let cur = *ordered.last().unwrap();
                let mut best = None;
                let mut best_d = f64::MAX;
                for (j, p) in centers.iter().enumerate() {
                    if !visited[j] {
                        let d = (p.x - cur.x).powi(2) + (p.y - cur.y).powi(2);
                        if d < best_d {
                            best_d = d;
                            best = Some(j);
                        }
                    }
                }
                if let Some(j) = best {
                    visited[j] = true;
                    ordered.push(centers[j]);
                } else {
                    break;
                }
            }
            centers = ordered;
        }
        LineOrder::PolarSweep => {
            let n = centers.len();
            let gx = centers.iter().map(|p| p.x).sum::<f64>() / n as f64;
            let gy = centers.iter().map(|p| p.y).sum::<f64>() / n as f64;
            centers.sort_by(|a, b| {
                (a.y - gy).atan2(a.x - gx)
                    .partial_cmp(&(b.y - gy).atan2(b.x - gx))
                    .unwrap()
            });
        }
    }

    let dotted = cfg.line_dotted;
    let dash_period = if dotted {
        (cfg.line_dotted_freq as f64).max(1.0)
    } else {
        0.0
    };
    let dash_on = if dotted {
        dash_period * (cfg.line_dotted_ratio / 100.0)
    } else {
        0.0
    };
    let curve_on = cfg.line_curve;
    let bend_ratio = if curve_on {
        0.0
    } else {
        cfg.line_bending / 100.0
    };
    let curve_segs = 16usize;

    let mut polylines: Vec<Vec<Pt>> = Vec::new();

    if cfg.line_type == LineType::ConnectAll {
        let mut conns: Vec<(f64, usize, usize)> = Vec::new();
        for i in 0..centers.len() {
            for j in i + 1..centers.len() {
                let dx = centers[j].x - centers[i].x;
                let dy = centers[j].y - centers[i].y;
                conns.push(((dx * dx + dy * dy).sqrt(), i, j));
            }
        }
        // 接続数制限: 最短の N 本のみ残す
        if cfg.line_connect_limit
            && cfg.line_connect_max > 0.0
            && (conns.len() as f64) > cfg.line_connect_max
        {
            conns.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            conns.truncate(cfg.line_connect_max as usize);
        }
        for &(_, i, j) in &conns {
            polylines.push(vec![centers[i], centers[j]]);
        }
    } else if cfg.line_type == LineType::MinimumSpanningTree {
        // クラスカルのアルゴリズムによる最小全域木
        let n = centers.len();
        struct Edge {
            a: usize,
            b: usize,
            w: f64,
        }
        let mut edges: Vec<Edge> = Vec::new();
        for i in 0..n {
            for j in i + 1..n {
                let dx = centers[j].x - centers[i].x;
                let dy = centers[j].y - centers[i].y;
                edges.push(Edge {
                    a: i,
                    b: j,
                    w: (dx * dx + dy * dy).sqrt(),
                });
            }
        }
        edges.sort_by(|a, b| a.w.partial_cmp(&b.w).unwrap());
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            if parent[x] != x {
                parent[x] = find(parent, parent[x]);
            }
            parent[x]
        }
        for e in &edges {
            let ra = find(&mut parent, e.a);
            let rb = find(&mut parent, e.b);
            if ra != rb {
                parent[ra] = rb;
                polylines.push(vec![centers[e.a], centers[e.b]]);
            }
        }
    } else {
        polylines.push(centers);
    }

    for poly in &mut polylines {
        let logical = poly.clone();
        let mut work = poly.clone();
        if cfg.line_type == LineType::Sequential && curve_on && work.len() > 2 {
            work = catmull_sample(&work, curve_segs);
        }
        // ベンディング (2次ベジェ)
        if bend_ratio != 0.0 && work.len() >= 2 {
            let mut bent: Vec<Pt> = Vec::new();
            for i in 0..work.len() - 1 {
                let a = work[i];
                let b = work[i + 1];
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let len = (dx * dx + dy * dy).sqrt();
                if len < 1e-6 {
                    bent.push(a);
                    continue;
                }
                let mx = (a.x + b.x) * 0.5;
                let my = (a.y + b.y) * 0.5;
                let nx = -dy / len;
                let ny = dx / len;
                let off = bend_ratio * len * 0.5;
                let ctrl = Pt {
                    x: mx + nx * off,
                    y: my + ny * off,
                };
                let seg = quad_bezier(a, ctrl, b, 12);
                if i > 0 {
                    bent.pop();
                }
                bent.extend_from_slice(&seg);
            }
            if !bent.is_empty() {
                work = bent;
            }
        }
        if work.len() >= 2 {
            draw_polyline(dst, w, h, &work, cfg, dash_period, dash_on);
            if cfg.arrow_enabled {
                let n = logical.len();
                let sections: Vec<(usize, usize)> = if cfg.line_type == LineType::Sequential {
                    (1..n).map(|i| (i - 1, i)).collect()
                } else {
                    (0..n - 1).map(|i| (i, i + 1)).collect()
                };
                for &(from, to) in &sections {
                    let sec_len = ((logical[to].x - logical[from].x).powi(2)
                        + (logical[to].y - logical[from].y).powi(2))
                    .sqrt();
                    let back = (cfg.arrow_position / 100.0) * sec_len;
                    let (tip, dir) = point_at_arc_back(&work, logical[to].x, logical[to].y, back);
                    draw_arrow_at(dst, w, h, tip.x, tip.y, dir.x, dir.y, cfg);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------
// メインオーバーレイ
// ---------------------------------------------------------------------
pub fn draw_overlay(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    src: &[RgbaPixel],
    boxes: &[Box],
    cfg: &FilterConfig,
) {
    if boxes.is_empty() {
        return;
    }

    // ---- ランダム塗り: ボックス間でコンテンツを交換するための順列 ----
    let random_fill = cfg.fill_enabled && cfg.fill_mode == FillMode::Random;
    let mut rand_perm: Vec<usize> = (0..boxes.len()).collect();
    let mut rng = Rng::new(0x9E3779B97F4A7C15);
    if random_fill {
        for i in (1..boxes.len()).rev() {
            let j = (rng.next_u32() as usize) % (i + 1);
            rand_perm.swap(i, j);
        }
    }

    // ---- ボックス ----
    if cfg.box_enabled {
        for (bi, b) in boxes.iter().enumerate() {
            let g = make_geom(b, cfg.box_shape, cfg.box_corner_radius);
            if cfg.fill_enabled {
                let mut rand_source: Option<RandSource> = None;
                if random_fill {
                    let mut participates = true;
                    if cfg.fill_random_only {
                        participates = (rng.next_u32() % 100) < cfg.fill_random_probability as u32;
                    }
                    if participates {
                        let sb = &boxes[rand_perm[bi]];
                        rand_source = Some(RandSource {
                            x0: sb.min_x,
                            y0: sb.min_y,
                            x1: sb.max_x,
                            y1: sb.max_y,
                        });
                    }
                }
                draw_shape_fill(dst, w, h, src, &g, cfg, rand_source.as_ref());
            }
            if cfg.stroke_enabled {
                match cfg.box_shape {
                    BoxShape::Rectangle | BoxShape::Square => stroke_rect(dst, w, h, &g, cfg),
                    BoxShape::Ellipse | BoxShape::Circle => stroke_ellipse(dst, w, h, &g, cfg),
                }
            }
        }
    }

    // ---- マーカー ----
    if cfg.marker_enabled {
        for b in boxes {
            draw_marker(dst, w, h, b, cfg);
        }
    }

    // ---- 接続線 ----
    draw_connection_lines(dst, w, h, boxes, cfg);

    // ---- テキスト ----
    if cfg.text_enabled {
        text::draw_text_overlay(dst, w, h, boxes, cfg);
    }
}
