//! The `tiles` layout: a screen made of rounded, semi-transparent tiles (clock,
//! CPU/GPU load and temperature, watts, RAM, network, ...), each with a big value and
//! a progress bar or a ring, in the style of modern "glass" dashboards.
//!
//! * `layout = tiles` shows it; `tiles = ...` chooses which tiles, in which order and
//!   rows (`|` starts a new row), and whether a tile uses a bar or a ring.
//! * `tiles_font = smooth | pixel` picks anti-aliased text (Chakra Petch, see
//!   `smooth_font.rs`) or the pixel font used by the rest of the program.
//! * `tiles_bg` chooses what is behind the tiles; `tiles_bar_color` and
//!   `tile_<name>_color` the colors of the bars and rings.
//!
//! The tiles are painted on a transparent RGBA layer that is only re-drawn when
//! something shown changes (a value, the time, a setting); every frame just lays the
//! layer over the background, so an animated background costs little.

use super::*;
use crate::layouts::WidgetData;
use crate::smooth_font::{self, Weight};
use std::path::PathBuf;

type Rgb = (u8, u8, u8);

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Clock,
    Cpu,
    CpuTemp,
    CpuWatt,
    Gpu,
    GpuTemp,
    GpuWatt,
    Ram,
    Net,
    Disk,
    Fps,
    Weather,
    Music,
}

impl Kind {
    pub const ALL: [Kind; 13] = [
        Kind::Clock, Kind::Cpu, Kind::CpuTemp, Kind::CpuWatt, Kind::Gpu, Kind::GpuTemp, Kind::GpuWatt,
        Kind::Ram, Kind::Net, Kind::Disk, Kind::Fps, Kind::Weather, Kind::Music,
    ];
    pub const NAMES: &'static str =
        "clock, cpu, cpu_temp, cpu_watt, gpu, gpu_temp, gpu_watt, ram, net, disk, fps, weather, music";

    pub fn key(self) -> &'static str {
        match self {
            Kind::Clock => "clock",
            Kind::Cpu => "cpu",
            Kind::CpuTemp => "cpu_temp",
            Kind::CpuWatt => "cpu_watt",
            Kind::Gpu => "gpu",
            Kind::GpuTemp => "gpu_temp",
            Kind::GpuWatt => "gpu_watt",
            Kind::Ram => "ram",
            Kind::Net => "net",
            Kind::Disk => "disk",
            Kind::Fps => "fps",
            Kind::Weather => "weather",
            Kind::Music => "music",
        }
    }

    fn index(self) -> usize {
        Kind::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }

    pub fn parse(s: &str) -> Option<Kind> {
        let t = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match t.as_str() {
            "clock" | "time" | "orologio" => Kind::Clock,
            "cpu" | "cpu_load" | "cpu_usage" => Kind::Cpu,
            "cpu_temp" | "cputemp" | "cpu_temperature" => Kind::CpuTemp,
            "cpu_watt" | "cpu_power" | "cpu_w" => Kind::CpuWatt,
            "gpu" | "gpu_load" | "gpu_usage" => Kind::Gpu,
            "gpu_temp" | "gputemp" | "gpu_temperature" => Kind::GpuTemp,
            "gpu_watt" | "gpu_power" | "gpu_w" => Kind::GpuWatt,
            "ram" | "mem" | "memory" => Kind::Ram,
            "net" | "network" | "rete" => Kind::Net,
            "disk" | "disco" => Kind::Disk,
            "fps" => Kind::Fps,
            "weather" | "meteo" => Kind::Weather,
            "music" | "nowplaying" | "now_playing" | "musica" => Kind::Music,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    Bar,
    Ring,
}

/// One entry of `tiles = ...`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TileSpec {
    pub kind: Kind,
    pub look: Look,
    /// Spans every row (only the first tile of the first row; landscape).
    pub tall: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontStyle {
    Smooth,
    Pixel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Dark,
    Light,
}

/// Colors of a bar / ring: `Auto` = green-yellow-red by level; one color = solid;
/// several = a gradient along the bar or ring.
#[derive(Clone, PartialEq, Debug)]
pub enum BarColor {
    Auto,
    Colors(Vec<Rgb>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BgMode {
    /// Whatever is behind the interface on the other screens (background image/effect or dark).
    Global,
    Color,
    Gradient,
    Effect,
    Image,
}

impl BgMode {
    pub const NAMES: &'static str = "global | color | gradient | effect | image";
    pub fn parse(s: &str) -> Option<BgMode> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "global" | "default" | "auto" | "none" => BgMode::Global,
            "color" | "solid" | "colore" => BgMode::Color,
            "gradient" | "gradiente" => BgMode::Gradient,
            "effect" | "animated" | "plasma" | "effetto" => BgMode::Effect,
            "image" | "immagine" | "picture" => BgMode::Image,
            _ => return None,
        })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    pub rows: Vec<Vec<TileSpec>>,
    pub font: FontStyle,
    pub theme: Theme,
    pub fill: Option<Rgb>,
    /// Tile opacity 0-100 (None = the theme's own).
    pub opacity: Option<u32>,
    pub text: Option<Rgb>,
    pub radius: Option<u32>,
    pub gap: u32,
    pub bar: BarColor,
    pub bar_by_kind: Vec<Option<BarColor>>,
    pub cpu_watt_max: u32,
    pub gpu_watt_max: u32,
    pub fps_max: u32,
    pub clock_seconds: bool,
    pub bg: BgMode,
    pub bg_colors: Vec<Rgb>,
    pub bg_image: Option<PathBuf>,
    /// 0-100: darkens the background image / effect.
    pub bg_dim: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let t = |kind, look, tall| TileSpec { kind, look, tall };
        Settings {
            rows: vec![
                vec![t(Kind::Clock, Look::Bar, true), t(Kind::CpuTemp, Look::Bar, false), t(Kind::GpuTemp, Look::Bar, false)],
                vec![t(Kind::Ram, Look::Ring, false), t(Kind::Cpu, Look::Ring, false), t(Kind::Gpu, Look::Ring, false)],
            ],
            font: FontStyle::Smooth,
            theme: Theme::Dark,
            fill: None,
            opacity: None,
            text: None,
            radius: None,
            gap: 14,
            bar: BarColor::Auto,
            bar_by_kind: vec![None; Kind::ALL.len()],
            cpu_watt_max: 200,
            gpu_watt_max: 400,
            fps_max: 240,
            clock_seconds: false,
            bg: BgMode::Global,
            bg_colors: vec![(0x1b, 0x2a, 0x4a), (0x4a, 0x1b, 0x3a)],
            bg_image: None,
            bg_dim: 0,
        }
    }
}

impl Settings {
    fn palette(&self) -> Palette {
        let (fill, opacity, text) = match self.theme {
            Theme::Dark => ((0x14, 0x18, 0x26), 66, (0xFF, 0xFF, 0xFF)),
            Theme::Light => ((0xFF, 0xFF, 0xFF), 52, (0x1E, 0x26, 0x3C)),
        };
        Palette {
            fill: self.fill.unwrap_or(fill),
            alpha: (self.opacity.unwrap_or(opacity).min(100) * 255 / 100) as u8,
            text: self.text.unwrap_or(text),
            light: self.theme == Theme::Light,
        }
    }

    pub fn bar_color(&self, kind: Kind) -> &BarColor {
        self.bar_by_kind.get(kind.index()).and_then(|o| o.as_ref()).unwrap_or(&self.bar)
    }
}

struct Palette {
    fill: Rgb,
    alpha: u8,
    text: Rgb,
    light: bool,
}

fn parse_rgb(key: &str, raw: &str) -> anyhow::Result<Rgb> {
    match parse_color(raw.trim()).map_err(|e| anyhow::anyhow!("{key}: {e}"))? {
        ColorMode::Custom(r, g, b) => Ok((r, g, b)),
        ColorMode::Default => anyhow::bail!("{key}: 'default' is not a color"),
    }
}

fn parse_rgb_list(key: &str, raw: &str) -> anyhow::Result<Vec<Rgb>> {
    raw.split(',').filter(|p| !p.trim().is_empty()).map(|p| parse_rgb(key, p)).collect()
}

/// `auto`, one color or several (`#FF0080, #00FFFF` = a gradient).
fn parse_bar_color(key: &str, raw: &str) -> anyhow::Result<BarColor> {
    let t = raw.trim();
    if t.eq_ignore_ascii_case("auto") || t.eq_ignore_ascii_case("default") || t.is_empty() {
        return Ok(BarColor::Auto);
    }
    Ok(BarColor::Colors(parse_rgb_list(key, t)?))
}

