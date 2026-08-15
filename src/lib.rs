//! HL_Blobox - AviUtl2 フィルタプラグイン。
//!
//! AE 向けエフェクトプラグイン「HL_Blobox」の機能・パラメータを
//! Rust でAviUtl2向けに再実装したものです。
//! 適用レイヤーをキーイングしてバウンディングボックスを検出し、その上に
//! ボックス・マーカー・接続線・テキストなどをオーバーレイ描画します。
//! 
//! ほとんどがAIによってコーディングされています

mod keyer;
mod overlay;
mod preset;
mod text;
mod wildcard;

use std::collections::HashMap;
use std::sync::Mutex;

use aviutl2::{
    AnyResult, AviUtl2Info, tracing,
    filter::{
        FilterConfigColorValue, FilterConfigItemSliceExt, FilterConfigItems,
        FilterConfigSelectItems, FilterPlugin, FilterPluginFlags, FilterPluginTable,
        FilterProcVideo, FilterUserdata, RgbaPixel,
    },
};

use keyer::{compute_key_mask, dilate_mask, filter_boxes, find_boxes, sample_box_colors};
use overlay::draw_overlay;

// ---------------------------------------------------------------------
// 選択肢 (セレクトボックス) の enum 定義
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum KeyingMode {
    #[item(name = "ルミナンス")]
    Luminance,
    #[item(name = "カラー")]
    Color,
    #[item(name = "モーション検出")]
    MotionDetection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum LumaTarget {
    #[item(name = "明るい部分")]
    Brighter,
    #[item(name = "暗い部分")]
    Darker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum OverlapMode {
    #[item(name = "そのまま")]
    Keep,
    #[item(name = "小さい方を除去")]
    RemoveSmaller,
    #[item(name = "大きい方を除去")]
    RemoveBigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum BoxShape {
    #[item(name = "矩形")]
    Rectangle,
    #[item(name = "正方形")]
    Square,
    #[item(name = "楕円")]
    Ellipse,
    #[item(name = "円")]
    Circle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum StrokePosition {
    #[item(name = "内側")]
    Inside,
    #[item(name = "中央")]
    Center,
    #[item(name = "外側")]
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum CornerMode {
    #[item(name = "なし")]
    None,
    #[item(name = "角のみ")]
    CornerOnly,
    #[item(name = "横括弧")]
    HorizontalBracket,
    #[item(name = "縦括弧")]
    VerticalBracket,
    #[item(name = "鍵括弧(左)")]
    BracketLeft,
    #[item(name = "鍵括弧(右)")]
    BracketRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum FillMode {
    #[item(name = "単色")]
    Solid,
    #[item(name = "グラデーション")]
    Gradient,
    #[item(name = "ハッチ")]
    Hatch,
    #[item(name = "反転")]
    Invert,
    #[item(name = "ランダム")]
    Random,
    #[item(name = "二値化")]
    Binarize,
    #[item(name = "カラーノイズ")]
    ColorNoise,
    #[item(name = "ノイズ")]
    Noise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum GradType {
    #[item(name = "リニア")]
    Linear,
    #[item(name = "ラジアル")]
    Radial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum MarkerType {
    #[item(name = "ドット")]
    Dot,
    #[item(name = "スクエア")]
    Square,
    #[item(name = "クロス")]
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum LineType {
    #[item(name = "順次")]
    Sequential,
    #[item(name = "全接続")]
    ConnectAll,
    #[item(name = "最小全域木")]
    MinimumSpanningTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum LineOrder {
    #[item(name = "上から下")]
    TopToBottom,
    #[item(name = "下から上")]
    BottomToTop,
    #[item(name = "左から右")]
    LeftToRight,
    #[item(name = "右から左")]
    RightToLeft,
    #[item(name = "中心からの距離")]
    DistanceFromCenter,
    #[item(name = "最近傍")]
    NearestNeighbor,
    #[item(name = "極座標スイープ")]
    PolarSweep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum TextReference {
    #[item(name = "ボックス")]
    Box,
    #[item(name = "マーカー")]
    Marker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum AlignH {
    #[item(name = "左")]
    Left,
    #[item(name = "中央")]
    Center,
    #[item(name = "右")]
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum AlignV {
    #[item(name = "上")]
    Top,
    #[item(name = "中央")]
    Middle,
    #[item(name = "下")]
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FilterConfigSelectItems)]
pub enum FontName {
    #[item(name = "Meiryo UI")]
    MeiryoUi,
    #[item(name = "Yu Gothic UI")]
    YuGothicUi,
    #[item(name = "MS Gothic")]
    MsGothic,
    #[item(name = "MS Mincho")]
    MsMincho,
    #[item(name = "Arial")]
    Arial,
    #[item(name = "Arial Black")]
    ArialBlack,
    #[item(name = "Comic Sans MS")]
    ComicSansMs,
    #[item(name = "Courier New")]
    CourierNew,
    #[item(name = "Georgia")]
    Georgia,
    #[item(name = "Impact")]
    Impact,
    #[item(name = "Lucida Console")]
    LucidaConsole,
    #[item(name = "Segoe UI")]
    SegoeUi,
    #[item(name = "Tahoma")]
    Tahoma,
    #[item(name = "Times New Roman")]
    TimesNewRoman,
    #[item(name = "Trebuchet MS")]
    TrebuchetMs,
    #[item(name = "Verdana")]
    Verdana,
    #[item(name = "Consolas")]
    Consolas,
}

impl FontName {
    pub fn as_str(&self) -> &'static str {
        match self {
            FontName::MeiryoUi => "Meiryo UI",
            FontName::YuGothicUi => "Yu Gothic UI",
            FontName::MsGothic => "MS Gothic",
            FontName::MsMincho => "MS Mincho",
            FontName::Arial => "Arial",
            FontName::ArialBlack => "Arial Black",
            FontName::ComicSansMs => "Comic Sans MS",
            FontName::CourierNew => "Courier New",
            FontName::Georgia => "Georgia",
            FontName::Impact => "Impact",
            FontName::LucidaConsole => "Lucida Console",
            FontName::SegoeUi => "Segoe UI",
            FontName::Tahoma => "Tahoma",
            FontName::TimesNewRoman => "Times New Roman",
            FontName::TrebuchetMs => "Trebuchet MS",
            FontName::Verdana => "Verdana",
            FontName::Consolas => "Consolas",
        }
    }
}

// ---------------------------------------------------------------------
// フィルタ設定項目 (プロパティ一覧.txt / AE コードに対応)
// ---------------------------------------------------------------------

#[aviutl2::filter::filter_config_items]
#[derive(Debug, Clone, PartialEq)]
pub struct FilterConfig {
    // ---- プリセット (ボタン) ----
    #[button(name = "プリセット保存", error = "log")]
    save_preset: preset::on_save_preset,
    #[button(name = "プリセット読込", error = "log")]
    load_preset: preset::on_load_preset,

    // ---- キーイング設定 ----
    #[group(name = "キーイング設定", opened = false)]
    keying_group: group! {
        #[select(name = "モード", items = KeyingMode, default = KeyingMode::Luminance)]
        key_mode: KeyingMode,
        #[select(name = "ターゲット", items = LumaTarget, default = LumaTarget::Brighter)]
        luma_target: LumaTarget,
        #[color(name = "カラーターゲット", default = 0xffffff)]
        color_target: FilterConfigColorValue,
        #[track(name = "しきい値", range = 0.0..=255.0, default = 50.0, step = 1.0)]
        threshold: f64,
        #[track(name = "ブラー半径", range = 0.0..=100.0, default = 1.0, step = 1.0)]
        blur_radius: f64,
        #[track(name = "最小ボックスサイズ", range = 0.0..=100.0, default = 0.0, step = 0.01)]
        min_box_size: f64,
        #[track(name = "最大ボックスサイズ", range = 0.0..=100.0, default = 100.0, step = 0.01)]
        max_box_size: f64,
        #[select(name = "重なりボックス", items = OverlapMode, default = OverlapMode::Keep)]
        overlap_mode: OverlapMode,
    },

    // ---- ボックス設定 ----
    #[group(name = "ボックス設定", opened = false)]
    box_group: group! {
        #[check(name = "ボックス有効", default = true)]
        box_enabled: bool,
        #[select(name = "形", items = BoxShape, default = BoxShape::Rectangle)]
        box_shape: BoxShape,
        #[track(name = "角丸半径", range = 0.0..=100.0, default = 0.0, step = 1.0)]
        box_corner_radius: f64,
    },

    // ---- ボックス-ストローク設定 ----
    #[group(name = "ボックス-ストローク設定", opened = false)]
    stroke_group: group! {
        #[check(name = "ストローク有効", default = true)]
        stroke_enabled: bool,
        #[color(name = "ストローク色", default = 0xffffff)]
        stroke_color: FilterConfigColorValue,
        #[select(name = "ストローク位置", items = StrokePosition, default = StrokePosition::Center)]
        stroke_position: StrokePosition,
        #[track(name = "ストローク太さ", range = 0.0..=100.0, default = 1.0, step = 1.0)]
        stroke_width: f64,
        #[track(name = "ストローク不透明度", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        stroke_opacity: f64,
        #[check(name = "ストローク点線", default = false)]
        stroke_dotted: bool,
        #[track(name = "ストローク点線頻度", range = 0.0..=100.0, default = 5.0, step = 1.0)]
        stroke_dotted_freq: f64,
        #[track(name = "ストローク点線比率", range = 0.0..=100.0, default = 50.0, step = 1.0)]
        stroke_dotted_ratio: f64,
        #[track(name = "ストローク点線オフセット", range = 0.0..=360.0, default = 0.0, step = 1.0)]
        stroke_dotted_offset: f64,
        #[select(name = "角の種類", items = CornerMode, default = CornerMode::None)]
        stroke_corner_mode: CornerMode,
        #[track(name = "角長さ", range = 0.0..=100.0, default = 10.0, step = 1.0)]
        stroke_corner_length: f64,
    },

    // ---- ボックス-塗り設定 ----
    #[group(name = "ボックス-塗り設定", opened = false)]
    fill_group: group! {
        #[check(name = "塗り有効", default = false)]
        fill_enabled: bool,
        #[track(name = "塗り不透明度", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        fill_opacity: f64,
        #[select(name = "塗りモード", items = FillMode, default = FillMode::Solid)]
        fill_mode: FillMode,
        #[color(name = "塗り色", default = 0xffffff)]
        fill_color: FilterConfigColorValue,
        #[select(name = "グラデーションタイプ", items = GradType, default = GradType::Linear)]
        grad_type: GradType,
        #[color(name = "グラデーション開始色", default = 0x000000)]
        grad_start_color: FilterConfigColorValue,
        #[color(name = "グラデーション終了色", default = 0xffffff)]
        grad_end_color: FilterConfigColorValue,
        #[track(name = "グラデーション開始位置", range = 0.0..=100.0, default = 0.0, step = 1.0)]
        grad_start: f64,
        #[track(name = "グラデーション終了位置", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        grad_end: f64,
        #[track(name = "グラデーション角度", range = 0.0..=360.0, default = 0.0, step = 1.0)]
        grad_angle: f64,
        #[track(name = "ハッチ頻度", range = 0.0..=100.0, default = 5.0, step = 1.0)]
        hatch_freq: f64,
        #[track(name = "ハッチ比率", range = 0.0..=100.0, default = 50.0, step = 1.0)]
        hatch_ratio: f64,
        #[track(name = "ハッチ角度", range = 0.0..=360.0, default = 45.0, step = 1.0)]
        hatch_angle: f64,
        #[track(name = "塗り二値化しきい値", range = 0.0..=255.0, default = 128.0, step = 1.0)]
        fill_binarize_threshold: f64,
        #[check(name = "塗り二値化反転", default = false)]
        fill_binarize_invert: bool,
        #[check(name = "塗りランダムのみ", default = false)]
        fill_random_only: bool,
        #[track(name = "塗りランダム確率", range = 0.0..=100.0, default = 50.0, step = 1.0)]
        fill_random_probability: f64,
    },

    // ---- マーカー設定 ----
    #[group(name = "マーカー設定", opened = false)]
    marker_group: group! {
        #[check(name = "マーカー有効", default = false)]
        marker_enabled: bool,
        #[select(name = "マーカー種類", items = MarkerType, default = MarkerType::Dot)]
        marker_type: MarkerType,
        #[color(name = "マーカー色", default = 0xffffff)]
        marker_color: FilterConfigColorValue,
        #[track(name = "マーカーサイズ", range = 0.0..=1000.0, default = 10.0, step = 1.0)]
        marker_size: f64,
        #[track(name = "マーカー太さ", range = 0.0..=100.0, default = 1.0, step = 1.0)]
        marker_width: f64,
        #[track(name = "マーカー角度", range = 0.0..=360.0, default = 0.0, step = 1.0)]
        marker_angle: f64,
        #[track(name = "マーカー透明度", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        marker_opacity: f64,
    },

    // ---- 接続線設定 ----
    #[group(name = "接続線設定", opened = false)]
    line_group: group! {
        #[check(name = "接続線有効", default = false)]
        line_enabled: bool,
        #[select(name = "接続線種類", items = LineType, default = LineType::Sequential)]
        line_type: LineType,
        #[select(name = "接続線順序", items = LineOrder, default = LineOrder::TopToBottom)]
        line_order: LineOrder,
        #[check(name = "接続数制限", default = false)]
        line_connect_limit: bool,
        #[track(name = "最大接続数", range = 1.0..=1000.0, default = 100.0, step = 1.0)]
        line_connect_max: f64,
        #[color(name = "接続線色", default = 0xffffff)]
        line_color: FilterConfigColorValue,
        #[track(name = "接続線太さ", range = 0.0..=100.0, default = 1.0, step = 1.0)]
        line_width: f64,
        #[track(name = "接続線不透明度", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        line_opacity: f64,
        #[track(name = "ベンディング", range = -100.0..=100.0, default = 0.0, step = 1.0)]
        line_bending: f64,
        #[check(name = "カーブ", default = false)]
        line_curve: bool,
    },

    // ---- 接続線-点線設定 ----
    #[group(name = "接続線-点線設定", opened = false)]
    line_dotted_group: group! {
        #[check(name = "線点線", default = false)]
        line_dotted: bool,
        #[track(name = "線点線頻度", range = 0.0..=100.0, default = 5.0, step = 1.0)]
        line_dotted_freq: f64,
        #[track(name = "線点線比率", range = 0.0..=100.0, default = 50.0, step = 1.0)]
        line_dotted_ratio: f64,
        #[track(name = "線点線オフセット", range = 0.0..=360.0, default = 0.0, step = 1.0)]
        line_dotted_offset: f64,
    },

    // ---- 接続線-矢印設定 ----
    #[group(name = "接続線-矢印設定", opened = false)]
    arrow_group: group! {
        #[check(name = "矢印有効", default = false)]
        arrow_enabled: bool,
        #[color(name = "矢印色", default = 0xffffff)]
        arrow_color: FilterConfigColorValue,
        #[track(name = "矢印不透明度", range = 0.0..=100.0, default = 100.0, step = 1.0)]
        arrow_opacity: f64,
        #[track(name = "矢印サイズ", range = 0.0..=100.0, default = 10.0, step = 1.0)]
        arrow_size: f64,
        #[track(name = "矢印角度オフセット", range = 0.0..=360.0, default = 0.0, step = 1.0)]
        arrow_angle_offset: f64,
        #[track(name = "矢印位置オフセット", range = 0.0..=100.0, default = 0.0, step = 1.0)]
        arrow_position: f64,
    },

    // ---- テキスト設定 ----
    #[group(name = "テキスト設定", opened = false)]
    text_group: group! {
        #[check(name = "テキスト有効", default = false)]
        text_enabled: bool,
        #[text(
            name = "テキスト内容",
            default = "X:$[box_x_position],Y:$[box_y_position]"
        )]
        text_content: String,
        #[color(name = "テキスト色", default = 0xffffff)]
        text_color: FilterConfigColorValue,
        #[select(name = "テキスト表示基準", items = TextReference, default = TextReference::Box)]
        text_reference: TextReference,
        #[select(name = "テキスト水平揃え", items = AlignH, default = AlignH::Center)]
        text_h_align: AlignH,
        #[select(name = "テキスト垂直揃え", items = AlignV, default = AlignV::Middle)]
        text_v_align: AlignV,
        #[select(name = "テキスト水平位置", items = AlignH, default = AlignH::Center)]
        text_h_pos: AlignH,
        #[select(name = "テキスト垂直位置", items = AlignV, default = AlignV::Middle)]
        text_v_pos: AlignV,
        #[track(name = "テキストXオフセット", range = -1000.0..=1000.0, default = 0.0, step = 1.0)]
        text_offset_x: f64,
        #[track(name = "テキストYオフセット", range = -1000.0..=1000.0, default = 0.0, step = 1.0)]
        text_offset_y: f64,
        #[track(name = "テキストサイズ", range = 0.0..=1000.0, default = 24.0, step = 1.0)]
        text_size: f64,
        #[select(name = "フォント", items = FontName, default = FontName::MeiryoUi)]
        text_font: FontName,
    },
}

// ---------------------------------------------------------------------
// ユーザーデータ (モーション検出用の前フレーム保持)
// ---------------------------------------------------------------------

/// モーション検出用に保持しておくフレーム数。
/// AviUtl2 は同じフレームを複数回レンダリングすることがあるため、
/// 直前フレームがキャッシュに残るように数フレーム分保持する。
const MOTION_CACHE_FRAMES: usize = 4;

/// エフェクト毎に保持されるユーザーデータ。
/// モーション検出モードで使用する直前フレームをオブジェクトIDごとに保持します。
pub struct BloboxUserdata {
    /// オブジェクトID -> (フレーム番号, 画像) のリスト
    prev_frames: Mutex<HashMap<i64, Vec<(u32, Vec<RgbaPixel>)>>>,
}

impl FilterUserdata for BloboxUserdata {
    fn new(_effect_id: i64) -> Self {
        Self {
            prev_frames: Mutex::new(HashMap::new()),
        }
    }
}

impl BloboxUserdata {
    /// 指定フレームより前で、最も新しいフレームの画像をキャッシュから取得する。
    /// 理想的には直前フレーム (frame - 1)。AviUtl2 はフレームを順序どおりに
    /// 処理するとは限らないため、現在より前のフレームがあればベストエフォートで使う。
    fn get_prev_frame(&self, object_id: i64, current_frame: u32) -> Option<Vec<RgbaPixel>> {
        let map = self.prev_frames.lock().unwrap();
        map.get(&object_id).and_then(|frames| {
            frames
                .iter()
                .filter(|(f, _)| *f < current_frame)
                .max_by_key(|(f, _)| *f)
                .map(|(_, img)| img.clone())
        })
    }

    /// 現在フレームの画像をキャッシュに保存する。
    /// 同じフレームの再レンダリング時は上書きし、直前フレームは残す。
    ///
    /// キャッシュは現在フレームから見て直近の数フレームだけ保持する。
    /// パラメータ変更などで AviUtl2 がフレーム 0 から再レンダリングした場合、
    /// 以前のレンダリングで溜まった古いフレーム番号の残骸がキャッシュに残って
    /// 新しいフレームを押し出してしまわないようにするため。
    fn store_frame(&self, object_id: i64, frame: u32, image: Vec<RgbaPixel>) {
        let mut map = self.prev_frames.lock().unwrap();
        let frames = map.entry(object_id).or_default();
        match frames.iter_mut().find(|(f, _)| *f == frame) {
            Some(slot) => slot.1 = image,
            None => frames.push((frame, image)),
        }
        // 現在フレームより古すぎる・新しいフレームは再レンダリング前の残骸なので捨てる
        let lo = frame.saturating_sub(MOTION_CACHE_FRAMES as u32);
        frames.retain(|(f, _)| *f >= lo && *f <= frame);
        frames.sort_by(|a, b| b.0.cmp(&a.0));
    }
}

// ---------------------------------------------------------------------
// プラグイン本体
// ---------------------------------------------------------------------

#[aviutl2::plugin(FilterPlugin)]
struct HLBlobox;

impl FilterPlugin for HLBlobox {
    type Userdata = BloboxUserdata;

    fn new(_info: AviUtl2Info) -> AnyResult<Self> {
        aviutl2::tracing_subscriber::fmt()
            .with_max_level(if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            })
            .event_format(aviutl2::logger::AviUtl2Formatter)
            .with_writer(aviutl2::logger::AviUtl2LogWriter)
            .init();
        Ok(Self)
    }

    fn plugin_info(&self) -> FilterPluginTable {
        FilterPluginTable {
            name: "HL_Blobox".to_string(),
            // ラベルはフィルタ効果のジャンル (カテゴリ) になる。
            // 「ジャンル\ラベル」形式にするとラベルが名前と二重に表示されるため、
            // ジャンル名だけを指定する (例: DepthMapFilter は L"加工")。
            label: Some("HL_Plugins".to_string()),
            information: format!(
                "HL_Blobox v{version} by halkun19 - AviUtl2 Blob tracker plugin",
                version = env!("CARGO_PKG_VERSION")
            ),
            flags: aviutl2::bitflag!(FilterPluginFlags { video: true, filter: true }),
            config_items: FilterConfig::to_config_items(),
        }
    }

    fn proc_video(
        &self,
        config: &[aviutl2::filter::FilterConfigItem],
        video: &mut FilterProcVideo<Self::Userdata>,
    ) -> AnyResult<()> {
        let cfg: FilterConfig = config.to_struct();

        let width = video.video_object.width as usize;
        let height = video.video_object.height as usize;
        if width == 0 || height == 0 {
            return Ok(());
        }

        let mut image: Vec<RgbaPixel> = vec![RgbaPixel::default(); width * height];
        if video.get_image_data(&mut image) == 0 {
            return Ok(());
        }
        let original = image.clone();

        // モーション検出: 直前フレームをキャッシュから探す
        // AviUtl2 はフレームを順序どおり処理するとは限らないため、
        // 現在より前で最も新しいフレームをベストエフォートで使用する。
        let prev = if cfg.key_mode == KeyingMode::MotionDetection {
            let found = video
                .userdata
                .read()
                .get_prev_frame(video.object.id, video.object.frame);
            if found.is_none() {
                tracing::debug!(
                    "HL_Blobox motion: 直前フレームなし (id={}, frame={})",
                    video.object.id,
                    video.object.frame
                );
            }
            found
        } else {
            None
        };

        // 1) キーイング
        let mut mask = compute_key_mask(&image, width, height, &cfg, prev.as_deref());
        if cfg.key_mode == KeyingMode::MotionDetection {
            // 差分マスクは断片化しやすいため、膨張させて近傍の領域を統合する。
            // これにより検出ボックスの長辺が大きくなり、最小ボックスサイズの
            // フィルタでも除去されにくくなる。
            mask = dilate_mask(&mask, width, height, 2);
        }

        // 2) 連結成分 → バウンディングボックス
        let mut boxes = find_boxes(&mask, width, height);
        filter_boxes(&mut boxes, width, height, &cfg);
        sample_box_colors(&image, width, height, &mut boxes);
        if cfg.key_mode == KeyingMode::MotionDetection {
            tracing::debug!(
                "HL_Blobox motion: id={} frame={} prev={} mask_px={} boxes={}",
                video.object.id,
                video.object.frame,
                prev.is_some(),
                mask.iter().filter(|&&v| v != 0).count(),
                boxes.len()
            );
        }

        // 3) オーバーレイ描画 (元画像の上に描画)
        draw_overlay(&mut image, width, height, &original, &boxes, &cfg);

        video.set_image_data(&image, width as u32, height as u32);

        // モーション検出用に現在フレームをキャッシュに保存
        // (同じフレームの再レンダリングでは上書きし、直前フレームは残す)
        if cfg.key_mode == KeyingMode::MotionDetection {
            video
                .userdata
                .write()
                .store_frame(video.object.id, video.object.frame, original);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------
// 手動エクスポート
//
// 設定項目が多いため、`FilterConfigItem` (Data バリアントが 16KiB) を要素に
// 持つ配列を生成する `to_config_items()` がスタックを 1MB 超消費する。
// AviUtl2 のデフォルトのプラグインスレッド (スタック 1MB) では
// スタックオーバーフローするため、初期化を大きなスタックのスレッドで実行する。
// ---------------------------------------------------------------------

use aviutl2::sys::cache2::CACHE_HANDLE;
use aviutl2::sys::config2::CONFIG_HANDLE;
use aviutl2::sys::filter2::FILTER_PLUGIN_TABLE;
use aviutl2::sys::logger2::LOG_HANDLE;

const INIT_STACK_SIZE: usize = 64 * 1024 * 1024;

#[no_mangle]
pub unsafe extern "C" fn RequiredVersion() -> u32 {
    aviutl2::MINIMUM_AVIUTL2_VERSION.into()
}

#[no_mangle]
pub unsafe extern "C" fn InitializeLogger(logger: *mut LOG_HANDLE) {
    aviutl2::logger::__initialize_logger_unwind(logger)
}

#[no_mangle]
pub unsafe extern "C" fn InitializeConfig(config: *mut CONFIG_HANDLE) {
    aviutl2::config::__initialize_config_handle_unwind(config)
}

#[no_mangle]
pub unsafe extern "C" fn InitializeCache(cache: *mut CACHE_HANDLE) {
    aviutl2::cache::__initialize_cache_unwind(cache)
}

#[no_mangle]
pub unsafe extern "C" fn InitializePlugin(version: u32) -> bool {
    let result = std::thread::Builder::new()
        .stack_size(INIT_STACK_SIZE)
        .spawn(move || {
            aviutl2::filter::__bridge::initialize_plugin_c_unwind::<HLBlobox>(version)
        });
    match result {
        Ok(handle) => handle.join().unwrap_or(false),
        Err(e) => {
            tracing::error!("Failed to spawn plugin init thread: {e}");
            false
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn UninitializePlugin() {
    aviutl2::filter::__bridge::uninitialize_plugin_c_unwind::<HLBlobox>()
}

#[no_mangle]
pub unsafe extern "C" fn GetFilterPluginTable() -> *mut FILTER_PLUGIN_TABLE {
    aviutl2::filter::__bridge::create_table_unwind::<HLBlobox>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aviutl2::filter::RgbaPixel;

    fn solid(w: usize, h: usize, rgb: (u8, u8, u8)) -> Vec<RgbaPixel> {
        (0..w * h)
            .map(|_| RgbaPixel {
                r: rgb.0,
                g: rgb.1,
                b: rgb.2,
                a: 255,
            })
            .collect()
    }

    fn draw_rect(
        img: &mut [RgbaPixel],
        w: usize,
        x0: usize,
        y0: usize,
        x1: usize,
        y1: usize,
        rgb: (u8, u8, u8),
    ) {
        for y in y0..y1 {
            for x in x0..x1 {
                img[y * w + x] = RgbaPixel {
                    r: rgb.0,
                    g: rgb.1,
                    b: rgb.2,
                    a: 255,
                };
            }
        }
    }

    #[test]
    fn test_luminance_keying_finds_blob() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (10, 10, 10));
        draw_rect(&mut img, w, 16, 16, 48, 48, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        assert!(mask.iter().any(|&v| v == 1), "mask should contain keyed pixels");
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        assert_eq!(boxes.len(), 1, "expected exactly one box");
        assert!(boxes[0].min_x >= 16 && boxes[0].max_x <= 47);
        assert!(boxes[0].min_y >= 16 && boxes[0].max_y <= 47);
    }

    #[test]
    fn test_blurred_luminance_keying() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 16, 16, 48, 48, (200, 200, 200));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 100.0;
        cfg.blur_radius = 2.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        assert_eq!(boxes.len(), 1);
    }

    #[test]
    fn test_color_keying() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 8, 8, 24, 24, (255, 0, 0));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Color;
        cfg.color_target = aviutl2::filter::FilterConfigColorValue::from_rgb(255, 0, 0);
        cfg.threshold = 40.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].cr, 255);
        assert_eq!(boxes[0].cg, 0);
        assert_eq!(boxes[0].cb, 0);
    }

    #[test]
    fn test_motion_detection() {
        let w = 64usize;
        let h = 64usize;
        let frame1 = solid(w, h, (0, 0, 0));
        let mut frame2 = frame1.clone();
        draw_rect(&mut frame2, w, 16, 16, 32, 32, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::MotionDetection;
        cfg.threshold = 50.0;

        // 前フレームなし → 空マスク
        let mask = compute_key_mask(&frame2, w, h, &cfg, None);
        assert!(mask.iter().all(|&v| v == 0));

        // 前フレームあり → 変化領域を検出
        let mask = compute_key_mask(&frame2, w, h, &cfg, Some(&frame1));
        assert!(mask.iter().any(|&v| v == 1));
    }

    #[test]
    fn test_overlap_removal() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 8, 8, 56, 56, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.overlap_mode = OverlapMode::RemoveSmaller;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        assert!(boxes.len() <= 1, "overlapping boxes should be removed");
    }

    #[test]
    fn test_draw_overlay_changes_pixels() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 16, 16, 48, 48, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.box_enabled = true;
        cfg.stroke_enabled = true;
        cfg.stroke_color = aviutl2::filter::FilterConfigColorValue::from_rgb(0, 255, 0);
        cfg.fill_enabled = true;
        cfg.fill_mode = FillMode::Solid;
        cfg.fill_color = aviutl2::filter::FilterConfigColorValue::from_rgb(255, 0, 0);

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        assert!(!boxes.is_empty());

        let before = img.clone();
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);
        assert_ne!(img, before, "overlay drawing should modify pixels");
    }

    #[test]
    fn test_config_items_generated() {
        let items = FilterConfig::to_config_items();
        assert!(items.len() > 50, "expected many config items, got {}", items.len());
        let cfg: FilterConfig = items.as_slice().to_struct();
        assert_eq!(cfg.key_mode, KeyingMode::Luminance);
    }

    #[test]
    fn test_corner_only_stroke_stays_in_box() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 12, 12, 52, 52, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.stroke_enabled = true;
        cfg.stroke_color = aviutl2::filter::FilterConfigColorValue::from_rgb(0, 255, 0);
        cfg.stroke_width = 2.0;
        cfg.stroke_position = StrokePosition::Inside;
        cfg.box_corner_radius = 10.0;
        cfg.stroke_corner_mode = CornerMode::CornerOnly;
        // 角の長さを大きくしても、隣の角の弧やボックス外へ直線が伸びないこと
        cfg.stroke_corner_length = 100.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        assert_eq!(boxes.len(), 1);
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);

        let (min_x, min_y, max_x, max_y) = (12usize, 12usize, 51usize, 51usize);
        // 内側ストロークはボックス境界からはみ出さない (マージン 0 で検証)
        let margin = 0usize;
        let mut outside = 0usize;
        let mut stroke_count = 0usize;
        for y in 0..h {
            for x in 0..w {
                let p = img[y * w + x];
                let is_stroke = p.g > 100 && p.r < 100 && p.b < 100;
                if is_stroke {
                    stroke_count += 1;
                    if x + margin < min_x
                        || x > max_x + margin
                        || y + margin < min_y
                        || y > max_y + margin
                    {
                        outside += 1;
                    }
                }
            }
        }
        assert!(stroke_count > 0, "ストロークが描画されていません");
        assert_eq!(outside, 0, "内側ストロークがボックス外にはみ出しています");
    }

    #[test]
    fn test_corner_only_length_includes_arc() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 12, 12, 52, 52, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.stroke_enabled = true;
        cfg.stroke_color = aviutl2::filter::FilterConfigColorValue::from_rgb(0, 255, 0);
        cfg.stroke_width = 2.0;
        cfg.stroke_position = StrokePosition::Center;
        cfg.box_corner_radius = 10.0;
        cfg.stroke_corner_mode = CornerMode::CornerOnly;
        // 角の長さが弧の半分 (pi*10/2/2 ≈ 7.85) 未満の場合、
        // 直線の延長は出さず、部分弧だけで表現されるはず
        cfg.stroke_corner_length = 5.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);

        // 上辺 (y=12) に直線の延長がほとんど現れないこと
        let mut top_green = 0usize;
        for x in 0..w {
            let p = img[12 * w + x];
            if p.g > 100 && p.r < 100 {
                top_green += 1;
            }
        }
        assert!(
            top_green <= 8,
            "角の長さに角丸が含まれていない (上辺に直線が伸びている): {top_green}"
        );
    }

    #[test]
    fn test_corner_only_no_gap_with_rounded() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 12, 12, 52, 52, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.stroke_enabled = true;
        cfg.stroke_color = aviutl2::filter::FilterConfigColorValue::from_rgb(0, 255, 0);
        cfg.stroke_width = 2.0;
        cfg.stroke_position = StrokePosition::Center;
        cfg.box_corner_radius = 10.0;
        cfg.stroke_corner_mode = CornerMode::CornerOnly;
        // 直線部の長さ (lh = 19) に対して十分大きい角の長さ
        // 直線の延長 = 30 - pi*10/2/2 ≈ 22 → クランプで辺全体を覆う
        cfg.stroke_corner_length = 30.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);

        // 上辺 (y=12) の中央 (x=32) がストロークで埋まっていること (隙間がない)
        let p = img[12 * w + 32];
        assert!(
            p.g > 100 && p.r < 100,
            "角の長さが十分なのに辺に隙間が空いている: {p:?}"
        );
    }

    #[test]
    fn test_dilate_mask_connects() {
        let w = 10usize;
        let h = 10usize;
        let mut mask = vec![0u8; w * h];
        mask[4 * w + 5] = 1;
        mask[5 * w + 5] = 1;
        mask[6 * w + 5] = 1;
        let dilated = keyer::dilate_mask(&mask, w, h, 1);
        assert!(dilated[4 * w + 4] == 1);
        assert!(dilated[3 * w + 5] == 1);
        assert!(dilated[5 * w + 5] == 1);
        assert!(dilated[0] == 0);
    }

    #[test]
    fn test_corner_modes() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 12, 12, 52, 52, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.stroke_enabled = true;
        cfg.stroke_color = aviutl2::filter::FilterConfigColorValue::from_rgb(0, 255, 0);
        cfg.stroke_width = 2.0;
        cfg.stroke_position = StrokePosition::Center;
        cfg.stroke_corner_length = 30.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        assert_eq!(boxes.len(), 1);

        for shape in [
            BoxShape::Rectangle,
            BoxShape::Square,
            BoxShape::Ellipse,
            BoxShape::Circle,
        ] {
            cfg.box_shape = shape;
            for mode in [
                CornerMode::None,
                CornerMode::CornerOnly,
                CornerMode::HorizontalBracket,
                CornerMode::VerticalBracket,
                CornerMode::BracketLeft,
                CornerMode::BracketRight,
            ] {
                let mut img2 = img.clone();
                cfg.stroke_corner_mode = mode;
                let original = img2.clone();
                draw_overlay(&mut img2, w, h, &original, &boxes, &cfg);
                // どのモードでも何か描画され、クラッシュしないこと
                let green = img2.iter().filter(|p| p.g > 100 && p.r < 100).count();
                assert!(
                    green > 0,
                    "shape {:?} / mode {:?} でストロークが描画されていません",
                    shape, mode
                );
            }
        }
    }

    #[test]
    fn test_motion_cache_survives_rerender() {
        let ud = super::BloboxUserdata::new(0);
        let id = 7i64;
        let img = |v: u8| {
            let mut px = vec![RgbaPixel::default(); 16];
            for p in px.iter_mut() {
                *p = RgbaPixel {
                    r: v,
                    g: v,
                    b: v,
                    a: 255,
                };
            }
            px
        };

        assert!(ud.get_prev_frame(id, 0).is_none());
        ud.store_frame(id, 0, img(0));
        assert!(ud.get_prev_frame(id, 0).is_none());
        ud.store_frame(id, 0, img(0));

        assert!(ud.get_prev_frame(id, 1).is_some());
        ud.store_frame(id, 1, img(1));
        assert!(
            ud.get_prev_frame(id, 1).is_some(),
            "再レンダリングで直前フレームが失われると、フレーム毎にしか検出できない"
        );
        ud.store_frame(id, 1, img(1));

        assert!(ud.get_prev_frame(id, 2).is_some());
        ud.store_frame(id, 2, img(2));
        assert!(ud.get_prev_frame(id, 2).is_some());
        ud.store_frame(id, 2, img(2));

        let map = ud.prev_frames.lock().unwrap();
        assert!(map.get(&id).unwrap().len() <= super::MOTION_CACHE_FRAMES + 1);
    }

    #[test]
    fn test_motion_cache_clears_stale_frames() {
        let ud = super::BloboxUserdata::new(0);
        let id = 9i64;
        let img = |v: u8| {
            let mut px = vec![RgbaPixel::default(); 16];
            for p in px.iter_mut() {
                *p = RgbaPixel {
                    r: v,
                    g: v,
                    b: v,
                    a: 255,
                };
            }
            px
        };

        // 過去のレンダリングでフレーム 100〜103 がキャッシュされている状態
        ud.store_frame(id, 100, img(100));
        ud.store_frame(id, 101, img(101));
        ud.store_frame(id, 102, img(102));
        ud.store_frame(id, 103, img(103));

        // パラメータ変更などでフレーム 0 から再レンダリング
        assert!(ud.get_prev_frame(id, 0).is_none());
        ud.store_frame(id, 0, img(0));
        // 古い残骸 (100〜103) が消えているため、フレーム 1 ではフレーム 0 が使える
        assert!(
            ud.get_prev_frame(id, 1).is_some(),
            "再レンダリング時に古いフレームの残骸が残り、モーション検出が止まっている"
        );
        ud.store_frame(id, 1, img(1));
        assert!(ud.get_prev_frame(id, 2).is_some());
    }

    #[test]
    fn test_fill_binarize() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 16, 16, 48, 48, (128, 128, 128));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.fill_enabled = true;
        cfg.fill_mode = FillMode::Binarize;
        cfg.fill_binarize_threshold = 100.0;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        sample_box_colors(&img, w, h, &mut boxes);
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);
        let px = img[32 * w + 32];
        assert!(px.r >= 200 && px.g >= 200 && px.b >= 200);
    }

    #[test]
    fn test_fill_noise_modes() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 16, 16, 48, 48, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.fill_enabled = true;

        for mode in [FillMode::ColorNoise, FillMode::Noise] {
            let mut img2 = img.clone();
            cfg.fill_mode = mode;
            let mask = compute_key_mask(&img2, w, h, &cfg, None);
            let mut boxes = find_boxes(&mask, w, h);
            filter_boxes(&mut boxes, w, h, &cfg);
            sample_box_colors(&img2, w, h, &mut boxes);
            let original = img2.clone();
            draw_overlay(&mut img2, w, h, &original, &boxes, &cfg);
            let px = img2[32 * w + 32];
            assert!(px.r != 255 || px.g != 255 || px.b != 255);
        }
    }

    #[test]
    fn test_fill_random() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 8, 8, 24, 24, (255, 255, 255));
        draw_rect(&mut img, w, 40, 40, 56, 56, (128, 128, 128));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.fill_enabled = true;
        cfg.fill_mode = FillMode::Random;

        let mask = compute_key_mask(&img, w, h, &cfg, None);
        let mut boxes = find_boxes(&mask, w, h);
        filter_boxes(&mut boxes, w, h, &cfg);
        assert!(boxes.len() >= 2);
        sample_box_colors(&img, w, h, &mut boxes);
        let original = img.clone();
        draw_overlay(&mut img, w, h, &original, &boxes, &cfg);
    }

    #[test]
    fn test_line_modes_smoke() {
        let w = 64usize;
        let h = 64usize;
        let mut img = solid(w, h, (0, 0, 0));
        draw_rect(&mut img, w, 8, 8, 20, 20, (255, 255, 255));
        draw_rect(&mut img, w, 30, 30, 42, 42, (255, 255, 255));
        draw_rect(&mut img, w, 30, 8, 42, 20, (255, 255, 255));

        let mut cfg = FilterConfig::default();
        cfg.key_mode = KeyingMode::Luminance;
        cfg.luma_target = LumaTarget::Brighter;
        cfg.threshold = 50.0;
        cfg.blur_radius = 0.0;
        cfg.line_enabled = true;
        cfg.arrow_enabled = true;

        for line_type in [
            LineType::Sequential,
            LineType::ConnectAll,
            LineType::MinimumSpanningTree,
        ] {
            cfg.line_type = line_type;
            for order in [
                LineOrder::TopToBottom,
                LineOrder::DistanceFromCenter,
                LineOrder::NearestNeighbor,
                LineOrder::PolarSweep,
            ] {
                cfg.line_order = order;
                cfg.line_connect_limit = true;
                cfg.line_connect_max = 2.0;
                cfg.line_curve = order == LineOrder::NearestNeighbor;
                let mask = compute_key_mask(&img, w, h, &cfg, None);
                let mut boxes = find_boxes(&mask, w, h);
                filter_boxes(&mut boxes, w, h, &cfg);
                sample_box_colors(&img, w, h, &mut boxes);
                let original = img.clone();
                let mut img2 = img.clone();
                draw_overlay(&mut img2, w, h, &original, &boxes, &cfg);
            }
        }
    }

    #[test]
    fn test_wildcard_expansion() {
        let mut rng = wildcard::Rng::new(42);
        let ctx = wildcard::BoxContext {
            x: 10.0,
            y: 20.0,
            w: 30.0,
            h: 40.0,
            r: 255,
            g: 128,
            b: 0,
            pixels: 1200,
            total_pixels: 4096,
            id: 3,
        };
        assert_eq!(
            wildcard::expand_wildcards("$[box_x_position]", &ctx, &mut rng),
            "10"
        );
        assert_eq!(
            wildcard::expand_wildcards("$[box_width]x$[box_height]", &ctx, &mut rng),
            "30x40"
        );
        assert_eq!(
            wildcard::expand_wildcards("$[box_pixel_percent_1]", &ctx, &mut rng),
            "29.3"
        );
        assert_eq!(wildcard::expand_wildcards("$[box_id]", &ctx, &mut rng), "3");
        assert_eq!(
            wildcard::expand_wildcards("$[box_x_position + 10]", &ctx, &mut rng),
            "20"
        );
        let v = wildcard::expand_wildcards("$[random_int_1,10]", &ctx, &mut rng);
        assert!(v.parse::<i64>().is_ok());
        assert_eq!(
            wildcard::expand_wildcards("$[unknown]", &ctx, &mut rng),
            "$[unknown]"
        );
    }
}
