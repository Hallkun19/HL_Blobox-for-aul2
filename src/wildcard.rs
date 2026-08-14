//! テキストテンプレートのワイルドカード展開 (`$[...]`)。
//!
//! 仕様: ワイルドカード一覧.txt を参照。

/// シンプルな決定論的 PRNG (xorshift64* 系)。
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545F4914F6CDD1D)) >> 32) as u32
    }
    pub fn next_range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u64;
        lo + (self.next_u32() as u64 % span) as i64
    }
    pub fn next_float(&mut self, lo: f64, hi: f64) -> f64 {
        if hi < lo {
            return lo;
        }
        let t = self.next_u32() as f64 / u32::MAX as f64;
        lo + (hi - lo) * t
    }
    pub fn choice<T: Clone>(&mut self, options: &[T]) -> T {
        let i = (self.next_u32() as usize) % options.len();
        options[i].clone()
    }
    pub fn random_from(&mut self, charset: &str, len: usize) -> String {
        let bytes = charset.as_bytes();
        let mut out = String::with_capacity(len);
        for _ in 0..len {
            out.push(bytes[(self.next_u32() as usize) % bytes.len()] as char);
        }
        out
    }
}

/// 1ボックス分のワイルドカード展開コンテキスト。
pub struct BoxContext {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub pixels: i64,
    pub total_pixels: i64,
    pub id: i32,
}

const ALPHABET_MIX: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ALPHABET_UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ALPHABET_LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const ALNUM_MIX: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const ALNUM_UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const ALNUM_LOWER: &str = "abcdefghijklmnopqrstuvwxyz0123456789";
const SPECIAL: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";
const HEXDIGITS: &str = "0123456789abcdef";
const ALL_MIX: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