/// `tiles = clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, gpu:ring`
pub fn parse_tiles(raw: &str) -> anyhow::Result<Vec<Vec<TileSpec>>> {
    let mut rows = Vec::new();
    for (ri, row) in raw.split('|').enumerate() {
        let mut out = Vec::new();
        for (ci, item) in row.split(',').enumerate() {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            let mut parts = item.split(':');
            let name = parts.next().unwrap_or("");
            let kind = Kind::parse(name)
                .ok_or_else(|| anyhow::anyhow!("tiles: '{name}' is not a tile ({})", Kind::NAMES))?;
            let (mut look, mut tall) = (Look::Bar, false);
            for m in parts {
                match m.trim().to_ascii_lowercase().as_str() {
                    "ring" | "circle" | "anello" => look = Look::Ring,
                    "bar" | "barra" => look = Look::Bar,
                    "tall" | "alto" => tall = true,
                    other => anyhow::bail!("tiles: '{other}' is not valid after '{name}:' (ring | bar | tall)"),
                }
            }
            if tall && !(ri == 0 && out.is_empty() && ci == 0) {
                anyhow::bail!("tiles: 'tall' only works on the first tile of the first row");
            }
            out.push(TileSpec { kind, look, tall });
        }
        if !out.is_empty() {
            rows.push(out);
        }
    }
    if rows.is_empty() {
        anyhow::bail!("tiles: the list is empty ({})", Kind::NAMES);
    }
    let total: usize = rows.iter().map(Vec::len).sum();
    if total > 16 {
        anyhow::bail!("tiles: at most 16 tiles (you listed {total})");
    }
    Ok(rows)
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    if let Some(v) = file.get("tiles") {
        s.rows = parse_tiles(v)?;
    }
    if let Some(v) = file.get("tiles_font") {
        s.font = match v.trim().to_ascii_lowercase().as_str() {
            "smooth" | "liscio" | "vector" | "ttf" => FontStyle::Smooth,
            "pixel" | "normal" | "normale" | "bitmap" => FontStyle::Pixel,
            _ => anyhow::bail!("tiles_font: '{v}' not valid (smooth | pixel)"),
        };
    }
    if let Some(v) = file.get("tiles_theme") {
        s.theme = match v.trim().to_ascii_lowercase().as_str() {
            "dark" | "scuro" => Theme::Dark,
            "light" | "chiaro" => Theme::Light,
            _ => anyhow::bail!("tiles_theme: '{v}' not valid (dark | light)"),
        };
    }
    if let Some(v) = file.get("tiles_color") {
        s.fill = Some(parse_rgb("tiles_color", v)?);
    }
    if let Some(v) = file.get("tiles_text_color") {
        s.text = Some(parse_rgb("tiles_text_color", v)?);
    }
    let num = |key: &str, lo: u32, hi: u32| -> anyhow::Result<Option<u32>> {
        match file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            None => Ok(None),
            Some(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
            Some(v) => anyhow::bail!("{key}: must be between {lo} and {hi} (got {v})"),
        }
    };
    s.opacity = num("tiles_opacity", 0, 100)?;
    s.radius = num("tiles_radius", 0, 80)?;
    if let Some(v) = num("tiles_gap", 0, 60)? {
        s.gap = v;
    }
    if let Some(v) = num("tiles_cpu_watt_max", 10, 2000)? {
        s.cpu_watt_max = v;
    }
    if let Some(v) = num("tiles_gpu_watt_max", 10, 2000)? {
        s.gpu_watt_max = v;
    }
    if let Some(v) = num("tiles_fps_max", 30, 1000)? {
        s.fps_max = v;
    }
    if let Some(v) = file.get_bool("tiles_clock_seconds").map_err(|e| anyhow::anyhow!(e))? {
        s.clock_seconds = v;
    }
    if let Some(v) = file.get("tiles_bar_color") {
        s.bar = parse_bar_color("tiles_bar_color", v)?;
    }
    for k in Kind::ALL {
        let key = format!("tile_{}_color", k.key());
        if let Some(v) = file.get(&key) {
            s.bar_by_kind[k.index()] = Some(parse_bar_color(&key, v)?);
        }
    }
    if let Some(v) = file.get("tiles_bg") {
        s.bg = BgMode::parse(v).ok_or_else(|| anyhow::anyhow!("tiles_bg: '{v}' not valid ({})", BgMode::NAMES))?;
    }
    if let Some(v) = file.get("tiles_bg_color") {
        let list = parse_rgb_list("tiles_bg_color", v)?;
        if list.is_empty() {
            anyhow::bail!("tiles_bg_color: needs at least one color, e.g. #1B2A4A or #1B2A4A, #4A1B3A");
        }
        s.bg_colors = list;
    }
    if let Some(v) = file.get("tiles_bg_image") {
        s.bg_image = Some(PathBuf::from(v));
    }
    if s.bg == BgMode::Image && s.bg_image.is_none() {
        anyhow::bail!("tiles_bg = image needs tiles_bg_image = <file>");
    }
    s.bg_dim = num("tiles_bg_dim", 0, 100)?.unwrap_or(0);
    Ok(s)
}

// -------------------------------------------------------------------- layout

pub type Rect = (u32, u32, u32, u32);

fn weight(kind: Kind) -> f32 {
    if kind == Kind::Clock || kind == Kind::Weather || kind == Kind::Music {
        1.5
    } else {
        1.0
    }
}

/// Splits `total` pixels (minus the gaps) among `weights`: returns (x offset, width) pairs.
fn split(total: u32, gap: u32, weights: &[f32]) -> Vec<(u32, u32)> {
    let n = weights.len() as u32;
    if n == 0 {
        return Vec::new();
    }
    let avail = total.saturating_sub(gap * (n - 1)) as f32;
    let sum: f32 = weights.iter().sum();
    let mut out = Vec::new();
    let (mut prev, mut acc) = (0u32, 0f32);
    for (i, w) in weights.iter().enumerate() {
        acc += w;
        let end = if i as u32 == n - 1 { avail.round() as u32 } else { (avail * acc / sum).round() as u32 };
        out.push((prev + gap * i as u32, end.saturating_sub(prev)));
        prev = end;
    }
    out
}

/// Position of every tile inside `area` (x, y, w, h).
pub fn cells(s: &Settings, area: Rect) -> Vec<(TileSpec, Rect)> {
    let (ax, ay, aw, ah) = area;
    let gap = s.gap;
    let (x0, y0) = (ax + gap, ay + gap);
    let (w, h) = (aw.saturating_sub(gap * 2), ah.saturating_sub(gap * 2));
    if w < 40 || h < 40 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if h > w {
        // Portrait: one column (two when there are many), every tile the same size.
        let all: Vec<TileSpec> = s.rows.iter().flatten().copied().collect();
        let cols = if all.len() > 6 { 2 } else { 1 };
        let rows = (all.len() as u32).div_ceil(cols);
        let cw = w.saturating_sub(gap * (cols - 1)) / cols;
        let ch = h.saturating_sub(gap * (rows - 1)) / rows;
        for (i, t) in all.into_iter().enumerate() {
            let (c, r) = (i as u32 % cols, i as u32 / cols);
            out.push((TileSpec { tall: false, ..t }, (x0 + c * (cw + gap), y0 + r * (ch + gap), cw, ch)));
        }
        return out;
    }
    let nrows = s.rows.len() as u32;
    let rh = h.saturating_sub(gap * (nrows - 1)) / nrows;
    let first_tall = nrows >= 2 && s.rows[0].first().is_some_and(|t| t.tall);
    // A tall tile takes its share of the first row's width over every row; the other
    // tiles of every row share what is left beside it.
    let (rest_x, rest_w) = if first_tall {
        let weights: Vec<f32> = s.rows[0].iter().map(|t| weight(t.kind)).collect();
        let (_, tw) = split(w, gap, &weights)[0];
        out.push((s.rows[0][0], (x0, y0, tw, h)));
        (tw + gap, w.saturating_sub(tw + gap))
    } else {
        (0, w)
    };
    for (ri, row) in s.rows.iter().enumerate() {
        let tiles: Vec<&TileSpec> = row.iter().skip(if first_tall && ri == 0 { 1 } else { 0 }).collect();
        let weights: Vec<f32> = tiles.iter().map(|t| weight(t.kind)).collect();
        let ry = y0 + ri as u32 * (rh + gap);
        for (t, (cx, cw)) in tiles.into_iter().zip(split(rest_w, gap, &weights)) {
            out.push((TileSpec { tall: false, ..*t }, (x0 + rest_x + cx, ry, cw, rh)));
        }
    }
    out
}

// --------------------------------------------------------------------- layer

/// Premultiplied-RGBA picture the tiles are drawn on.
pub struct Layer {
    pub(crate) w: usize,
    pub(crate) h: usize,
    px: Vec<u8>,
}

impl Layer {
    pub fn new(w: u32, h: u32) -> Self {
        Layer { w: w as usize, h: h as usize, px: vec![0; (w * h * 4) as usize] }
    }

    pub(crate) fn clear(&mut self) {
        self.px.iter_mut().for_each(|b| *b = 0);
    }

    /// Source-over: `rgb` (not premultiplied) at `alpha` 0-255 onto pixel (x, y).
    #[inline]
    pub(crate) fn over(&mut self, x: i32, y: i32, rgb: Rgb, alpha: u8) {
        if alpha == 0 || x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        let a = alpha as u32;
        let inv = 255 - a;
        let d = &mut self.px[i..i + 4];
        d[0] = ((rgb.0 as u32 * a + d[0] as u32 * inv + 127) / 255) as u8;
        d[1] = ((rgb.1 as u32 * a + d[1] as u32 * inv + 127) / 255) as u8;
        d[2] = ((rgb.2 as u32 * a + d[2] as u32 * inv + 127) / 255) as u8;
        d[3] = (a + (d[3] as u32 * inv + 127) / 255).min(255) as u8;
    }

