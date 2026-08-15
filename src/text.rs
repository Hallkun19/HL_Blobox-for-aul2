//! テキストオーバーレイ描画 (GDI によるレンダリング, Windows のみ)。

use std::ffi::c_void;

use aviutl2::filter::RgbaPixel;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    GdiFlush, SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CALCRECT,
    DT_CENTER, DT_LEFT, DT_RIGHT, DT_TOP, DT_WORDBREAK, DRAW_TEXT_FORMAT, HDC, OUT_DEFAULT_PRECIS,
    TRANSPARENT,
};

use crate::keyer::Box;
use crate::wildcard::{expand_wildcards, BoxContext, Rng};
use crate::{AlignH, AlignV, FilterConfig, TextReference};

struct RenderedText {
    width: i32,
    height: i32,
    dib: Vec<u8>,
    row_pitch: usize,
}

struct DcGuard(HDC);
impl Drop for DcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

fn render_to_dib(
    font_name: &str,
    font_size: i32,
    text: &str,
    halign: AlignH,
) -> Option<RenderedText> {
    if text.is_empty() {
        return None;
    }

    let dc = unsafe { CreateCompatibleDC(None) };
    if dc.0.is_null() {
        return None;
    }
    let _dc_guard = DcGuard(dc);

    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut fname: Vec<u16> = font_name.encode_utf16().collect();
    fname.push(0);

    // CreateFontW: 高さ / 幅 / エスケープメント / 方向 / 太さ / イタリック / 下線 / 打ち消し線
    let font = unsafe {
        CreateFontW(
            -font_size,
            0,
            0,
            0,
            400, // FW_NORMAL
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0, // DEFAULT_PITCH | FF_DONTCARE
            PCWSTR(fname.as_ptr()),
        )
    };
    if font.is_invalid() {
        return None;
    }
    let old_font = unsafe { SelectObject(dc, font.into()) };

    let align = match halign {
        AlignH::Left => DT_LEFT,
        AlignH::Center => DT_CENTER,
        AlignH::Right => DT_RIGHT,
    };
    let fmt = DRAW_TEXT_FORMAT(align.0 | DT_WORDBREAK.0 | DT_TOP.0);

    // サイズ計測
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: 100000,
        bottom: 100000,
    };
    unsafe {
        DrawTextW(dc, &mut wide, &mut rc, fmt | DT_CALCRECT);
    }
    let tw = (rc.right + 1).max(1);
    let th = (rc.bottom + 1).max(1);

    // DIB 生成 (32bpp, トップダウン)
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = tw;
    bmi.bmiHeader.biHeight = -th;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = 0; // BI_RGB

    let mut bits: *mut c_void = std::ptr::null_mut();
    let bmp_result = unsafe {
        CreateDIBSection(
            Some(dc),
            &bmi as *const BITMAPINFO,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
    };
    let bmp = match bmp_result {
        Ok(bmp) => bmp,
        Err(_) => {
            unsafe {
                SelectObject(dc, old_font);
                let _ = DeleteObject(font.into());
            }
            return None;
        }
    };
    let old_bmp = unsafe { SelectObject(dc, bmp.into()) };

    let row_pitch = (((tw * 4) + 3) / 4) as usize * 4;
    let dib_size = row_pitch * th as usize;
    if bits.is_null() || dib_size == 0 {
        unsafe {
            SelectObject(dc, old_bmp);
            SelectObject(dc, old_font);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteObject(font.into());
        }
        return None;
    }

    // 黒背景に白文字で描画 (青チャンネルをカバレッジとして使用)
    unsafe {
        std::ptr::write_bytes(bits, 0, dib_size);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FFFFFF));
        let mut rc2 = RECT {
            left: 0,
            top: 0,
            right: tw,
            bottom: th,
        };
        DrawTextW(dc, &mut wide, &mut rc2, fmt);
        let _ = GdiFlush();
    }

    let mut dib = vec![0u8; dib_size];
    unsafe {
        std::ptr::copy_nonoverlapping(bits as *const u8, dib.as_mut_ptr(), dib_size);
    }

    unsafe {
        SelectObject(dc, old_bmp);
        SelectObject(dc, old_font);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteObject(font.into());
    }

    Some(RenderedText {
        width: tw,
        height: th,
        dib,
        row_pitch,
    })
}

