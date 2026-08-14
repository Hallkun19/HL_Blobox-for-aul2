//! キーイング・連結成分検出・バウンディングボックス生成。

use aviutl2::filter::RgbaPixel;

use crate::{FilterConfig, KeyingMode, LumaTarget, OverlapMode};

/// 検出されたバウンディングボックス。
#[derive(Debug, Clone, Copy)]
pub struct Box {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub center_x: i32,
    pub center_y: i32,
    pub pixels: i64,
    pub cr: u8,
    pub cg: u8,
    pub cb: u8,
    pub id: i32,
}

#[inline]
fn luma(p: &RgbaPixel) -> f32 {
    (0.299 * p.r as f32 + 0.587 * p.g as f32 + 0.114 * p.b as f32) / 255.0
}

// ---------------------------------------------------------------------
// 積分画像 (O(1) ボックスブラー)
// ---------------------------------------------------------------------
struct Integral {
    s: Vec<f32>,
    w: usize,
    h: usize,
}

fn build_integral(img: &[RgbaPixel], w: usize, h: usize, channel: u8) -> Integral {
    let ww = w + 1;
    let hh = h + 1;
    let mut s = vec![0f32; ww * hh];
    for y in 0..h {
        let mut row = 0f32;
        for x in 0..w {
            let p = &img[y * w + x];
            let v = match channel {
                0 => p.r as f32 / 255.0,
                1 => p.g as f32 / 255.0,
                2 => p.b as f32 / 255.0,
                _ => luma(p),
            };
            row += v;
            s[(y + 1) * ww + (x + 1)] = s[y * ww + (x + 1)] + row;
        }
    }
    Integral { s, w, h }
}

#[inline]
fn blur_sample(i: &Integral, x: usize, y: usize, r: usize) -> f32 {
    let w = i.w;
    let h = i.h;
    let x0 = x.saturating_sub(r);
    let y0 = y.saturating_sub(r);
    let x1 = (x + r + 1).min(w);
    let y1 = (y + r + 1).min(h);
    if x1 <= x0 || y1 <= y0 {
        return 0.0;
    }
    let ww = w + 1;
    let a = i.s[y1 * ww + x1];
    let b = i.s[y0 * ww + x1];
    let c = i.s[y1 * ww + x0];
    let d = i.s[y0 * ww + x0];
    (a - b - c + d) / ((x1 - x0) * (y1 - y0)) as f32
}