    /// Multiplies the whole layer's opacity by `f` (0-1); pixels are premultiplied.
    pub(crate) fn scale_alpha(&mut self, f: f32) {
        let m = (f.clamp(0.0, 1.0) * 256.0) as u32;
        for v in self.px.iter_mut() {
            *v = ((*v as u32 * m) >> 8) as u8;
        }
    }

    /// Overwrites the whole layer with `other` (same size), e.g. a cached background.
    pub(crate) fn copy_from(&mut self, other: &Layer) {
        if other.w == self.w && other.h == self.h {
            self.px.copy_from_slice(&other.px);
        }
    }

    /// Fills the entire layer from tightly packed RGB888 pixels (alpha = 255).
    pub(crate) fn fill_opaque_rgb(&mut self, rgb: &[u8]) {
        if rgb.len() != self.w * self.h * 3 {
            return;
        }
        for (src, dst) in rgb.chunks_exact(3).zip(self.px.chunks_exact_mut(4)) {
            dst[0] = src[0];
            dst[1] = src[1];
            dst[2] = src[2];
            dst[3] = 255;
        }
    }

    /// Composites non-premultiplied RGBA8888 pixels at `(ox, oy)`.
    pub(crate) fn blit_rgba(&mut self, ox: i32, oy: i32, rw: u32, rh: u32, rgba: &[u8]) {
        if rgba.len() < (rw * rh * 4) as usize {
            return;
        }
        for yy in 0..rh as i32 {
            let dy = oy + yy;
            if dy < 0 || dy as usize >= self.h {
                continue;
            }
            for xx in 0..rw as i32 {
                let dx = ox + xx;
                if dx < 0 || dx as usize >= self.w {
                    continue;
                }
                let si = (yy as usize * rw as usize + xx as usize) * 4;
                let a = rgba[si + 3];
                if a > 0 {
                    self.over(dx, dy, (rgba[si], rgba[si + 1], rgba[si + 2]), a);
                }
            }
        }
    }

    /// Copies this layer, turned by 90 degrees, into `dst` (which must be `h` wide and `w` high):
    /// clockwise by default, counter-clockwise when `ccw`.
    pub(crate) fn rotate_into(&self, dst: &mut Layer, ccw: bool) {
        if dst.w != self.h || dst.h != self.w {
            *dst = Layer::new(self.h as u32, self.w as u32);
        }
        let (sw, sh) = (self.w, self.h);
        let dw = dst.w;
        for y in 0..dst.h {
            for x in 0..dw {
                // panel (x, y) <- canvas (cx, cy)
                let (cx, cy) = if ccw { (sw - 1 - y, x) } else { (y, sh - 1 - x) };
                let si = (cy * sw + cx) * 4;
                let di = (y * dw + x) * 4;
                dst.px[di..di + 4].copy_from_slice(&self.px[si..si + 4]);
            }
        }
    }

    /// Lays the layer over `fb` with its top-left corner at (ox, oy).
    pub fn composite_onto(&self, fb: &mut Framebuffer, ox: u32, oy: u32) {
        let fw = fb.width() as usize;
        let fh = fb.height() as usize;
        let (ox, oy) = (ox as usize, oy as usize);
        if ox >= fw || oy >= fh {
            return;
        }
        let w = self.w.min(fw - ox);
        let h = self.h.min(fh - oy);
        let dst = fb.as_bytes_mut();
        for row in 0..h {
            let s = &self.px[row * self.w * 4..row * self.w * 4 + w * 4];
            let d0 = ((oy + row) * fw + ox) * 3;
            let d = &mut dst[d0..d0 + w * 3];
            for (sp, dp) in s.chunks_exact(4).zip(d.chunks_exact_mut(3)) {
                let a = sp[3] as u32;
                if a == 0 {
                    continue;
                }
                if a == 255 {
                    dp.copy_from_slice(&sp[..3]);
                    continue;
                }
                let inv = 255 - a;
                for c in 0..3 {
                    dp[c] = (sp[c] as u32 + (dp[c] as u32 * inv + 127) / 255).min(255) as u8;
                }
            }
        }
    }
}

