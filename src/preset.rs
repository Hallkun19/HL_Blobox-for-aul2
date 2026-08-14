//! プリセットの保存 / 読み込み。
//!
//! AE 版の「Save Preset / Load Preset」に相当します。
//! AviUtl2 のデータフォルダ `Presets/HL_Blobox` に JSON として保存します。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use aviutl2::anyhow::{anyhow, Result as AnyResult};
use aviutl2::generic::{EditSection, EditSectionEffectCaller};

/// プリセットに含める設定項目名 (表示名) の一覧。
/// `FilterConfig` の定義順と一致させること。
pub const CONFIG_ITEM_NAMES: &[&str] = &[
    // キーイング設定
    "モード",
    "ターゲット",
    "カラーターゲット",
    "しきい値",
    "ブラー半径",
    "最小ボックスサイズ",
    "最大ボックスサイズ",
    "重なりボックス",
    // ボックス設定
    "ボックス有効",
    "形",
    "角丸半径",
    // ボックス-ストローク設定
    "ストローク有効",
    "ストローク色",
    "ストローク位置",
    "ストローク太さ",
    "ストローク不透明度",
    "ストローク点線",
    "ストローク点線頻度",
    "ストローク点線比率",
    "ストローク点線オフセット",
    "角の種類",
    "角長さ",
    // ボックス-塗り設定
    "塗り有効",
    "塗り不透明度",
    "塗りモード",
    "塗り色",
    "グラデーションタイプ",
    "グラデーション開始色",
    "グラデーション終了色",
    "グラデーション開始位置",
    "グラデーション終了位置",
    "グラデーション角度",
    "ハッチ頻度",
    "ハッチ比率",
    "ハッチ角度",
    "塗り二値化しきい値",
    "塗り二値化反転",
    "塗りランダムのみ",
    "塗りランダム確率",
    // マーカー設定
    "マーカー有効",
    "マーカー種類",
    "マーカー色",
    "マーカーサイズ",
    "マーカー太さ",
    "マーカー角度",
    "マーカー透明度",
    // 接続線設定
    "接続線有効",
    "接続線種類",
    "接続線順序",
    "接続数制限",
    "最大接続数",
    "接続線色",
    "接続線太さ",
    "接続線不透明度",
    "ベンディング",
    "カーブ",
    // 接続線-点線設定
    "線点線",
    "線点線頻度",
    "線点線比率",
    "線点線オフセット",
    // 接続線-矢印設定
    "矢印有効",
    "矢印色",
    "矢印不透明度",
    "矢印サイズ",
    "矢印角度オフセット",
    "矢印位置オフセット",
    // テキスト設定
    "テキスト有効",
    "テキスト内容",
    "テキスト表示基準",
    "テキスト水平揃え",
    "テキスト垂直揃え",
    "テキスト水平位置",
    "テキスト垂直位置",
    "テキストXオフセット",
    "テキストYオフセット",
    "テキストサイズ",
    "フォント",
];

#[derive(serde::Serialize, serde::Deserialize)]
struct PresetFile {
    version: u32,
    values: BTreeMap<String, String>,
}

/// プリセットを保存するフォルダ。
fn preset_dir() -> PathBuf {
    aviutl2::config::app_data_path()
        .join("Presets")
        .join("HL_Blobox")
}

/// フォーカス中のオブジェクトに適用されている HL_Blobox エフェクトを探す。
fn find_effect(edit: &EditSection) -> AnyResult<EditSectionEffectCaller<'_, EditSection>> {
    let object = edit
        .get_focused_object()?
        .ok_or_else(|| anyhow!("オブジェクトが選択されていません"))?;
    let effects = edit.get_effects(object)?;
    let effect = effects
        .into_iter()
        .find(|e| {
            EditSectionEffectCaller::new(edit, *e)
                .get_name()
                .map(|n| n == "HL_Blobox")
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("HL_Blobox エフェクトが見つかりません"))?;
    Ok(EditSectionEffectCaller::new(edit, effect))
}

/// Windows のファイルダイアログを開く。
fn run_file_dialog(save: bool, dir: &Path, default_name: &str) -> Option<PathBuf> {
    use windows::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
        OPENFILENAMEW,
    };
    use windows::core::{PCWSTR, PWSTR};

    let filter = "HL_Blobox Preset (*.json)\0*.json\0All Files (*.*)\0*.*\0\0";
    let filter_wide: Vec<u16> = filter.encode_utf16().collect();
    let mut idir: Vec<u16> = dir.to_string_lossy().encode_utf16().collect();
    idir.push(0);
    let def_ext: Vec<u16> = "json\0".encode_utf16().collect();
    let mut def_name: Vec<u16> = default_name.encode_utf16().collect();
    def_name.push(0);
    let mut file_buf = vec![0u16; 4096];

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.lpstrFilter = PCWSTR(filter_wide.as_ptr());
    ofn.lpstrFile = PWSTR(file_buf.as_mut_ptr());
    ofn.nMaxFile = file_buf.len() as u32;
    ofn.lpstrInitialDir = PCWSTR(idir.as_ptr());
    ofn.lpstrDefExt = PCWSTR(def_ext.as_ptr());
    ofn.lpstrFileTitle = PWSTR(def_name.as_mut_ptr());
    ofn.nMaxFileTitle = def_name.len() as u32;
    ofn.Flags = if save {
        OFN_OVERWRITEPROMPT
    } else {
        OFN_FILEMUSTEXIST
    } | OFN_EXPLORER;

    let ok = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    if !ok {
        return None;
    }

    let len = file_buf.iter().position(|&c| c == 0).unwrap_or(file_buf.len());
    let path = PathBuf::from(String::from_utf16_lossy(&file_buf[..len]));
    if save && path.extension().is_none() {
        return Some(path.with_extension("json"));
    }
    Some(path)
}

/// プリセット保存ボタン。
pub fn on_save_preset(edit: &mut EditSection) -> AnyResult<()> {
    let effect = find_effect(edit)?;
    let dir = preset_dir();
    let _ = std::fs::create_dir_all(&dir);

    let mut values = BTreeMap::new();
    for name in CONFIG_ITEM_NAMES {
        if let Ok(value) = effect.get_item_value(name) {
            values.insert((*name).to_string(), value);
        }
    }

    let Some(path) = run_file_dialog(true, &dir, "hl_blobox_preset.json") else {
        return Ok(()); // キャンセル
    };
    let data = PresetFile { version: 1, values };
    let json = serde_json::to_string_pretty(&data)?;
    std::fs::write(&path, json)?;
    aviutl2::tracing::info!("プリセットを保存しました: {}", path.display());
    Ok(())
}

/// プリセット読込ボタン。
pub fn on_load_preset(edit: &mut EditSection) -> AnyResult<()> {
    let effect = find_effect(edit)?;
    let dir = preset_dir();

    let Some(path) = run_file_dialog(false, &dir, "") else {
        return Ok(()); // キャンセル
    };
    let json = std::fs::read_to_string(&path)?;
    let data: PresetFile = serde_json::from_str(&json)?;

    let mut applied = 0usize;
    for (name, value) in &data.values {
        if effect.set_item_value(name, value).is_ok() {
            applied += 1;
        }
    }
    aviutl2::tracing::info!(
        "プリセットを読み込みました: {} ({} items)",
        path.display(),
        applied
    );
    Ok(())
}