/// キーイングマスクを計算する。`mask[y*w+x]` に 1 を立てる。
/// `prev` はモーション検出モードでのみ使用される。
pub fn compute_key_mask(
    img: &[RgbaPixel],
    w: usize,
    h: usize,
    cfg: &FilterConfig,
    prev: Option<&[RgbaPixel]>,
) -> Vec<u8> {
    let mut mask = vec![0u8; w * h];
    let r = cfg.blur_radius.round().max(0.0) as usize;
    let thr = (cfg.threshold / 255.0) as f32;

    match cfg.key_mode {
        KeyingMode::Luminance => {
            let brighter = cfg.luma_target == LumaTarget::Brighter;
            if r == 0 {
                for y in 0..h {
                    for x in 0..w {
                        let l = luma(&img[y * w + x]);
                        let on = if brighter { l >= thr } else { l <= thr };
                        mask[y * w + x] = if on { 1 } else { 0 };
                    }
                }
            } else {
                let il = build_integral(img, w, h, 3);
                for y in 0..h {
                    for x in 0..w {
                        let l = blur_sample(&il, x, y, r);
                        let on = if brighter { l >= thr } else { l <= thr };
                        mask[y * w + x] = if on { 1 } else { 0 };
                    }
                }
            }
        }
        KeyingMode::Color => {
            let kr = cfg.color_target.to_rgb().0 as f32 / 255.0;
            let kg = cfg.color_target.to_rgb().1 as f32 / 255.0;
            let kb = cfg.color_target.to_rgb().2 as f32 / 255.0;
            let limit = thr * 3.0;
            if r == 0 {
                for y in 0..h {
                    for x in 0..w {
                        let p = &img[y * w + x];
                        let dr = p.r as f32 / 255.0 - kr;
                        let dg = p.g as f32 / 255.0 - kg;
                        let db = p.b as f32 / 255.0 - kb;
                        let dist = dr.abs() + dg.abs() + db.abs();
                        mask[y * w + x] = if dist < limit { 1 } else { 0 };
                    }
                }
            } else {
                let ir = build_integral(img, w, h, 0);
                let ig = build_integral(img, w, h, 1);
                let ib = build_integral(img, w, h, 2);
                for y in 0..h {
                    for x in 0..w {
                        let dr = blur_sample(&ir, x, y, r) - kr;
                        let dg = blur_sample(&ig, x, y, r) - kg;
                        let db = blur_sample(&ib, x, y, r) - kb;
                        let dist = dr.abs() + dg.abs() + db.abs();
                        mask[y * w + x] = if dist < limit { 1 } else { 0 };
                    }
                }
            }
        }
        KeyingMode::MotionDetection => {
            let Some(prev) = prev else {
                // 前フレームが無い間は検出不能
                return mask;
            };
            if prev.len() != img.len() {
                return mask;
            }
            if r == 0 {
                for y in 0..h {
                    for x in 0..w {
                        let c = luma(&img[y * w + x]);
                        let p = luma(&prev[y * w + x]);
                        mask[y * w + x] = if (c - p).abs() >= thr { 1 } else { 0 };
                    }
                }
            } else {
                let ic = build_integral(img, w, h, 3);
                let ip = build_integral(prev, w, h, 3);
                for y in 0..h {
                    for x in 0..w {
                        let c = blur_sample(&ic, x, y, r);
                        let p = blur_sample(&ip, x, y, r);
                        mask[y * w + x] = if (c - p).abs() >= thr { 1 } else { 0 };
                    }
                }
            }
        }
    }

    mask
}

/// マスクを膨張させる (モルフォロジー dilate)。
/// モーション検出の差分マスクは断片化しやすいため、近傍の画素を統合して
/// 検出ボックスを安定させるために使用する。
pub fn dilate_mask(mask: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let ww = w + 1;
    let hh = h + 1;
    let mut s = vec![0u32; ww * hh];
    for y in 0..h {
        let mut row = 0u32;
        for x in 0..w {
            row += mask[y * w + x] as u32;
            s[(y + 1) * ww + (x + 1)] = s[y * ww + (x + 1)] + row;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let x0 = x.saturating_sub(r);
            let y0 = y.saturating_sub(r);
            let x1 = (x + r + 1).min(w);
            let y1 = (y + r + 1).min(h);
            // 中間で負にならないように引き算の順序を工夫する
            let sum = (s[y1 * ww + x1] - s[y1 * ww + x0])
                - (s[y0 * ww + x1] - s[y0 * ww + x0]);
            if sum > 0 {
                out[y * w + x] = 1;
            }
        }
    }
    out
}