/// Coverage (0-1) of the pixel centered at (px, py) by a rounded rectangle.
#[inline]
pub(crate) fn rrect_cov(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
    let qx = (px - cx).abs() - hx;
    let qy = (py - cy).abs() - hy;
    let d = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

pub(crate) fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// Color at position `t` (0-1) along a list of color stops.
pub(crate) fn gradient_at(stops: &[Rgb], t: f32) -> Rgb {
    match stops.len() {
        0 => (255, 255, 255),
        1 => stops[0],
        n => {
            let p = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (p.floor() as usize).min(n - 2);
            lerp_rgb(stops[i], stops[i + 1], p - i as f32)
        }
    }
}

impl Layer {
    /// One solid-colored run of pixels on row `y`, from `x0` (included) to `x1` (excluded).
    pub(crate) fn span_over(&mut self, y: i32, x0: i32, x1: i32, rgb: Rgb, alpha: u8) {
        if y < 0 || y as usize >= self.h || alpha == 0 {
            return;
        }
        let (x0, x1) = (x0.max(0) as usize, (x1.max(0) as usize).min(self.w));
        if x0 >= x1 {
            return;
        }
        let a = alpha as u32;
        let inv = 255 - a;
        let src = [(rgb.0 as u32 * a + 127) / 255, (rgb.1 as u32 * a + 127) / 255, (rgb.2 as u32 * a + 127) / 255];
        let div255 = |v: u32| (v + 128 + ((v + 128) >> 8)) >> 8;
        let row = &mut self.px[(y as usize * self.w + x0) * 4..(y as usize * self.w + x1) * 4];
        for d in row.chunks_exact_mut(4) {
            d[0] = (src[0] + div255(d[0] as u32 * inv)).min(255) as u8;
            d[1] = (src[1] + div255(d[1] as u32 * inv)).min(255) as u8;
            d[2] = (src[2] + div255(d[2] as u32 * inv)).min(255) as u8;
            d[3] = (a + div255(d[3] as u32 * inv)).min(255) as u8;
        }
    }

    /// Same as `fill_rrect` for one color; whole-pixel rectangles (the tiles) take a
    /// fast path for everything but the corners.
    pub(crate) fn fill_rrect_solid(&mut self, rect: (f32, f32, f32, f32), radius: f32, rgb: Rgb, alpha: u8) {
        let (x, y, w, h) = rect;
        if x.fract() != 0.0 || y.fract() != 0.0 || w.fract() != 0.0 || h.fract() != 0.0 || w < 1.0 || h < 1.0 {
            return self.fill_rrect(rect, radius, &move |_| rgb, alpha);
        }
        let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let (xi, yi, wi, hi, ri) = (x as i32, y as i32, w as i32, h as i32, r.ceil() as i32);
        for py in yi..yi + hi {
            if py - yi < ri || yi + hi - 1 - py < ri {
                let fy = py as f32 + 0.5;
                for px in (xi..xi + ri).chain(xi + wi - ri..xi + wi) {
                    let cov = rrect_cov(px as f32 + 0.5, fy, x, y, w, h, r);
                    if cov > 0.0 {
                        self.over(px, py, rgb, (alpha as f32 * cov).round() as u8);
                    }
                }
                self.span_over(py, xi + ri, xi + wi - ri, rgb, alpha);
            } else {
                self.span_over(py, xi, xi + wi, rgb, alpha);
            }
        }
    }

    /// Anti-aliased rounded rectangle; `color(t)` gives the color at horizontal position t (0-1).
    pub(crate) fn fill_rrect(&mut self, rect: (f32, f32, f32, f32), radius: f32, color: &dyn Fn(f32) -> Rgb, alpha: u8) {
        let (x, y, w, h) = rect;
        if w <= 0.0 || h <= 0.0 || alpha == 0 {
            return;
        }
        let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        for py in (y.floor() as i32)..((y + h).ceil() as i32) {
            for px in (x.floor() as i32)..((x + w).ceil() as i32) {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let near_x = fx < x + r || fx > x + w - r;
                let near_y = fy < y + r || fy > y + h - r;
                let cov = if r > 0.0 && near_x && near_y {
                    rrect_cov(fx, fy, x, y, w, h, r)
                } else {
                    // straight edges: only a fractional position makes them soft
                    let ex = (fx - x + 0.5).min(x + w - fx + 0.5).clamp(0.0, 1.0);
                    let ey = (fy - y + 0.5).min(y + h - fy + 0.5).clamp(0.0, 1.0);
                    ex * ey
                };
                if cov > 0.0 {
                    let t = ((fx - x) / w).clamp(0.0, 1.0);
                    self.over(px, py, color(t), (alpha as f32 * cov).round() as u8);
                }
            }
        }
    }

    /// Thin outline just inside a rounded rectangle.
    pub(crate) fn stroke_rrect(&mut self, rect: (f32, f32, f32, f32), radius: f32, thick: f32, rgb: Rgb, alpha: u8) {
        let (x, y, w, h) = rect;
        let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let band = thick + r.min(thick + 2.0) + 2.0;
        let (x0, x1) = (x.floor() as i32, (x + w).ceil() as i32);
        for py in (y.floor() as i32)..((y + h).ceil() as i32) {
            let fy = py as f32 + 0.5;
            // rows well inside the tile only have pixels near the left and right edges
            let edge_row = fy - y < band + r || y + h - fy < band + r;
            let mut px = x0;
            while px < x1 {
                let fx = px as f32 + 0.5;
                if !edge_row && fx - x > band && x + w - fx > band {
                    px = ((x + w - band).floor() as i32).max(px + 1);
                    continue;
                }
                let outer = rrect_cov(fx, fy, x, y, w, h, r);
                if outer > 0.0 {
                    let inner = rrect_cov(fx, fy, x + thick, y + thick, w - 2.0 * thick, h - 2.0 * thick, (r - thick).max(0.0));
                    let cov = (outer - inner).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.over(px, py, rgb, (alpha as f32 * cov).round() as u8);
                    }
                }
                px += 1;
            }
        }
    }

    /// A ring (track) or an arc starting at the top and going clockwise by `frac` of the
    /// circle, with round ends. `color(t)`: t = position along the FULL circle (0-1).
    pub(crate) fn arc(&mut self, cx: f32, cy: f32, r_out: f32, thick: f32, frac: f32, color: &dyn Fn(f32) -> Rgb, alpha: u8) {
        let frac = frac.clamp(0.0, 1.0);
        if frac <= 0.0 || alpha == 0 {
            return;
        }
        let r_in = r_out - thick;
        let r_mid = r_out - thick / 2.0;
        let sweep = frac * std::f32::consts::TAU;
        let full = frac >= 0.9995;
        let (ex, ey) = (cx + r_mid * sweep.sin(), cy - r_mid * sweep.cos());
        let (sx, sy) = (cx, cy - r_mid);
        let cap = thick / 2.0;
        let x0 = (cx - r_out - 1.0).floor() as i32;
        let x1 = (cx + r_out + 1.0).ceil() as i32;
        let y0 = (cy - r_out - 1.0).floor() as i32;
        let y1 = (cy + r_out + 1.0).ceil() as i32;
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let (dx, dy) = (fx - cx, fy - cy);
                let d = dx.hypot(dy);
                if d > r_out + 1.0 || d < r_in - 1.0 {
                    continue;
                }
                let radial = (r_out + 0.5 - d).clamp(0.0, 1.0) * (d - r_in + 0.5).clamp(0.0, 1.0);
                let mut ang = dx.atan2(-dy); // 0 at the top, clockwise
                if ang < 0.0 {
                    ang += std::f32::consts::TAU;
                }
                // Angle measured from the start of the arc; the second half of the gap counts
                // as "before the start" so both ends get a soft edge.
                let a_s = if ang > sweep + (std::f32::consts::TAU - sweep) / 2.0 { ang - std::f32::consts::TAU } else { ang };
                let mut cov = if full {
                    radial
                } else {
                    let from_start = (a_s * d + 0.5).clamp(0.0, 1.0);
                    let to_end = ((sweep - a_s) * d + 0.5).clamp(0.0, 1.0);
                    radial * from_start * to_end
                };
                if !full {
                    for (qx, qy) in [(sx, sy), (ex, ey)] {
                        let dd = (fx - qx).hypot(fy - qy);
                        cov = cov.max((cap + 0.5 - dd).clamp(0.0, 1.0));
                    }
                }
                if cov > 0.0 {
                    let t = if full { ang / std::f32::consts::TAU } else { a_s.clamp(0.0, sweep) / std::f32::consts::TAU };
                    self.over(px, py, color(t), (alpha as f32 * cov).round() as u8);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------- text

pub(crate) struct Painter<'a> {
    pub(crate) layer: &'a mut Layer,
    pub(crate) font: FontStyle,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum W {
    Light,
    Regular,
    Medium,
}

impl Painter<'_> {
    fn pixel_scale(cap: f32) -> u32 {
        ((cap / 7.0).round() as u32).max(1)
    }

    pub(crate) fn width(&self, text: &str, cap: f32, w: W) -> f32 {
        match self.font {
            FontStyle::Smooth => smooth_font::text_width(text, smooth_font::px_for_cap(cap), weight_of(w)),
            FontStyle::Pixel => {
                let n = text.chars().count() as u32;
                let s = Self::pixel_scale(cap);
                (n * 6 * s).saturating_sub(s) as f32
            }
        }
    }

    /// Largest cap height (<= `cap`) at which `text` is at most `max_w` wide.
    pub(crate) fn fit(&self, text: &str, cap: f32, max_w: f32, w: W) -> f32 {
        let cap = cap.max(1.0);
        let natural = self.width(text, cap, w);
        if natural <= max_w || natural <= 0.0 {
            return cap;
        }
        match self.font {
            FontStyle::Smooth => (cap * max_w / natural).floor().max(5.0),
            FontStyle::Pixel => {
                let mut s = Self::pixel_scale(cap);
                while s > 1 && self.width(text, (s * 7) as f32, w) > max_w {
                    s -= 1;
                }
                (s * 7) as f32
            }
        }
    }

    /// Draws `text` with its left edge at `x` and its baseline at `base`; returns the width.
    pub(crate) fn text(&mut self, x: f32, base: f32, text: &str, cap: f32, w: W, color: Rgb, alpha: u8) -> f32 {
        match self.font {
            FontStyle::Smooth => {
                let layer = &mut *self.layer;
                smooth_font::draw_with(text, x, base, smooth_font::px_for_cap(cap), weight_of(w), |px, py, cov| {
                    layer.over(px, py, color, ((cov as u32 * alpha as u32 + 127) / 255) as u8);
                })
            }
            FontStyle::Pixel => {
                let s = Self::pixel_scale(cap) as i32;
                let top = base.round() as i32 - 7 * s;
                let mut pen = x.round() as i32;
                for c in text.chars() {
                    let g = aperture::font::glyph(c);
                    for (ry, bits) in g.iter().enumerate() {
                        for col in 0..5 {
                            if (bits >> (4 - col)) & 1 == 1 {
                                for dy in 0..s {
                                    for dx in 0..s {
                                        self.layer.over(pen + col * s + dx, top + ry as i32 * s + dy, color, alpha);
                                    }
                                }
                            }
                        }
                    }
                    pen += 6 * s;
                }
                (pen - x.round() as i32 - s).max(0) as f32
            }
        }
    }
}

fn weight_of(w: W) -> Weight {
    match w {
        W::Light => Weight::Light,
        W::Regular => Weight::Regular,
        W::Medium => Weight::Medium,
    }
}

/// "SATURDAY" -> "Saturday", "10 OCTOBER 2026" -> "10 October 2026" (smooth font only;
/// the pixel font has capitals only).
pub(crate) fn nice_case(s: &str) -> String {
    let mut out = String::new();
    let mut start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = c == ' ' || c == '-';
        }
    }
    out
}

// --------------------------------------------------------------------- views

#[derive(Clone, Debug, PartialEq)]
struct TileView {
    kind: Kind,
    look: Look,
    label: String,
    value: String,
    unit: String,
    detail: String,
    /// Fill of the bar / ring, 0-1 (None = no bar).
    frac: Option<f32>,
    /// Second line of the clock (the date) / icon of the weather tile.
    extra: String,
    icon: Option<(weather_icon::WeatherIcon, Rgb)>,
}

fn rate(kb: f64) -> (String, &'static str) {
    if kb >= 1024.0 {
        (format!("{:.1}", kb / 1024.0), "MB/s")
    } else {
        (format!("{kb:.0}"), "KB/s")
    }
}