fn default_template() -> &'static str {
    "X:$[box_x_position],Y:$[box_y_position]"
}

pub fn draw_text_overlay(
    dst: &mut [RgbaPixel],
    w: usize,
    h: usize,
    boxes: &[Box],
    cfg: &FilterConfig,
) {
    let template = if cfg.text_content.trim().is_empty() {
        default_template().to_string()
    } else {
        cfg.text_content.clone()
    };
    if template.is_empty() || boxes.is_empty() {
        return;
    }

    let font_name = cfg.text_font.as_str();
    let font_size = (cfg.text_size).round().max(1.0) as i32;
    let total_pixels = (w * h) as i64;
    let rng_seed = 0x9E3779B97F4A7C15u64;

    for b in boxes {
        let ctx = BoxContext {
            x: b.min_x as f64,
            y: b.min_y as f64,
            w: (b.max_x - b.min_x + 1) as f64,
            h: (b.max_y - b.min_y + 1) as f64,
            r: b.cr,
            g: b.cg,
            b: b.cb,
            pixels: b.pixels,
            total_pixels,
            id: b.id,
        };
        let mut rng = Rng::new(rng_seed ^ (b.id as u64).wrapping_mul(0x9E3779B1));
        let expanded = expand_wildcards(&template, &ctx, &mut rng);
        if expanded.is_empty() {
            continue;
        }

        let Some(rendered) = render_to_dib(font_name, font_size, &expanded, cfg.text_h_align) else {
            continue;
        };

        // 参照ボックス
        let (bx0, by0, bx1, by1) = if cfg.text_reference == TextReference::Marker {
            let hs = (cfg.marker_size as f64) * 0.5;
            (
                b.center_x as f64 - hs,
                b.center_y as f64 - hs,
                b.center_x as f64 + hs,
                b.center_y as f64 + hs,
            )
        } else {
            (
                b.min_x as f64,
                b.min_y as f64,
                b.max_x as f64,
                b.max_y as f64,
            )
        };

        // アンカー
        let ax = match cfg.text_h_pos {
            AlignH::Left => bx0,
            AlignH::Right => bx1,
            AlignH::Center => (bx0 + bx1) * 0.5,
        };
        let ay = match cfg.text_v_pos {
            AlignV::Top => by0,
            AlignV::Bottom => by1,
            AlignV::Middle => (by0 + by1) * 0.5,
        };
        let ax = ax + cfg.text_offset_x;
        let ay = ay + cfg.text_offset_y;

        // テキストブロックの位置
        let left = match cfg.text_h_align {
            AlignH::Left => ax,
            AlignH::Right => ax - rendered.width as f64,
            AlignH::Center => ax - rendered.width as f64 * 0.5,
        };
        let top = match cfg.text_v_align {
            AlignV::Top => ay,
            AlignV::Bottom => ay - rendered.height as f64,
            AlignV::Middle => ay - rendered.height as f64 * 0.5,
        };

        let px0 = (left.floor() as i32).max(0);
        let py0 = (top.floor() as i32).max(0);
        let px1 = (px0 + rendered.width - 1).min(w as i32 - 1);
        let py1 = (py0 + rendered.height - 1).min(h as i32 - 1);
        if px0 >= w as i32 || py0 >= h as i32 || px1 < 0 || py1 < 0 {
            continue;
        }

        for y in py0..=py1 {
            for x in px0..=px1 {
                let sx = (x - px0) as usize;
                let sy = (y - py0) as usize;
                // 青チャンネル = カバレッジ (白文字を黒背景に描画)
                let cov = rendered.dib[sy * rendered.row_pitch + sx * 4] as f32 / 255.0;
                if cov <= 0.01 {
                    continue;
                }
                let (tr, tg, tb) = cfg.text_color.to_rgb();
                let px = &mut dst[(y as usize) * w + x as usize];
                let ia = 1.0 - cov;
                px.r = (px.r as f32 * ia + cov * tr as f32) as u8;
                px.g = (px.g as f32 * ia + cov * tg as f32) as u8;
                px.b = (px.b as f32 * ia + cov * tb as f32) as u8;
                px.a = (px.a as f32 * ia + cov * 255.0) as u8;
            }
        }
    }
}