/// マスクから 4-連結の連結成分を検出し、ボックスとして返す。
pub fn find_boxes(mask: &[u8], w: usize, h: usize) -> Vec<Box> {
    let mut visited = vec![false; mask.len()];
    let mut boxes = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for y0 in 0..h {
        for x0 in 0..w {
            let start = y0 * w + x0;
            if mask[start] == 0 || visited[start] {
                continue;
            }
            let mut b = Box {
                min_x: x0 as i32,
                max_x: x0 as i32,
                min_y: y0 as i32,
                max_y: y0 as i32,
                center_x: 0,
                center_y: 0,
                pixels: 0,
                cr: 0,
                cg: 0,
                cb: 0,
                id: 1,
            };
            stack.clear();
            stack.push(start);
            visited[start] = true;
            while let Some(idx) = stack.pop() {
                let x = idx % w;
                let y = idx / w;
                b.pixels += 1;
                if x < b.min_x as usize {
                    b.min_x = x as i32;
                }
                if x > b.max_x as usize {
                    b.max_x = x as i32;
                }
                if y < b.min_y as usize {
                    b.min_y = y as i32;
                }
                if y > b.max_y as usize {
                    b.max_y = y as i32;
                }
                if x > 0 && mask[idx - 1] != 0 && !visited[idx - 1] {
                    visited[idx - 1] = true;
                    stack.push(idx - 1);
                }
                if x + 1 < w && mask[idx + 1] != 0 && !visited[idx + 1] {
                    visited[idx + 1] = true;
                    stack.push(idx + 1);
                }
                if y > 0 && mask[idx - w] != 0 && !visited[idx - w] {
                    visited[idx - w] = true;
                    stack.push(idx - w);
                }
                if y + 1 < h && mask[idx + w] != 0 && !visited[idx + w] {
                    visited[idx + w] = true;
                    stack.push(idx + w);
                }
            }
            b.center_x = (b.min_x + b.max_x + 1) / 2;
            b.center_y = (b.min_y + b.max_y + 1) / 2;
            boxes.push(b);
        }
    }
    boxes
}

#[inline]
fn boxes_overlap(a: &Box, b: &Box) -> bool {
    a.min_x <= b.max_x && b.min_x <= a.max_x && a.min_y <= b.max_y && b.min_y <= a.max_y
}

#[inline]
fn area(b: &Box) -> i64 {
    (b.max_x - b.min_x + 1) as i64 * (b.max_y - b.min_y + 1) as i64
}

/// サイズフィルタと重なり処理を適用し、ID (1始まり) を振る。
pub fn filter_boxes(boxes: &mut Vec<Box>, w: usize, h: usize, cfg: &FilterConfig) {
    let frame_dim = w.max(h).max(1) as f64;

    // サイズフィルタ: 長辺のフレーム寸法に対する割合 (%)
    if !(cfg.min_box_size <= 0.0 && cfg.max_box_size >= 100.0) {
        boxes.retain(|b| {
            let max_dim = (b.max_x - b.min_x + 1).max(b.max_y - b.min_y + 1) as f64;
            let pct = max_dim / frame_dim * 100.0;
            pct >= cfg.min_box_size && pct <= cfg.max_box_size
        });
    }

    match cfg.overlap_mode {
        OverlapMode::Keep => {}
        OverlapMode::RemoveSmaller => {
            boxes.sort_by(|a, b| area(b).cmp(&area(a)));
            let mut kept: Vec<Box> = Vec::with_capacity(boxes.len());
            for i in 0..boxes.len() {
                let b = boxes[i];
                if !kept.iter().any(|k| boxes_overlap(&b, k)) {
                    kept.push(b);
                }
            }
            *boxes = kept;
        }
        OverlapMode::RemoveBigger => {
            boxes.sort_by(|a, b| area(a).cmp(&area(b)));
            let mut kept: Vec<Box> = Vec::with_capacity(boxes.len());
            for i in 0..boxes.len() {
                let b = boxes[i];
                if !kept.iter().any(|k| boxes_overlap(&b, k)) {
                    kept.push(b);
                }
            }
            *boxes = kept;
        }
    }

    for (i, b) in boxes.iter_mut().enumerate() {
        b.id = i as i32 + 1;
    }
}

/// 各ボックスの中心座標にある元画像の色をサンプリングする。
pub fn sample_box_colors(img: &[RgbaPixel], w: usize, h: usize, boxes: &mut [Box]) {
    for b in boxes.iter_mut() {
        let x = b.center_x.clamp(0, w as i32 - 1) as usize;
        let y = b.center_y.clamp(0, h as i32 - 1) as usize;
        let p = img[y * w + x];
        b.cr = p.r;
        b.cg = p.g;
        b.cb = p.b;
    }
}