fn view(spec: TileSpec, d: &WidgetData, s: &Settings, now: &chrono::DateTime<Local>) -> TileView {
    let tr = i18n::t();
    let smooth = s.font == FontStyle::Smooth;
    let deg = if smooth { "\u{b0}" } else { "C" };
    let case = |t: String| if smooth { nice_case(&t) } else { t };
    let o = opts();
    let pct = |v: f32| format!("{:.*}", o.percent_decimals as usize, v);
    let mut v = TileView {
        kind: spec.kind,
        look: spec.look,
        label: String::new(),
        value: String::new(),
        unit: String::new(),
        detail: String::new(),
        frac: None,
        extra: String::new(),
        icon: None,
    };
    match spec.kind {
        Kind::Clock => {
            v.label = case(i18n::weekday(now));
            v.extra = case(i18n::day_month_year(now));
            v.value = now.format(if s.clock_seconds { "%H:%M:%S" } else { "%H:%M" }).to_string();
            v.look = Look::Bar;
        }
        Kind::Cpu => {
            v.label = "CPU".into();
            v.value = pct(d.cpu_pct);
            v.unit = "%".into();
            v.frac = Some(d.cpu_pct / 100.0);
            if let Some(m) = d.cpu_mhz {
                v.detail = format_freq_mhz(m);
            }
        }
        Kind::CpuTemp => {
            v.label = "CPU TEMP".into();
            match d.cpu_temp {
                Some(t) => {
                    v.value = format!("{t:.0}{deg}");
                    v.frac = Some(t / 100.0);
                }
                None => v.value = "--".into(),
            }
            v.detail = pct(d.cpu_pct) + "%";
        }
        Kind::CpuWatt => {
            v.label = "CPU WATT".into();
            match d.cpu_power {
                Some(p) => {
                    v.value = format!("{p:.0}");
                    v.unit = "W".into();
                    v.frac = Some(p / s.cpu_watt_max as f32);
                }
                None => v.value = "--".into(),
            }
        }
        Kind::Gpu => {
            v.label = "GPU".into();
            match d.gpu_pct {
                Some(p) => {
                    v.value = pct(p);
                    v.unit = "%".into();
                    v.frac = Some(p / 100.0);
                }
                None => v.value = "--".into(),
            }
            if let Some(t) = d.gpu_temp {
                v.detail = format!("{t}{deg}");
            }
        }
        Kind::GpuTemp => {
            v.label = "GPU TEMP".into();
            match d.gpu_temp {
                Some(t) => {
                    v.value = format!("{t}{deg}");
                    v.frac = Some(t as f32 / 100.0);
                }
                None => v.value = "--".into(),
            }
            if let Some(p) = d.gpu_pct {
                v.detail = pct(p) + "%";
            }
        }
        Kind::GpuWatt => {
            v.label = "GPU WATT".into();
            match d.gpu_power {
                Some(p) => {
                    v.value = p.to_string();
                    v.unit = "W".into();
                    v.frac = Some(p as f32 / s.gpu_watt_max as f32);
                }
                None => v.value = "--".into(),
            }
        }
        Kind::Ram => {
            let p = if d.total_mb > 0 { d.used_mb as f32 * 100.0 / d.total_mb as f32 } else { 0.0 };
            v.label = tr.mem.to_string();
            v.value = pct(p);
            v.unit = "%".into();
            v.frac = Some(p / 100.0);
            v.detail = fmt_mem(d.used_mb, d.total_mb, o.mem_gb);
        }
        Kind::Net => {
            let (n, u) = rate(d.net_kb.0);
            let (nu, uu) = rate(d.net_kb.1);
            v.label = format!("{} {}", tr.net, tr.net_down);
            v.value = n;
            v.unit = u.into();
            v.detail = format!("{} {nu}{uu}", tr.net_up);
        }
        Kind::Disk => {
            v.label = format!("{} {}", tr.disk, tr.disk_read);
            v.value = format!("{:.1}", d.disk_mb.0);
            v.unit = "MB/s".into();
            v.detail = format!("{} {:.1}MB/s", tr.disk_write, d.disk_mb.1);
        }
        Kind::Fps => {
            v.label = "FPS".into();
            match d.fps {
                Some(f) => {
                    v.value = f.to_string();
                    v.frac = Some(f as f32 / s.fps_max as f32);
                    if f > 0 {
                        v.detail = format!("{:.1}ms", 1000.0 / f as f32);
                    }
                }
                None => v.value = "--".into(),
            }
        }
        Kind::Weather => {
            v.label = tr.weather.to_string();
            if let Some(w) = &d.weather {
                let f = o.weather_fahrenheit;
                v.label = w.city.clone().unwrap_or_else(|| tr.weather.to_string());
                v.value = format!("{:.0}{}", w.temp_in(f), if smooth { "\u{b0}" } else { if f { "F" } else { "C" } });
                v.detail = case(format!(
                    "{}{}",
                    weather_icon::condition_name(w.icon),
                    w.humidity.map_or_else(String::new, |h| format!(" {h}%"))
                ));
                v.icon = Some((w.icon, weather_icon::default_color(w.icon)));
            } else {
                v.value = "--".into();
            }
        }
        Kind::Music => {
            v.label = tr.now_playing.to_string();
            v.value = d.now_playing.clone().unwrap_or_else(|| "-".into());
        }
    }
    v.frac = v.frac.map(|f| f.clamp(0.0, 1.0));
    v
}

// ------------------------------------------------------------------ drawing

fn bar_rgb(s: &Settings, kind: Kind, frac: f32, color_mode: ColorMode) -> Box<dyn Fn(f32) -> Rgb> {
    match s.bar_color(kind) {
        BarColor::Auto => {
            let c = level_color(frac.max(0.15), color_mode);
            Box::new(move |_| c)
        }
        BarColor::Colors(list) => {
            let list = list.clone();
            Box::new(move |t| gradient_at(&list, t))
        }
    }
}

struct Look2 {
    text: Rgb,
    muted: Rgb,
    track_alpha: u8,
}

fn draw_tile(p: &mut Painter, pal: &Palette, s: &Settings, v: &TileView, rect: Rect, rel: (u32, u32), color_mode: ColorMode) {
    let (rx, ry, rw, rh) = rect;
    let (x, y, w, h) = ((rx - rel.0) as f32, (ry - rel.1) as f32, rw as f32, rh as f32);
    if w < 70.0 || h < 50.0 {
        return;
    }
    let radius = s.radius.map_or_else(|| (w.min(h) * 0.12).clamp(10.0, 30.0), |r| r as f32);
    let fill = pal.fill;
    p.layer.fill_rrect_solid((x, y, w, h), radius, fill, pal.alpha);
    p.layer.stroke_rrect((x, y, w, h), radius, 1.5, (255, 255, 255), if pal.light { 150 } else { 34 });

    let look = Look2 { text: pal.text, muted: lerp_rgb(pal.text, pal.fill, 0.42), track_alpha: if pal.light { 38 } else { 40 } };
    match v.kind {
        Kind::Clock => draw_clock(p, &look, v, (x, y, w, h)),
        Kind::Weather => draw_weather(p, &look, v, (x, y, w, h)),
        Kind::Music => draw_music(p, &look, v, (x, y, w, h)),
        _ => match (v.look, v.frac) {
            (Look::Ring, Some(f)) => draw_ring(p, &look, s, v, f, (x, y, w, h), color_mode),
            (_, f) => draw_value(p, &look, s, v, f, (x, y, w, h), color_mode),
        },
    }
}

fn pad_for(w: f32, h: f32) -> f32 {
    (h.min(w * 0.5) * 0.11).clamp(8.0, 24.0)
}

fn label_cap(h: f32) -> f32 {
    (h * 0.095).clamp(8.0, 22.0)
}

/// Big value + optional unit; returns the total width drawn.
fn value_with_unit(p: &mut Painter, x: f32, base: f32, v: &TileView, cap: f32, w: W, color: Rgb, unit_color: Rgb) -> f32 {
    let vw = p.text(x, base, &v.value, cap, w, color, 255);
    if v.unit.is_empty() {
        return vw;
    }
    let ucap = (cap * 0.42).max(7.0);
    let gap = (cap * 0.1).max(2.0);
    vw + gap + p.text(x + vw + gap, base, &v.unit, ucap, W::Regular, unit_color, 255)
}

fn value_width(p: &Painter, v: &TileView, cap: f32, w: W) -> f32 {
    let mut total = p.width(&v.value, cap, w);
    if !v.unit.is_empty() {
        let ucap = (cap * 0.42).max(7.0);
        total += (cap * 0.1).max(2.0) + p.width(&v.unit, ucap, W::Regular);
    }
    total
}

/// Shrinks `cap` until value + unit fit in `max_w`.
fn fit_value(p: &Painter, v: &TileView, cap: f32, max_w: f32, w: W) -> f32 {
    let mut c = cap;
    for _ in 0..40 {
        if value_width(p, v, c, w) <= max_w || c <= 6.0 {
            break;
        }
        c = if p.font == FontStyle::Pixel { (c - 7.0).max(7.0) } else { (c * 0.94).floor() };
    }
    c
}

fn draw_value(p: &mut Painter, look: &Look2, s: &Settings, v: &TileView, frac: Option<f32>, (x, y, w, h): (f32, f32, f32, f32), color_mode: ColorMode) {
    let pad = pad_for(w, h);
    let inner = w - 2.0 * pad;
    let lcap = label_cap(h);
    let lbase = y + pad + lcap;
    let lab = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    p.text(x + pad, lbase, &fit_label(p, &v.label, lcap, inner), lcap, lab, look.muted, 255);

    let bar_h = (h * 0.045).clamp(3.0, 9.0);
    let bar_y = y + h - pad - bar_h;
    let (top, bottom) = (lbase + pad * 0.5, if frac.is_some() { bar_y - pad * 0.5 } else { y + h - pad });
    let avail = (bottom - top).max(10.0);
    let detail_w = if v.detail.is_empty() { 0.0 } else { p.width(&v.detail, lcap * 1.2, W::Regular) };
    let max_w = if detail_w > 0.0 { inner - detail_w - pad } else { inner };
    let cap = fit_value(p, v, (avail * 0.66).min(h * 0.36).max(10.0), max_w.max(20.0), W::Regular);
    let base = top + (avail + cap) / 2.0;
    let used = value_with_unit(p, x + pad, base, v, cap, W::Regular, look.text, look.muted);
    if detail_w > 0.0 && used + pad + detail_w <= inner + 0.5 {
        p.text(x + w - pad - detail_w, base, &v.detail, lcap * 1.2, W::Regular, look.muted, 255);
    }
    if let Some(f) = frac {
        let track = (x + pad, bar_y, inner, bar_h);
        let tc = look.text;
        p.layer.fill_rrect_solid(track, bar_h / 2.0, tc, look.track_alpha);
        let fw = (inner * f).max(if f > 0.0 { bar_h } else { 0.0 });
        let col = bar_rgb(s, v.kind, f, color_mode);
        p.layer.fill_rrect((x + pad, bar_y, fw, bar_h), bar_h / 2.0, &move |t| col(t * fw / inner), 255);
    }
}