fn parse_int(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

fn trim_float(v: f64, decimals: i32) -> String {
    if decimals >= 0 {
        format!("{:.*}", decimals as usize, v)
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        let mut s = format!("{:.6}", v);
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

fn hex_byte(v: u8, upper: bool) -> String {
    if upper {
        format!("{:02X}", v)
    } else {
        format!("{:02x}", v)
    }
}

// ---------------------------------------------------------------------
// 四則演算式の評価 (ボックスの数値変数を使用可能)
// ---------------------------------------------------------------------
struct ExprParser<'a> {
    s: &'a [u8],
    ctx: &'a BoxContext,
    pos: usize,
    ok: bool,
}

impl<'a> ExprParser<'a> {
    fn new(s: &'a str, ctx: &'a BoxContext) -> Self {
        ExprParser {
            s: s.as_bytes(),
            ctx,
            pos: 0,
            ok: true,
        }
    }
    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos] == b' ' {
            self.pos += 1;
        }
    }
    fn is_ident_char(c: u8) -> bool {
        c.is_ascii_alphanumeric() || c == b'_'
    }
    fn try_ident(&mut self) -> Option<f64> {
        let start = self.pos;
        while self.pos < self.s.len() && Self::is_ident_char(self.s[self.pos]) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.s[start..self.pos]).unwrap_or("");
        match name {
            "box_x_position" => Some(self.ctx.x),
            "box_y_position" => Some(self.ctx.y),
            "box_width" => Some(self.ctx.w),
            "box_height" => Some(self.ctx.h),
            "box_color_r" => Some(self.ctx.r as f64),
            "box_color_g" => Some(self.ctx.g as f64),
            "box_color_b" => Some(self.ctx.b as f64),
            "box_pixel_count" => Some(self.ctx.pixels as f64),
            "box_id" => Some(self.ctx.id as f64),
            _ => {
                self.ok = false;
                None
            }
        }
    }
    fn number(&mut self) -> f64 {
        self.skip_ws();
        if self.pos >= self.s.len() {
            self.ok = false;
            return 0.0;
        }
        let c = self.s[self.pos];
        if c == b'(' {
            self.pos += 1;
            let v = self.expr();
            self.skip_ws();
            if self.pos < self.s.len() && self.s[self.pos] == b')' {
                self.pos += 1;
            } else {
                self.ok = false;
            }
            return v;
        }
        if c.is_ascii_digit() || c == b'.' {
            let start = self.pos;
            while self.pos < self.s.len() {
                let cc = self.s[self.pos];
                if cc.is_ascii_digit() || cc == b'.' || cc == b'e' || cc == b'E' {
                    self.pos += 1;
                } else if (cc == b'+' || cc == b'-')
                    && self.pos > start
                    && (self.s[self.pos - 1] == b'e' || self.s[self.pos - 1] == b'E')
                {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            let text = std::str::from_utf8(&self.s[start..self.pos]).unwrap_or("0");
            text.parse::<f64>().unwrap_or_else(|_| {
                self.ok = false;
                0.0
            })
        } else if c.is_ascii_alphabetic() {
            self.try_ident().unwrap_or_else(|| {
                self.ok = false;
                0.0
            })
        } else {
            self.ok = false;
            0.0
        }
    }
    fn factor(&mut self) -> f64 {
        self.skip_ws();
        if self.pos < self.s.len() && self.s[self.pos] == b'-' {
            self.pos += 1;
            return -self.factor();
        }
        if self.pos < self.s.len() && self.s[self.pos] == b'+' {
            self.pos += 1;
            return self.factor();
        }
        self.number()
    }
    fn term(&mut self) -> f64 {
        let mut v = self.factor();
        while self.ok {
            self.skip_ws();
            if self.pos < self.s.len() && self.s[self.pos] == b'*' {
                self.pos += 1;
                v *= self.factor();
            } else if self.pos < self.s.len() && self.s[self.pos] == b'/' {
                self.pos += 1;
                let d = self.factor();
                if d == 0.0 {
                    self.ok = false;
                    break;
                }
                v /= d;
            } else {
                break;
            }
        }
        v
    }
    fn expr(&mut self) -> f64 {
        let mut v = self.term();
        while self.ok {
            self.skip_ws();
            if self.pos < self.s.len() && self.s[self.pos] == b'+' {
                self.pos += 1;
                v += self.term();
            } else if self.pos < self.s.len() && self.s[self.pos] == b'-' {
                self.pos += 1;
                v -= self.term();
            } else {
                break;
            }
        }
        v
    }
    fn run(&mut self) -> Option<f64> {
        let v = self.expr();
        self.skip_ws();
        if self.ok && self.pos == self.s.len() {
            Some(v)
        } else {
            None
        }
    }
}

/// 1つの `$[...]` ブロックを展開する。失敗時は空文字列。
fn eval_wildcard(inner: &str, ctx: &BoxContext, rng: &mut Rng) -> String {
    // --- ランダム系 ---
    if let Some(rest) = inner.strip_prefix("random_alphanumeric_upper_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALNUM_UPPER, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_alphanumeric_lower_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALNUM_LOWER, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_alphanumeric_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALNUM_MIX, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_alphabet_upper_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALPHABET_UPPER, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_alphabet_lower_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALPHABET_LOWER, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_alphabet_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALPHABET_MIX, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_special_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(SPECIAL, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_hex_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(HEXDIGITS, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_string_") {
        if let Some(len) = parse_int(rest) {
            return rng.random_from(ALL_MIX, len as usize);
        }
    }
    if let Some(rest) = inner.strip_prefix("random_choice_") {
        let opts: Vec<&str> = rest.split(',').collect();
        if !opts.is_empty() {
            return rng.choice(&opts).to_string();
        }
    }
    if let Some(rest) = inner.strip_prefix("random_float_") {
        let parts: Vec<&str> = rest.split(',').collect();
        if parts.len() == 2 {
            if let (Ok(lo), Ok(hi)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                return trim_float(rng.next_float(lo, hi), -1);
            }
        }
    }
    if let Some(rest) = inner.strip_prefix("random_int_") {
        let parts: Vec<&str> = rest.split(',').collect();
        if parts.len() == 2 {
            if let (Ok(lo), Ok(hi)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
                return rng.next_range(lo, hi).to_string();
            }
        }
    }
    if inner == "random_bool" {
        return if rng.next_u32() & 1 == 1 {
            "True".to_string()
        } else {
            "False".to_string()
        };
    }

    // --- ボックスメタデータ系 ---
    if let Some(rest) = inner.strip_prefix("box_pixel_percent_") {
        if let Ok(sig) = rest.parse::<i32>() {
            if (0..=5).contains(&sig) {
                let pct = ctx.pixels as f64 / ctx.total_pixels.max(1) as f64 * 100.0;
                return trim_float(pct, sig);
            }
        }
    }
    if inner == "box_color_rgb_hex_upper" {
        return format!("{:02X}{:02X}{:02X}", ctx.r, ctx.g, ctx.b);
    }
    if inner == "box_color_rgb_hex_lower" {
        return format!("{:02x}{:02x}{:02x}", ctx.r, ctx.g, ctx.b);
    }
    if inner == "box_color_rgb" {
        return format!(
            "{:03},{:03},{:03}",
            ctx.r, ctx.g, ctx.b
        );
    }
    if inner == "box_color_r_hex_upper" {
        return hex_byte(ctx.r, true);
    }
    if inner == "box_color_g_hex_upper" {
        return hex_byte(ctx.g, true);
    }
    if inner == "box_color_b_hex_upper" {
        return hex_byte(ctx.b, true);
    }
    if inner == "box_color_r_hex_lower" {
        return hex_byte(ctx.r, false);
    }
    if inner == "box_color_g_hex_lower" {
        return hex_byte(ctx.g, false);
    }
    if inner == "box_color_b_hex_lower" {
        return hex_byte(ctx.b, false);
    }
    if inner == "box_color_r" {
        return ctx.r.to_string();
    }
    if inner == "box_color_g" {
        return ctx.g.to_string();
    }
    if inner == "box_color_b" {
        return ctx.b.to_string();
    }
    if inner == "box_pixel_count" {
        return ctx.pixels.to_string();
    }
    if inner == "box_id" {
        return ctx.id.to_string();
    }

    // --- 四則演算 ---
    let mut parser = ExprParser::new(inner, ctx);
    if let Some(v) = parser.run() {
        return trim_float(v, -1);
    }

    String::new()
}

/// テキスト内の `$[...]` を展開する。未知・不正なものはそのまま残す。
pub fn expand_wildcards(text: &str, ctx: &BoxContext, rng: &mut Rng) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            // 閉じ括弧を探す
            if let Some(rel) = text[i + 2..].find(']') {
                let close = i + 2 + rel;
                let inner = &text[i + 2..close];
                let expanded = eval_wildcard(inner, ctx, rng);
                if expanded.is_empty() {
                    // 展開失敗 → そのまま
                    out.push_str(&text[i..=close]);
                } else {
                    out.push_str(&expanded);
                }
                i = close + 1;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