fn fit_label(p: &Painter, label: &str, cap: f32, max_w: f32) -> String {
    if p.width(label, cap, W::Regular) <= max_w {
        return label.to_string();
    }
    let mut s: String = label.to_string();
    while s.chars().count() > 3 && p.width(&format!("{s}.."), cap, W::Regular) > max_w {
        s.pop();
    }
    format!("{}..", s.trim_end())
}

fn draw_ring(p: &mut Painter, look: &Look2, s: &Settings, v: &TileView, frac: f32, (x, y, w, h): (f32, f32, f32, f32), color_mode: ColorMode) {
    let pad = pad_for(w, h);
    let lcap = label_cap(h);
    let lbase = y + pad + lcap;
    let lab = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    let wide = w > h * 1.3;
    let (cx, cy, d);
    if wide {
        d = (h - 2.0 * pad).min(w * 0.55).max(20.0);
        cx = x + w - pad - d / 2.0;
        cy = y + h / 2.0;
        let left_w = (cx - d / 2.0 - pad) - (x + pad);
        p.text(x + pad, lbase, &fit_label(p, &v.label, lcap, left_w.max(20.0)), lcap, lab, look.muted, 255);
        if !v.detail.is_empty() {
            // as large as fits, but never smaller than the label
            let dcap = [lcap * 1.2, lcap].into_iter().find(|c| p.width(&v.detail, *c, W::Regular) <= left_w);
            if let Some(dcap) = dcap {
                p.text(x + pad, y + h - pad, &v.detail, dcap, W::Regular, look.muted, 255);
            }
        }
    } else {
        let top = lbase + pad * 0.6;
        d = ((w - 2.0 * pad).min(y + h - pad - top)).max(20.0);
        cx = x + w / 2.0;
        cy = top + (y + h - pad - top) / 2.0;
        p.text(x + pad, lbase, &fit_label(p, &v.label, lcap, w - 2.0 * pad), lcap, lab, look.muted, 255);
    }
    let thick = (d * 0.11).clamp(4.0, 16.0);
    let r_out = d / 2.0;
    let tc = look.text;
    p.layer.arc(cx, cy, r_out, thick, 1.0, &move |_| tc, look.track_alpha);
    let col = bar_rgb(s, v.kind, frac, color_mode);
    p.layer.arc(cx, cy, r_out, thick, frac, &*col, 255);
    // The value inside the ring.
    let inner_w = (d - 2.0 * thick) * if p.font == FontStyle::Pixel { 0.74 } else { 0.86 };
    let cap = fit_value(p, v, d * 0.27, inner_w, W::Medium);
    let tw = value_width(p, v, cap, W::Medium);
    value_with_unit(p, cx - tw / 2.0, cy + cap / 2.0, v, cap, W::Medium, look.text, look.muted);
}

fn draw_clock(p: &mut Painter, look: &Look2, v: &TileView, (x, y, w, h): (f32, f32, f32, f32)) {
    let pad = pad_for(w, h) * 1.2;
    let inner = w - 2.0 * pad;
    let lcap = (h * 0.075).clamp(9.0, 28.0);
    let wd = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    p.text(x + pad, y + pad + lcap, &v.label, p.fit(&v.label, lcap, inner, wd), wd, look.text, 255);
    let dcap = lcap * 0.85;
    p.text(x + pad, y + pad + lcap + dcap + pad * 0.5, &v.extra, p.fit(&v.extra, dcap, inner, W::Regular), W::Regular, look.muted, 255);
    let top = y + pad + lcap + dcap + pad * 1.2;
    let avail = (y + h - pad - top).max(10.0);
    let tw = if p.font == FontStyle::Smooth && avail > 60.0 { W::Light } else { W::Regular };
    let cap = p.fit(&v.value, avail.min(h * 0.55), inner, tw);
    let width = p.width(&v.value, cap, tw);
    p.text(x + (w - width) / 2.0, top + (avail + cap) / 2.0, &v.value, cap, tw, look.text, 255);
}

fn draw_weather(p: &mut Painter, look: &Look2, v: &TileView, (x, y, w, h): (f32, f32, f32, f32)) {
    let pad = pad_for(w, h);
    let inner = w - 2.0 * pad;
    let lcap = label_cap(h);
    let lbase = y + pad + lcap;
    let lab = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    p.text(x + pad, lbase, &fit_label(p, &v.label, lcap, inner), lcap, lab, look.muted, 255);
    let icon_scale = (((h - 2.0 * pad) * 0.5) / weather_icon::ICON_SIZE as f32).floor().max(1.0) as u32;
    let icon_px = weather_icon::size(icon_scale) as f32;
    let show_icon = v.icon.is_some() && icon_px + pad < inner * 0.45;
    let (top, bottom) = (lbase + pad * 0.5, y + h - pad - if v.detail.is_empty() { 0.0 } else { lcap * 1.4 });
    let avail = (bottom - top).max(10.0);
    let max_w = inner - if show_icon { icon_px + pad } else { 0.0 };
    let cap = p.fit(&v.value, (avail * 0.7).min(h * 0.4), max_w.max(20.0), W::Regular);
    let base = top + (avail + cap) / 2.0;
    p.text(x + pad, base, &v.value, cap, W::Regular, look.text, 255);
    if let (true, Some((icon, (r, g, b)))) = (show_icon, v.icon) {
        let n = weather_icon::size(icon_scale);
        let mut tmp = Framebuffer::new(aperture::Resolution::new(n, n));
        tmp.clear(1, 0, 2);
        weather_icon::draw(&mut tmp, 0, 0, icon_scale, icon, r, g, b);
        let (ix, iy) = ((x + w - pad - icon_px) as i32, (y + (h - icon_px) / 2.0) as i32);
        for py in 0..n {
            for px in 0..n {
                let i = ((py * n + px) * 3) as usize;
                let c = &tmp.as_bytes()[i..i + 3];
                if (c[0], c[1], c[2]) != (1, 0, 2) {
                    p.layer.over(ix + px as i32, iy + py as i32, (c[0], c[1], c[2]), 255);
                }
            }
        }
    }
    if !v.detail.is_empty() {
        p.text(x + pad, y + h - pad, &fit_label(p, &v.detail, lcap * 1.1, inner), lcap * 1.1, W::Regular, look.muted, 255);
    }
}

fn draw_music(p: &mut Painter, look: &Look2, v: &TileView, (x, y, w, h): (f32, f32, f32, f32)) {
    let pad = pad_for(w, h);
    let inner = w - 2.0 * pad;
    let lcap = label_cap(h);
    let lbase = y + pad + lcap;
    let lab = if p.font == FontStyle::Smooth { W::Medium } else { W::Regular };
    p.text(x + pad, lbase, &fit_label(p, &v.label, lcap, inner), lcap, lab, look.muted, 255);
    let cap = (h * 0.2).clamp(10.0, 44.0);
    let mut title = v.value.clone();
    if p.width(&title, cap, W::Regular) > inner {
        while title.chars().count() > 3 && p.width(&format!("{title}..."), cap, W::Regular) > inner {
            title.pop();
        }
        title = format!("{}...", title.trim_end());
    }
    let (top, bottom) = (lbase + pad * 0.5, y + h - pad);
    p.text(x + pad, top + ((bottom - top) + cap) / 2.0, &title, cap, W::Regular, look.text, 255);
}

// -------------------------------------------------------------------- screen

/// What the screen needs from the main loop besides the data.
pub struct Env<'a> {
    pub fps: f32,
    pub ffmpeg: Option<&'a str>,
    pub effect: &'a effect::Settings,
}

pub struct TilesScreen {
    layer: Layer,
    sig: String,
    effect: effect::Effect,
    image: Option<(String, Option<Background>)>,
    gradient: Option<(String, Framebuffer)>,
    /// Number of times the layer was re-drawn (tests, tuning).
    pub redraws: u32,
}

impl TilesScreen {
    pub fn new() -> Self {
        TilesScreen {
            layer: Layer::new(1, 1),
            sig: String::new(),
            effect: effect::Effect::new(),
            image: None,
            gradient: None,
            redraws: 0,
        }
    }

    fn paint_background(&mut self, fb: &mut Framebuffer, s: &Settings, env: &Env) {
        match s.bg {
            BgMode::Global => {}
            BgMode::Color => {
                let c = s.bg_colors.first().copied().unwrap_or((8, 8, 16));
                fb.clear(c.0, c.1, c.2);
            }
            BgMode::Gradient => {
                let key = format!("{:?}|{}x{}", s.bg_colors, fb.width(), fb.height());
                if self.gradient.as_ref().map(|g| g.0 != key).unwrap_or(true) {
                    let mut g = Framebuffer::new(aperture::Resolution::new(fb.width(), fb.height()));
                    let (w, h) = (fb.width() as usize, fb.height() as usize);
                    let stops = if s.bg_colors.len() == 1 { vec![s.bg_colors[0], dim_color(s.bg_colors[0], 35)] } else { s.bg_colors.clone() };
                    let bytes = g.as_bytes_mut();
                    for y in 0..h {
                        for x in 0..w {
                            // diagonal: top-left -> bottom-right
                            let t = (x as f32 / w as f32 * 0.65 + y as f32 / h as f32 * 0.35).clamp(0.0, 1.0);
                            let c = gradient_at(&stops, t);
                            let i = (y * w + x) * 3;
                            bytes[i] = c.0;
                            bytes[i + 1] = c.1;
                            bytes[i + 2] = c.2;
                        }
                    }
                    self.gradient = Some((key, g));
                }
                if let Some((_, g)) = &self.gradient {
                    fb.copy_from(g);
                }
            }
            BgMode::Effect => {
                let style = env.effect.style_or_default();
                let brightness = env.effect.brightness * (100 - s.bg_dim.min(100)) / 100;
                self.effect.render(fb, env.effect, style, brightness);
            }
            BgMode::Image => {
                let path = s.bg_image.clone().unwrap_or_default();
                let key = format!("{}|{}x{}|{}", path.display(), fb.width(), fb.height(), s.bg_dim);
                if self.image.as_ref().map(|i| i.0 != key).unwrap_or(true) {
                    let res = aperture::Resolution::new(fb.width(), fb.height());
                    let loaded = match Background::load(&path, res, s.bg_dim as u8, env.fps, env.ffmpeg, &BgLayout::default()) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            eprintln!("WARNING: tiles_bg_image not loaded: {e}");
                            None
                        }
                    };
                    self.image = Some((key, loaded));
                }
                match self.image.as_mut().and_then(|i| i.1.as_mut()) {
                    Some(b) => b.render_into(fb),
                    None => fb.clear(8, 8, 16),
                }
            }
        }
    }

    /// Paints the background chosen by `tiles_bg` and the tiles over `fb`; `area` is the
    /// part of `fb` the interface may use (the screen minus the margins).
    pub fn draw(&mut self, fb: &mut Framebuffer, area: Rect, data: &WidgetData, color_mode: ColorMode, s: &Settings, env: &Env) {
        self.paint_background(fb, s, env);
        let cells = cells(s, area);
        if cells.is_empty() {
            return;
        }
        let now = Local::now();
        let views: Vec<TileView> = cells.iter().map(|(t, _)| view(*t, data, s, &now)).collect();
        let cm = format!("{color_mode:?}");
        let sig = format!("{s:?}|{area:?}|{views:?}|{cm}");
        if sig != self.sig {
            if self.layer.w != area.2 as usize || self.layer.h != area.3 as usize {
                self.layer = Layer::new(area.2, area.3);
            } else {
                self.layer.clear();
            }
            let pal = s.palette();
            let mut painter = Painter { layer: &mut self.layer, font: s.font };
            for ((_, rect), v) in cells.iter().zip(views.iter()) {
                draw_tile(&mut painter, &pal, s, v, *rect, (area.0, area.1), color_mode);
            }
            self.sig = sig;
            self.redraws += 1;
        }
        self.layer.composite_onto(fb, area.0, area.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> WidgetData {
        WidgetData {
            cpu_pct: 37.0, cpu_temp: Some(64.0), cpu_power: Some(88.0), cpu_mhz: Some(4900),
            gpu_pct: Some(83.0), gpu_temp: Some(71), gpu_power: Some(245), fps: Some(144),
            used_mb: 18200, total_mb: 32768, net_kb: (2300.0, 120.0), disk_mb: (35.2, 1.4),
            now_playing: Some("Artist - A very long song title that has to be cut with dots".into()),
            weather: Some(crate::weather::WeatherSnapshot {
                temp_c: 21.0, humidity: Some(55), code: 61,
                icon: weather_icon::WeatherIcon::Rain, city: Some("Milano".into()), forecast: Vec::new(),
            }),
            ticker: Vec::new(),
        }
    }

    fn env(e: &effect::Settings) -> Env<'_> {
        Env { fps: 30.0, ffmpeg: None, effect: e }
    }

    #[test]
    fn tile_lists_parse() {
        let r = parse_tiles("clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, GPU-Watt:ring").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 3);
        assert!(r[0][0].tall && r[0][0].kind == Kind::Clock);
        assert_eq!(r[1][2].kind, Kind::GpuWatt);
        assert_eq!(r[1][0].look, Look::Ring);
        assert!(parse_tiles("cpu, nope").is_err());
        assert!(parse_tiles("cpu:round").is_err());
        assert!(parse_tiles("cpu, clock:tall").is_err());
        assert!(parse_tiles("  |  ").is_err());
        let many = vec!["cpu"; 17].join(",");
        assert!(parse_tiles(&many).is_err());
    }

    #[test]
    fn settings_parse_colors_background_and_errors() {
        let f = |t: &str| parse(&ConfigFile::parse(t).unwrap());
        let s = f("tiles = cpu, gpu\ntiles_font = pixel\ntiles_theme = light\ntiles_bar_color = #FF0080, #00FFFF\ntile_cpu_color = #112233\ntile_gpu_temp_color = auto\ntiles_bg = gradient\ntiles_bg_color = #101820, #402010\ntiles_opacity = 40\ntiles_radius = 0\ntiles_gap = 8\n").unwrap();
        assert_eq!(s.font, FontStyle::Pixel);
        assert_eq!(s.theme, Theme::Light);
        assert_eq!(s.bar, BarColor::Colors(vec![(255, 0, 128), (0, 255, 255)]));
        assert_eq!(s.bar_color(Kind::Cpu), &BarColor::Colors(vec![(0x11, 0x22, 0x33)]));
        assert_eq!(s.bar_color(Kind::Gpu), &s.bar); // no override: the general colors
        assert_eq!(s.bar_color(Kind::GpuTemp), &BarColor::Auto);
        assert_eq!((s.bg, s.opacity, s.radius, s.gap), (BgMode::Gradient, Some(40), Some(0), 8));
        assert_eq!(f("").unwrap(), Settings::default());
        for bad in ["tiles_font = comic", "tiles_bg = video", "tiles_opacity = 120", "tile_cpu_color = #12", "tiles_bg = image\n", "tiles = cpu, gpu:tall"] {
            assert!(f(bad).is_err(), "{bad}");
        }
        assert_eq!(BgMode::parse("none"), Some(BgMode::Global));
    }

    #[test]
    fn cells_fit_inside_the_area_and_never_overlap() {
        let overlap = |a: Rect, b: Rect| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
        let lists = [
            "clock:tall, cpu_temp, gpu_temp | ram:ring, cpu:ring, gpu:ring",
            "clock, cpu, gpu, ram",
            "cpu | gpu | ram",
            "clock:tall, cpu | gpu | ram",
            "cpu, gpu, ram, net, disk, fps, weather, music | clock",
        ];
        for list in lists {
            let mut s = Settings::default();
            s.rows = parse_tiles(list).unwrap();
            for (w, h) in [(1920u32, 462u32), (1920, 400), (462, 1920), (400, 1920), (800, 480)] {
                let area = (0, 0, w, h);
                let cs = cells(&s, area);
                let n: usize = s.rows.iter().map(Vec::len).sum();
                assert_eq!(cs.len(), n, "{list} @ {w}x{h}");
                for (i, (_, r)) in cs.iter().enumerate() {
                    assert!(r.0 + r.2 <= w && r.1 + r.3 <= h, "{list} @ {w}x{h}: {r:?} outside");
                    assert!(r.2 > 0 && r.3 > 0, "{list} @ {w}x{h}: empty cell {r:?}");
                    for (_, q) in cs.iter().skip(i + 1) {
                        assert!(!overlap(*r, *q), "{list} @ {w}x{h}: {r:?} overlaps {q:?}");
                    }
                }
            }
        }
        // the tall tile spans every row
        let s = Settings::default();
        let cs = cells(&s, (0, 0, 1920, 400));
        let clock = cs.iter().find(|(t, _)| t.kind == Kind::Clock).unwrap().1;
        let ram = cs.iter().find(|(t, _)| t.kind == Kind::Ram).unwrap().1;
        assert!(clock.3 > ram.3 * 2 - 1 && ram.0 > clock.0 + clock.2, "{clock:?} {ram:?}");
    }

    #[test]
    fn the_layer_blends_like_source_over() {
        let mut l = Layer::new(2, 1);
        l.over(0, 0, (200, 100, 0), 255);
        l.over(1, 0, (255, 255, 255), 128);
        let mut fb = Framebuffer::new(aperture::Resolution::new(2, 1));
        fb.clear(0, 0, 100);
        l.composite_onto(&mut fb, 0, 0);
        assert_eq!(&fb.as_bytes()[0..3], &[200, 100, 0]);
        let p = &fb.as_bytes()[3..6];
        assert!((p[0] as i32 - 128).abs() <= 1 && (p[2] as i32 - 177).abs() <= 2, "{p:?}");
        // outside the layer / outside the framebuffer: no panic
        l.over(-3, 5, (1, 2, 3), 255);
        l.composite_onto(&mut fb, 5, 5);
    }

    #[test]
    fn rounded_corners_are_transparent_and_the_inside_is_filled() {
        let mut l = Layer::new(100, 60);
        l.fill_rrect((0.0, 0.0, 100.0, 60.0), 20.0, &|_| (255, 0, 0), 255);
        let a = |x: usize, y: usize| l.px[(y * 100 + x) * 4 + 3];
        assert!(a(0, 0) < 20, "corner pixel should be (almost) clear: {}", a(0, 0));
        assert_eq!(a(50, 30), 255);
        assert_eq!(a(50, 0), 255); // straight edge
        assert!(a(6, 5) > 0 && a(6, 5) < 255, "anti-aliased corner: {}", a(6, 5));
    }

    #[test]
    fn the_fast_fill_matches_the_general_one() {
        for (w, h, r) in [(100.0f32, 60.0f32, 20.0f32), (50.0, 50.0, 25.0), (80.0, 30.0, 0.0), (10.0, 10.0, 30.0)] {
            let (mut a, mut b) = (Layer::new(120, 80), Layer::new(120, 80));
            a.fill_rrect_solid((5.0, 7.0, w, h), r, (10, 200, 90), 170);
            b.fill_rrect((5.0, 7.0, w, h), r, &|_| (10, 200, 90), 170);
            let worst = a.px.iter().zip(b.px.iter()).map(|(p, q)| (*p as i32 - *q as i32).abs()).max().unwrap();
            assert!(worst <= 1, "{w}x{h} r={r}: differs by {worst}");
        }
    }

    #[test]
    fn rings_fill_the_requested_part_of_the_circle() {
        let mut l = Layer::new(100, 100);
        l.arc(50.0, 50.0, 40.0, 10.0, 0.25, &|_| (0, 255, 0), 255);
        let a = |x: usize, y: usize| l.px[(y * 100 + x) * 4 + 3];
        assert_eq!(a(50, 14), 255); // top (start)
        assert_eq!(a(86, 50), 255); // right (a quarter turn clockwise)
        assert_eq!(a(50, 86), 0); // bottom: not reached
        assert_eq!(a(14, 50), 0); // left: not reached
        assert_eq!(a(50, 50), 0); // hole
        let mut full = Layer::new(100, 100);
        full.arc(50.0, 50.0, 40.0, 10.0, 1.0, &|_| (0, 255, 0), 255);
        assert_eq!(full.px[(50 * 100 + 14) * 4 + 3], 255);
        assert_eq!(full.px[(86 * 100 + 50) * 4 + 3], 255);
    }

    #[test]
    fn gradients_interpolate_between_stops() {
        let g = [(0u8, 0u8, 0u8), (200, 100, 50)];
        assert_eq!(gradient_at(&g, 0.0), (0, 0, 0));
        assert_eq!(gradient_at(&g, 1.0), (200, 100, 50));
        assert_eq!(gradient_at(&g, 0.5), (100, 50, 25));
        assert_eq!(gradient_at(&[(9, 9, 9)], 0.7), (9, 9, 9));
    }

    #[test]
    fn nice_case_lowers_all_but_the_first_letter_of_each_word() {
        assert_eq!(nice_case("SATURDAY"), "Saturday");
        assert_eq!(nice_case("10 OCTOBER 2026"), "10 October 2026");
        assert_eq!(nice_case("PARTLY CLOUDY 55%"), "Partly Cloudy 55%");
    }

    /// Both fonts, both themes, every background, every tile, on both panels and in both
    /// orientations: nothing panics, the tiles show up, the layer is cached between frames.
    #[test]
    fn every_look_renders_on_every_panel() {
        let eff = effect::Settings { style: Some(effect::Style::Plasma), ..effect::Settings::default() };
        let all = "clock:tall, cpu_temp, gpu_temp, cpu_watt | ram:ring, cpu:ring, gpu:ring, gpu_watt:ring | net, disk, fps, weather, music";
        for (w, h) in [(1920u32, 462u32), (1920, 400), (462, 1920), (400, 1920)] {
            for font in [FontStyle::Smooth, FontStyle::Pixel] {
                for theme in [Theme::Dark, Theme::Light] {
                    for bg in [BgMode::Global, BgMode::Color, BgMode::Gradient, BgMode::Effect] {
                        let mut s = Settings { font, theme, bg, ..Settings::default() };
                        s.rows = parse_tiles(all).unwrap();
                        let mut screen = TilesScreen::new();
                        let mut fb = Framebuffer::new(aperture::Resolution::new(w, h));
                        fb.clear(10, 10, 30);
                        let before = fb.as_bytes().to_vec();
                        screen.draw(&mut fb, (0, 0, w, h), &data(), ColorMode::Default, &s, &env(&eff));
                        assert!(fb.as_bytes() != &before[..], "{w}x{h} {font:?} {theme:?} {bg:?}: nothing drawn");
                        let first = screen.redraws;
                        screen.draw(&mut fb, (0, 0, w, h), &data(), ColorMode::Default, &s, &env(&eff));
                        assert_eq!(screen.redraws, first, "the layer must be reused when nothing changed");
                    }
                }
            }
        }
    }

    #[test]
    fn a_changed_value_or_setting_redraws_the_layer() {
        let eff = effect::Settings::default();
        let s = Settings::default();
        let mut screen = TilesScreen::new();
        let mut fb = Framebuffer::new(aperture::Resolution::new(1920, 400));
        let mut d = data();
        screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s, &env(&eff));
        d.cpu_pct = 60.0;
        screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s, &env(&eff));
        assert_eq!(screen.redraws, 2);
        let s2 = Settings { font: FontStyle::Pixel, ..s.clone() };
        screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s2, &env(&eff));
        assert_eq!(screen.redraws, 3);
    }

    #[test]
    fn bar_colors_follow_the_general_and_the_per_tile_settings() {
        let at = |s: &Settings, k: Kind| bar_rgb(s, k, 0.5, ColorMode::Default)(0.5);
        let mut s = Settings::default();
        s.bar = BarColor::Colors(vec![(255, 0, 0)]);
        s.bar_by_kind[Kind::Gpu.index()] = Some(BarColor::Colors(vec![(0, 0, 255)]));
        assert_eq!(at(&s, Kind::Cpu), (255, 0, 0));
        assert_eq!(at(&s, Kind::Gpu), (0, 0, 255));
        s.bar = BarColor::Auto;
        assert_eq!(at(&s, Kind::Cpu), level_color(0.5, ColorMode::Default));
    }

    /// Rough cost of a frame (run with --nocapture): drawing the layer vs. re-using it.
    #[test]
    fn frame_cost_is_small() {
        let eff = effect::Settings::default();
        for bg in [BgMode::Global, BgMode::Effect] {
            let s = Settings { bg, ..Settings::default() };
            let mut screen = TilesScreen::new();
            let mut fb = Framebuffer::new(aperture::Resolution::new(1920, 400));
            let mut d = data();
            screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s, &env(&eff)); // fills the glyph cache
            let t = std::time::Instant::now();
            for i in 0..20 {
                d.cpu_pct = 1.0 + i as f32;
                screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s, &env(&eff));
            }
            let redraw = t.elapsed() / 20;
            let t = std::time::Instant::now();
            for _ in 0..50 {
                screen.draw(&mut fb, (0, 0, 1920, 400), &d, ColorMode::Default, &s, &env(&eff));
            }
            println!("{bg:?}: changed frame {redraw:?}, unchanged frame {:?}", t.elapsed() / 50);
        }
    }

    /// With TROFEO_DUMP_DIR set, saves previews of the main looks.
    #[test]
    fn dump_previews() {
        let Ok(dir) = std::env::var("TROFEO_DUMP_DIR") else { return };
        let eff = effect::Settings { style: Some(effect::Style::Aurora), ..effect::Settings::default() };
        let save = |name: &str, fb: &Framebuffer| {
            std::fs::write(
                format!("{dir}/{name}.ppm"),
                [format!("P6\n{} {}\n255\n", fb.width(), fb.height()).into_bytes(), fb.as_bytes().to_vec()].concat(),
            ).unwrap();
        };
        let cases: Vec<(&str, Settings, (u32, u32))> = vec![
            ("tiles_default_1920x400", Settings::default(), (1920, 400)),
            ("tiles_default_1920x462", Settings::default(), (1920, 462)),
            ("tiles_pixel", Settings { font: FontStyle::Pixel, ..Settings::default() }, (1920, 462)),
            ("tiles_light_gradient", Settings { theme: Theme::Light, bg: BgMode::Gradient, bg_colors: vec![(0x9f, 0xc5, 0xe8), (0xd9, 0xb8, 0xf0)], ..Settings::default() }, (1920, 400)),
            ("tiles_effect_colors", Settings { bg: BgMode::Effect, bar: BarColor::Colors(vec![(0x00, 0xC8, 0xFF), (0xFF, 0x00, 0x80)]), ..Settings::default() }, (1920, 400)),
            ("tiles_many", Settings { rows: parse_tiles("cpu, gpu, ram, net | cpu_temp, gpu_temp, cpu_watt, gpu_watt:ring | weather, music, fps, disk").unwrap(), ..Settings::default() }, (1920, 462)),
            ("tiles_portrait", Settings::default(), (400, 1920)),
        ];
        for (name, s, (w, h)) in cases {
            let mut screen = TilesScreen::new();
            let mut fb = Framebuffer::new(aperture::Resolution::new(w, h));
            fb.clear(0x08, 0x08, 0x10);
            screen.draw(&mut fb, (0, 0, w, h), &data(), ColorMode::Default, &s, &env(&eff));
            save(name, &fb);
        }
    }
}
