//! The `pro` layout: a dark "control center" dashboard in the style of premium
//! hardware monitors — one strip of panels, each with a header, a ring, the
//! detailed values and a short history graph:
//!
//! `TIME + WEATHER | PROCESSOR | GRAPHICS | MEMORY / STORAGE | NOW PLAYING / NETWORK`
//!
//! * Processor: load ring, temperature, power, clock, cores, model name, load history.
//! * Graphics: load ring, temperature, hotspot, power, fan RPM, clock, VRAM used/total,
//!   adapter name, FPS badge while a game reports it, load history.
//! * Memory: used / total GB + ring. Storage: system drive usage ring, drive temperature,
//!   read / write speed.
//! * Now playing: album cover, title, artist, progress (accent color from the cover).
//! * Network: download / upload speed with a small history.
//!
//! Everything is painted on a transparent [`Layer`] (shared with `tiles.rs`) that is
//! re-drawn only when something visible changes.

use super::*;
use crate::gpu_amd::GpuAmdData;
use crate::media::TrackInfo;
use crate::sysextra::DiskInfo;
use crate::tiles::{gradient_at, lerp_rgb, rrect_cov, FontStyle, Layer, Painter, W};
use crate::weather::WeatherSnapshot;
use crate::weather_icon::{self, WeatherIcon};
use chrono::Timelike;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

#[path = "pro_designs.rs"]
mod designs;
#[path = "pro_showcase.rs"]
mod showcase;

type Rgb = (u8, u8, u8);
type R = (f32, f32, f32, f32);

const WHITE: Rgb = (255, 255, 255);

// ------------------------------------------------------------------- themes

/// Colors of one theme. Every theme is dark; they differ in tint and accents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    bg: Rgb,
    panel: Rgb,
    text: Rgb,
    muted: Rgb,
    dim: Rgb,
    cpu: [Rgb; 2],
    gpu: [Rgb; 2],
    ram: [Rgb; 2],
    ssd: [Rgb; 2],
    net: [Rgb; 2],
    up: Rgb,
    fps: Rgb,
    /// Seconds line under the clock and the clock panel's accent.
    time: Rgb,
}

include!("dashboard/palettes.rs");

pub const THEMES: &[Theme] = &[
    APERTURE_THEMES[0],
    APERTURE_THEMES[1],
    APERTURE_THEMES[2],
    APERTURE_THEMES[3],
    APERTURE_THEMES[4],
    APERTURE_THEMES[5],
    // violet / cyan / pink on near-black (NZXT CAM-like)
    Theme {
        name: "nzxt", bg: (0, 0, 0), panel: (12, 13, 17), text: (242, 243, 246), muted: (168, 172, 184), dim: (118, 122, 136),
        cpu: [(124, 92, 255), (196, 140, 255)], gpu: [(14, 165, 233), (103, 232, 249)], ram: [(236, 72, 153), (251, 164, 206)],
        ssd: [(245, 158, 11), (253, 214, 99)], net: [(16, 185, 129), (110, 231, 183)], up: (96, 165, 250), fps: (163, 230, 53), time: (255, 255, 255),
    },
    // monochrome silver: no colors except warnings (Apple / studio gear look)
    Theme {
        name: "graphite", bg: (0, 0, 0), panel: (15, 15, 16), text: (246, 246, 247), muted: (165, 165, 170), dim: (112, 112, 118),
        cpu: [(150, 150, 158), (255, 255, 255)], gpu: [(150, 150, 158), (255, 255, 255)], ram: [(150, 150, 158), (255, 255, 255)],
        ssd: [(150, 150, 158), (255, 255, 255)], net: [(150, 150, 158), (255, 255, 255)], up: (200, 200, 206), fps: (255, 255, 255), time: (255, 255, 255),
    },
    // red / orange on warm black (gaming brands)
    Theme {
        name: "crimson", bg: (0, 0, 0), panel: (17, 10, 11), text: (247, 240, 240), muted: (188, 165, 167), dim: (138, 114, 116),
        cpu: [(220, 20, 50), (255, 99, 99)], gpu: [(255, 78, 42), (255, 160, 80)], ram: [(190, 24, 93), (251, 113, 133)],
        ssd: [(249, 115, 22), (253, 186, 116)], net: [(239, 68, 68), (252, 165, 165)], up: (253, 186, 116), fps: (250, 204, 21), time: (255, 120, 120),
    },
    // ice blue / teal on deep navy
    Theme {
        name: "arctic", bg: (2, 5, 10), panel: (10, 17, 27), text: (236, 244, 252), muted: (152, 172, 194), dim: (104, 124, 146),
        cpu: [(56, 189, 248), (186, 230, 253)], gpu: [(45, 212, 191), (153, 246, 228)], ram: [(129, 140, 248), (199, 210, 254)],
        ssd: [(96, 165, 250), (191, 219, 254)], net: [(34, 211, 238), (165, 243, 252)], up: (199, 210, 254), fps: (110, 231, 183), time: (186, 230, 253),
    },
    // amber / gold on warm black (instrument cluster)
    Theme {
        name: "ember", bg: (0, 0, 0), panel: (16, 13, 9), text: (250, 245, 236), muted: (190, 172, 150), dim: (136, 120, 100),
        cpu: [(251, 146, 60), (254, 205, 140)], gpu: [(234, 179, 8), (254, 240, 138)], ram: [(248, 113, 113), (254, 178, 160)],
        ssd: [(217, 119, 6), (252, 211, 77)], net: [(163, 230, 53), (217, 249, 157)], up: (254, 205, 140), fps: (250, 204, 21), time: (254, 205, 140),
    },
    // magenta / cyan / yellow on black (cyberpunk)
    Theme {
        name: "neon", bg: (0, 0, 0), panel: (11, 8, 19), text: (242, 240, 255), muted: (176, 164, 206), dim: (122, 110, 156),
        cpu: [(255, 0, 153), (255, 110, 199)], gpu: [(0, 229, 255), (140, 250, 255)], ram: [(190, 60, 255), (226, 150, 255)],
        ssd: [(255, 214, 0), (255, 240, 140)], net: [(0, 255, 140), (140, 255, 200)], up: (0, 229, 255), fps: (255, 230, 0), time: (255, 110, 199),
    },
];

/// How the panels are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Solid rounded panels with a thin border.
    Cards,
    /// Panels with a soft glow of their accent color at the top.
    Glass,
    /// No panels: hairline dividers between the sections.
    Flat,
}

/// Arrangement of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Airport departure board: every number on split-flap tiles.
    Board,
    /// Paper and ink: a light spec-sheet layout (Braun / Teenage Engineering).
    Studio,
    /// Live charts of the last two minutes per section.
    Pulse,
    /// Car instrument cluster: three dials with a real scale.
    Cluster,
    Axis,
    Panorama,
    Cinema,
    Ledger,
    Duet,
    Orbit,
    Atelier,

    /// Panels in a row (the first design).
    Classic,
    /// Car instrument cluster: two gauges and the clock in the middle.
    Cockpit,
    /// A few huge numbers.
    Focus,
    /// Solid colored rounded tiles (bento box).
    Bento,
    /// Green-phosphor terminal.
    Terminal,
    /// Angular, slanted HUD (racing / cyberpunk).
    Chamfer,
    /// Concentric activity rings.
    Rings,
    /// Dot-matrix digits.
    Dots,
    /// Portrait: lock screen over an aurora.
    Lock,
    /// Portrait: LED level meters.
    Equalizer,
    /// Portrait: a paper receipt.
    Receipt,
    /// Portrait: retro sunset.
    Synthwave,
    /// Portrait: technical drawing.
    Blueprint,
    /// Swiss typographic grid.
    Swiss,
    /// Clean: rounded tiles like phone widgets.
    Grid,
    /// Tiles tinted with each section's color (Samsung One UI widgets).
    Tint,
    /// Translucent tiles over a soft color wallpaper (macOS / Windows 11 widgets).
    Glass,
    /// Tiles with a big arc gauge per section (NZXT CAM / Kraken).
    Gauge,
    /// Three full-width rows, six readings in a line each.
    Stack,
    /// Eight equal tiles, one big number each.
    Mosaic,
    /// A mixing console of level faders.
    Mixer,
    /// A temperature chart with a scale next to a table.
    Chart,
    /// Music first: a large album cover tile.
    Player,
    /// Full-bleed game poster / hero background with logo and telemetry cards.
    Showcase,
}

impl Layout {
    pub const CURATED: [Layout; 13] = [Layout::Board, Layout::Studio, Layout::Pulse, Layout::Cluster, Layout::Panorama, Layout::Cinema, Layout::Axis, Layout::Ledger, Layout::Duet, Layout::Orbit, Layout::Atelier, Layout::Focus, Layout::Grid];
    /// The landscape designs (what `pro_layout = all` rotates through).
    pub const ALL: [Layout; 30] = [Layout::Board, Layout::Studio, Layout::Pulse, Layout::Cluster, Layout::Panorama, Layout::Cinema, Layout::Axis, Layout::Ledger, Layout::Duet, Layout::Orbit, Layout::Atelier, Layout::Focus, Layout::Grid, Layout::Stack, Layout::Mosaic, Layout::Mixer, Layout::Chart, Layout::Player, Layout::Showcase, Layout::Tint, Layout::Glass, Layout::Gauge, Layout::Swiss, Layout::Classic, Layout::Cockpit, Layout::Bento, Layout::Terminal, Layout::Chamfer, Layout::Rings, Layout::Dots];
    /// The portrait designs (screen mounted vertically; `pro_layout = portrait`).
    pub const PORTRAIT: [Layout; 5] = [Layout::Lock, Layout::Equalizer, Layout::Receipt, Layout::Synthwave, Layout::Blueprint];
    pub fn name(self) -> &'static str {
        match self {
            Layout::Board => "board",
            Layout::Studio => "studio",
            Layout::Pulse => "pulse",
            Layout::Cluster => "cluster",
            Layout::Axis => "axis",
            Layout::Panorama => "panorama",
            Layout::Cinema => "cinema",
            Layout::Ledger => "ledger",
            Layout::Duet => "duet",
            Layout::Orbit => "orbit",
            Layout::Atelier => "atelier",
            Layout::Classic => "classic",
            Layout::Cockpit => "cockpit",
            Layout::Focus => "focus",
            Layout::Bento => "bento",
            Layout::Terminal => "terminal",
            Layout::Chamfer => "chamfer",
            Layout::Rings => "rings",
            Layout::Dots => "dots",
            Layout::Lock => "lock",
            Layout::Equalizer => "equalizer",
            Layout::Receipt => "receipt",
            Layout::Synthwave => "synthwave",
            Layout::Blueprint => "blueprint",
            Layout::Swiss => "swiss",
            Layout::Grid => "grid",
            Layout::Tint => "tint",
            Layout::Glass => "glass",
            Layout::Gauge => "gauge",
            Layout::Stack => "stack",
            Layout::Mosaic => "mosaic",
            Layout::Mixer => "mixer",
            Layout::Chart => "chart",
            Layout::Player => "player",
            Layout::Showcase => "showcase",
        }
    }
    /// The clean collection, drawn with Inter.
    pub fn clean(self) -> bool {
        matches!(self, Layout::Board | Layout::Studio | Layout::Pulse | Layout::Cluster | Layout::Panorama | Layout::Cinema | Layout::Axis | Layout::Ledger | Layout::Duet | Layout::Orbit | Layout::Atelier | Layout::Focus | Layout::Grid | Layout::Tint | Layout::Glass | Layout::Gauge | Layout::Stack | Layout::Mosaic | Layout::Mixer | Layout::Chart | Layout::Player | Layout::Showcase)
    }
    /// Drawn on a 462 x 1920 canvas and turned 90 degrees onto the panel.
    pub fn portrait(self) -> bool {
        Layout::PORTRAIT.contains(&self)
    }
    fn find(name: &str) -> Option<Layout> {
        Layout::ALL.iter().chain(Layout::PORTRAIT.iter()).copied().find(|l| l.name() == name)
    }
}

/// Artwork mode when a running game occupies the music/media slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameArtMode {
    #[default]
    Logo,
    Cover,
}

/// `pro_layout`, `pro_theme`, `pro_style`, `pro_theme_interval` from trofeo.conf.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub layouts: Vec<Layout>,
    pub themes: Vec<Theme>,
    pub style: Style,
    /// Minutes between two themes when several are listed (0 = only the first).
    pub interval_min: u32,
    /// Accent colors taken from the cover of the song playing.
    pub cover_color: bool,
    /// Pop-up with the cover when a new song starts.
    pub song_popup: bool,
    /// Portrait layouts: turn the picture counter-clockwise instead of clockwise.
    pub rotate_ccw: bool,
    /// Layout / theme to switch to while a game reports FPS.
    pub game_layout: Option<Layout>,
    pub game_theme: Option<Theme>,
    /// Show running game logo/cover + title in the music slot across dashboards.
    pub game_media: bool,
    /// Artwork mode in music slot (`logo` | `cover`).
    pub game_art_mode: GameArtMode,
    /// Custom background poster/image path for the `showcase` dashboard.
    pub game_poster: String,
    /// Darken percent (0-100) applied over the `showcase` poster background.
    pub game_bg_dim: u32,
    /// Night schedule (start, end) in minutes after midnight, its theme and brightness (percent).
    pub night: Option<(u32, u32)>,
    pub night_theme: Option<Theme>,
    pub night_dim: u32,
    /// Temperature (°C) that makes the screen flash; 0 = off.
    pub alert_cpu: u32,
    pub alert_gpu: u32,
    /// 12-hour clock with AM / PM.
    pub clock12: bool,
    /// Label of the fan to show as CPU fan (HWiNFO / LibreHardwareMonitor); empty = automatic.
    pub fan_sensor: String,
    pub gpu_fan_sensor: String,
    pub logo: String,
    pub amd_adapter: String,
    pub edit_layout: String,
    pub edit_background: bool,
    pub edit_hidden: String,
    pub edit_text_scale: f32,
    pub edit_widgets: Vec<aperture::custom::Widget>,
    pub custom_enabled: bool,
    pub custom_widgets: Vec<aperture::custom::Widget>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { layouts: vec![Layout::Axis], themes: vec![THEMES[0]], style: Style::Cards, interval_min: 30, cover_color: true, song_popup: true, rotate_ccw: false, game_layout: None, game_theme: None, game_media: true, game_art_mode: GameArtMode::Logo, game_poster: String::new(), game_bg_dim: 45, night: None, night_theme: None, night_dim: 40, alert_cpu: 0, alert_gpu: 0, clock12: false, fan_sensor: String::new(), gpu_fan_sensor: String::new(), logo: String::new(), amd_adapter: String::new(), edit_layout: String::new(), edit_background: false, edit_hidden: String::new(), edit_text_scale: 1., edit_widgets: Vec::new(), custom_enabled: false, custom_widgets: Vec::new() }
    }
}

pub fn theme_names() -> String {
    THEMES.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
}

fn parse_hex(v: &str) -> Option<Rgb> {
    let h = v.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn rgb_to_hsv(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d < 1e-6 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if mx <= 0.0 { 0.0 } else { d / mx }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Rgb {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h.rem_euclid(360.0) / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8)
}

impl Theme {
    /// A full theme grown from one color: the CPU takes the color itself, the other
    /// sections are spread around the color wheel (the look of HuyBruh Dash 3.x).
    /// Very dark colors are lifted so text in that color stays readable.
    pub fn from_accent(c: Rgb) -> Theme {
        let c = if c.0.max(c.1).max(c.2) < 120 { lerp_rgb(c, WHITE, 0.5) } else { c };
        let (h, s, _) = rgb_to_hsv(c);
        let s = s.clamp(0.5, 0.9);
        let pair = |dh: f32| {
            let b = hsv_to_rgb((h + dh).rem_euclid(360.0), s, 0.96);
            [b, lerp_rgb(b, WHITE, 0.45)]
        };
        let cpu = [c, lerp_rgb(c, WHITE, 0.45)];
        let net = pair(205.0);
        Theme {
            name: "custom",
            bg: (0, 0, 0),
            panel: (12, 13, 17),
            text: (242, 243, 246),
            muted: (168, 172, 184),
            dim: (118, 122, 136),
            cpu,
            gpu: pair(150.0),
            ram: pair(-60.0),
            ssd: pair(45.0),
            net,
            up: net[1],
            fps: (163, 230, 53),
            time: (255, 255, 255),
        }
    }
}

fn theme_of(name: &str, accent: Option<Rgb>, key: &str) -> anyhow::Result<Theme> {
    if name.starts_with('#') {
        return parse_hex(name).map(Theme::from_accent).ok_or_else(|| anyhow::anyhow!("{key}: '{name}' is not a color (#RRGGBB)"));
    }
    if name == "custom" {
        return accent.map(Theme::from_accent).ok_or_else(|| anyhow::anyhow!("{key}: 'custom' needs pro_accent = #RRGGBB"));
    }
    THEMES
        .iter()
        .find(|t| t.name == name)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("{key}: '{name}' is not a theme ({}, custom, #RRGGBB, all)", theme_names()))
}

fn parse_time_range(key: &str, v: &str) -> anyhow::Result<(u32, u32)> {
    let bad = || anyhow::anyhow!("{key}: '{v}' not valid (HH:MM-HH:MM, e.g. 22:00-07:00)");
    let (a, b) = v.split_once('-').ok_or_else(bad)?;
    let hm = |t: &str| -> Option<u32> {
        let (h, m) = t.trim().split_once(':')?;
        let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    };
    Ok((hm(a).ok_or_else(bad)?, hm(b).ok_or_else(bad)?))
}

pub fn parse(file: &ConfigFile) -> anyhow::Result<Settings> {
    let mut s = Settings::default();
    s.gpu_fan_sensor = file.get("pro_gpu_fan_sensor").unwrap_or("").trim().to_string();
    s.amd_adapter = file.get("pro_amd_adapter").unwrap_or("").trim().to_string();
    s.logo = file.get("pro_logo").unwrap_or("").trim().to_string();
    s.edit_background=file.get("background").is_some_and(|v|!v.trim().is_empty());
    s.edit_hidden=file.get("pro_edit_hidden").unwrap_or("").to_string();
    s.edit_text_scale=file.get_f32("pro_edit_text_scale").map_err(anyhow::Error::msg)?.unwrap_or(1.).clamp(0.85,1.15);
    s.edit_layout=file.get("pro_edit_layout").unwrap_or("").to_string();
    s.edit_widgets=aperture::custom::parse(file.get("pro_edit_widgets").unwrap_or("")).map_err(anyhow::Error::msg)?;
    s.custom_enabled = file.get_bool("pro_custom_enabled").map_err(anyhow::Error::msg)?.unwrap_or(false);
    s.custom_widgets = aperture::custom::parse(file.get("pro_custom_widgets").unwrap_or("")).map_err(anyhow::Error::msg)?;
    let accent = match file.get("pro_accent") {
        Some(v) => Some(parse_hex(v).ok_or_else(|| anyhow::anyhow!("pro_accent: '{v}' is not a color (#RRGGBB)"))?),
        None => None,
    };
    if let Some(v) = file.get("pro_theme") {
        let mut list = Vec::new();
        for name in v.split(',').map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()) {
            if name == "all" {
                list.extend_from_slice(APERTURE_THEMES);
                continue;
            }
            list.push(theme_of(&name, accent, "pro_theme")?);
        }
        if !list.is_empty() {
            s.themes = list;
        }
    } else if let Some(c) = accent {
        s.themes = vec![Theme::from_accent(c)];
    }
    for (key, slot) in [("pro_game_theme", 0), ("pro_night_theme", 1)] {
        if let Some(v) = file.get(key).filter(|v| !v.trim().is_empty()) {
            let t = theme_of(&v.trim().to_ascii_lowercase(), accent, key)?;
            if slot == 0 {
                s.game_theme = Some(t);
            } else {
                s.night_theme = Some(t);
            }
        }
    }
    if let Some(v) = file.get("pro_game_layout").filter(|v| !v.trim().is_empty()) {
        let name = v.trim().to_ascii_lowercase();
        s.game_layout = Some(Layout::find(&name).ok_or_else(|| anyhow::anyhow!("pro_game_layout: '{name}' is not a layout"))?);
    }
    if let Some(v) = file.get("pro_night") {
        if !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "off" | "no" | "false") {
            s.night = Some(parse_time_range("pro_night", v.trim())?);
        }
    }
    if let Some(v) = file.get_u32("pro_night_dim").map_err(|e| anyhow::anyhow!(e))? {
        if !(10..=100).contains(&v) {
            anyhow::bail!("pro_night_dim: 10-100 (percent brightness at night)");
        }
        s.night_dim = v;
    }
    for (key, slot) in [("pro_alert_cpu", 0), ("pro_alert_gpu", 1)] {
        if let Some(v) = file.get_u32(key).map_err(|e| anyhow::anyhow!(e))? {
            if v != 0 && !(40..=120).contains(&v) {
                anyhow::bail!("{key}: 0 (off) or 40-120 °C");
            }
            if slot == 0 {
                s.alert_cpu = v;
            } else {
                s.alert_gpu = v;
            }
        }
    }
    if let Some(v) = file.get("pro_fan_sensor") {
        s.fan_sensor = v.trim().to_string();
    }
    if let Some(v) = file.get("pro_clock") {
        s.clock12 = match v.trim().to_ascii_lowercase().as_str() {
            "12" | "12h" | "am/pm" | "ampm" => true,
            "24" | "24h" => false,
            _ => anyhow::bail!("pro_clock: '{v}' not valid (24h | 12h)"),
        };
    }
    if let Some(v) = file.get("pro_layout") {
        let mut list = Vec::new();
        for name in v.split(',').map(|n| n.trim().to_ascii_lowercase()).filter(|n| !n.is_empty()) {
            if name == "all" {
                list.extend_from_slice(&Layout::CURATED);
                continue;
            }
            if name == "portrait" {
                list.extend_from_slice(&Layout::PORTRAIT);
                continue;
            }
            let l = Layout::find(&name).ok_or_else(|| {
                anyhow::anyhow!(
                    "pro_layout: '{name}' is not valid ({}, all, portrait)",
                    Layout::ALL.iter().chain(Layout::PORTRAIT.iter()).map(|l| l.name()).collect::<Vec<_>>().join(", ")
                )
            })?;
            list.push(l);
        }
        if !list.is_empty() {
            s.layouts = list;
        }
    }
    if let Some(v) = file.get("pro_style") {
        s.style = match v.trim().to_ascii_lowercase().as_str() {
            "cards" | "card" | "solid" => Style::Cards,
            "glass" | "glow" => Style::Glass,
            "flat" | "minimal" | "lines" => Style::Flat,
            _ => anyhow::bail!("pro_style: '{v}' not valid (cards | glass | flat)"),
        };
    }
    if let Some(v) = file.get_u32("pro_theme_interval").map_err(|e| anyhow::anyhow!(e))? {
        if v > 1440 {
            anyhow::bail!("pro_theme_interval: 0-1440 minutes");
        }
        s.interval_min = v;
    }
    let flag = |key: &str, v: &str| -> anyhow::Result<bool> {
        match v.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Ok(true),
            "false" | "off" | "no" | "0" => Ok(false),
            _ => anyhow::bail!("{key}: '{v}' not valid (true | false)"),
        }
    };
    if let Some(v) = file.get("pro_cover_color") {
        s.cover_color = flag("pro_cover_color", v)?;
    }
    if let Some(v) = file.get("pro_song_popup") {
        s.song_popup = flag("pro_song_popup", v)?;
    }
    if let Some(v) = file.get("pro_rotate") {
        s.rotate_ccw = match v.trim().to_ascii_lowercase().as_str() {
            "cw" | "clockwise" | "90" | "right" => false,
            "ccw" | "counterclockwise" | "270" | "left" => true,
            _ => anyhow::bail!("pro_rotate: '{v}' not valid (cw | ccw)"),
        };
    }
    if let Some(v) = file.get("pro_game_media") {
        s.game_media = flag("pro_game_media", v)?;
    }
    if let Some(v) = file.get("pro_game_art") {
        s.game_art_mode = match v.trim().to_ascii_lowercase().as_str() {
            "logo" => GameArtMode::Logo,
            "cover" | "poster" => GameArtMode::Cover,
            _ => anyhow::bail!("pro_game_art: '{v}' not valid (logo | cover)"),
        };
    }
    if let Some(v) = file.get("pro_game_poster") {
        s.game_poster = v.trim().to_string();
    }
    if let Some(v) = file.get_u32("pro_game_bg_dim").map_err(|e| anyhow::anyhow!(e))? {
        if v > 100 {
            anyhow::bail!("pro_game_bg_dim: must be between 0 and 100");
        }
        s.game_bg_dim = v;
    }
    Ok(s)
}

thread_local! {
    static CURRENT: std::cell::Cell<(Theme, Style)> = std::cell::Cell::new((THEMES[0], Style::Cards));
}

static CLOCK_12H: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the game-session layout shows (filled by [`ProScreen`] before painting).
#[derive(Clone, Default)]
struct SessionStats {
    /// FPS of the last two minutes, one sample per second (0 = no game).
    fps: Vec<f32>,
    avg: f32,
    max: f32,
    /// Average of the slowest 1 % of the samples.
    low1: f32,
    peak_cpu: f32,
    peak_gpu: f32,
    secs: u64,
    gaming: bool,
}

thread_local! {
    static STATS: std::cell::RefCell<SessionStats> = std::cell::RefCell::new(SessionStats::default());
}

fn th() -> Theme {
    CURRENT.with(|c| c.get().0)
}

fn style() -> Style {
    CURRENT.with(|c| c.get().1)
}

/// The theme and layout in use now: the only ones, or those of this time slot when
/// several rotate (every `interval_min` minutes).
fn pick(s: &Settings) -> (Theme, Layout) {
    let slot = if s.interval_min == 0 { 0 } else { (Local::now().timestamp().max(0) as u64) / 60 / s.interval_min as u64 };
    let t = if s.themes.is_empty() { THEMES[0] } else { s.themes[(slot % s.themes.len() as u64) as usize] };
    let l = if s.layouts.is_empty() { Layout::Classic } else { s.layouts[(slot % s.layouts.len() as u64) as usize] };
    (t, l)
}

/// Color right under the content (to "cut" shapes, e.g. the moon).
fn under() -> Rgb {
    if style() == Style::Flat { th().bg } else { th().panel }
}

const WARN: Rgb = (251, 146, 60);
const HOT: Rgb = (248, 82, 82);

/// Seconds of history in the graphs (one sample per second).
const HIST: usize = 120;

/// Everything the screen shows (filled by the main loop or the preview).
#[derive(Clone, Copy)]
pub struct Data<'a> {
    pub cpu_name: &'a str,
    pub cores: (usize, usize),
    pub cpu_pct: f32,
    pub cpu_temp: Option<f32>,
    pub cpu_power: Option<f32>,
    pub cpu_mhz: Option<u32>,
    pub gpu_pct: Option<f32>,
    pub gpu: &'a GpuAmdData,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
    pub disk: Option<&'a DiskInfo>,
    /// CPU fan (motherboard, HWiNFO or LibreHardwareMonitor), when available.
    pub cpu_fan_rpm: Option<u32>,
    /// Every fan header of the motherboard (for the pump / case fan readings).
    pub fans: &'a [crate::superio::Fan],
    /// Read / write, MB/s.
    pub disk_rate: (f64, f64),
    /// Down / up, KB/s.
    pub net_kb: (f64, f64),
    pub weather: Option<&'a WeatherSnapshot>,
    pub fahrenheit: bool,
    pub uptime_s: u64,
    pub track: Option<&'a Arc<TrackInfo>>,
    pub game: Option<&'a Arc<crate::game_info::GameInfo>>,
}

struct Cover {
    key: usize,
    /// Side of `rgb`: the full picture the player gave (up to 640 px), not a thumbnail.
    size: u32,
    rgb: Vec<u8>,
    accent: Rgb,
    /// Three distinct colors of the cover (for `pro_cover_color`).
    palette: [Rgb; 3],
    /// The picture resized (once, with a high-quality filter) to the sizes it is drawn at.
    scaled: std::sync::Mutex<Vec<(u32, Vec<u8>)>>,
}

impl Cover {
    fn new(key: usize, size: u32, rgb: Vec<u8>) -> Cover {
        // colors from a small copy: fast, and the same result as before
        let small = scale_square(&rgb, size, size, 64);
        let accent = cover_accent(&small);
        let palette = cover_palette(&small, accent);
        Cover { key, size, rgb, accent, palette, scaled: std::sync::Mutex::new(Vec::new()) }
    }

    /// The cover at `n` x `n` pixels. Shrinking uses Lanczos (sharp, no shimmer), growing a
    /// small picture uses Catmull-Rom plus a light unsharp mask, so it stays crisp instead of
    /// the soft look of plain bilinear stretching.
    fn at(&self, n: u32) -> Vec<u8> {
        if let Ok(cache) = self.scaled.lock() {
            if let Some((_, px)) = cache.iter().find(|(s, _)| *s == n) {
                return px.clone();
            }
        }
        let px = if n == self.size {
            self.rgb.clone()
        } else {
            let src = image::RgbImage::from_raw(self.size, self.size, self.rgb.clone())
                .unwrap_or_else(|| image::RgbImage::new(self.size.max(1), self.size.max(1)));
            let grow = n > self.size;
            let filter = if grow { image::imageops::FilterType::CatmullRom } else { image::imageops::FilterType::Lanczos3 };
            let out = image::imageops::resize(&src, n, n, filter);
            let out = if grow && n >= self.size * 5 / 4 { image::imageops::unsharpen(&out, 0.8, 2) } else { out };
            out.into_raw()
        };
        if let Ok(mut cache) = self.scaled.lock() {
            if cache.len() >= 4 {
                cache.remove(0);
            }
            cache.push((n, px.clone()));
        }
        px
    }
}

pub struct ProScreen {
    layer: Layer,
    sig: String,
    cpu: VecDeque<f32>,
    gpu: VecDeque<f32>,
    net: VecDeque<f32>,
    last_push: Option<Instant>,
    hist_ver: u64,
    cover: Option<Cover>,
    game_cover: Option<(String, Cover)>,
    showcase_bg: Option<(String, Layer)>,
    logo: (String, Option<Cover>),
    /// Layout / colors drawn last, and the frame they made: a change cross-fades from it.
    look: String,
    fade: Option<(Vec<u8>, Instant)>,
    /// Titles too long for their box, scrolled on every frame, and since when.
    marquees: Vec<MarqueeSpec>,
    marquee_since: Instant,
    marquee_k: f32,
    /// Last master volume (percent, muted) and since when the volume pop-up shows.
    vol: Option<(f32, bool)>,
    vol_since: Option<Instant>,
    osd: Layer,
    /// Portrait layouts: the canvas is drawn upright here, then turned into `layer`.
    canvas: Layer,
    portrait: bool,
    rotate_ccw: bool,
    /// Last song seen (title, artist), whether the first frame passed, and since when its pop-up shows.
    last_song: Option<(String, String)>,
    song_init: bool,
    song_since: Option<Instant>,
    /// FPS history (one sample per second) and every FPS sample of the current game session.
    fps: VecDeque<f32>,
    fps_samples: Vec<f32>,
    peak_cpu: f32,
    peak_gpu: f32,
    session_start: Instant,
    game_since: Option<Instant>,
    fps_last: Option<Instant>,
    alert_cpu: bool,
    alert_gpu: bool,
    alert_temps: (Option<f32>, Option<f32>),
    alert_since: Instant,
    /// Start-up logo: when it appeared (None = not shown yet), and whether it is over.
    splash_since: Option<Instant>,
    splash_done: bool,
}

/// Multiplies every pixel of `area` by `pct` percent (night dimming).
fn dim_area(fb: &mut Framebuffer, area: (u32, u32, u32, u32), pct: u32) {
    let lut: Vec<u8> = (0..256u32).map(|v| (v * pct / 100) as u8).collect();
    let fw = fb.width();
    let rows = area.1..(area.1 + area.3).min(fb.height());
    let (x0, x1) = (area.0 as usize * 3, (area.0 + area.2).min(fw) as usize * 3);
    let bytes = fb.as_bytes_mut();
    for y in rows {
        let row = &mut bytes[y as usize * fw as usize * 3..(y as usize + 1) * fw as usize * 3];
        for b in &mut row[x0..x1] {
            *b = lut[*b as usize];
        }
    }
}

fn blend_rect(fb: &mut Framebuffer, x: u32, y: u32, w: u32, h: u32, c: Rgb, a: u8) {
    let fw = fb.width();
    let fh = fb.height();
    let bytes = fb.as_bytes_mut();
    let a = a as u32;
    for yy in y..(y + h).min(fh) {
        for xx in x..(x + w).min(fw) {
            let i = ((yy * fw + xx) * 3) as usize;
            for (ch, src) in [c.0, c.1, c.2].into_iter().enumerate() {
                bytes[i + ch] = ((bytes[i + ch] as u32 * (255 - a) + src as u32 * a) / 255) as u8;
            }
        }
    }
}

/// How long the start-up logo stays on screen.
const SPLASH_SECS: f32 = 2.4;

/// How long the new-song pop-up stays on screen.
const SONG_SECS: f32 = 3.2;

/// How long the volume pop-up stays on screen after the last change.
const OSD_SECS: f32 = 1.8;

impl ProScreen {
    pub fn new() -> Self {
        ProScreen {
            layer: Layer::new(1, 1),
            sig: String::new(),
            cpu: VecDeque::new(),
            gpu: VecDeque::new(),
            net: VecDeque::new(),
            last_push: None,
            hist_ver: 0,
            cover: None,
            game_cover: None,
            showcase_bg: None,
            logo: (String::new(), None),
            look: String::new(),
            fade: None,
            marquees: Vec::new(),
            marquee_since: Instant::now(),
            marquee_k: 1.0,
            vol: None,
            vol_since: None,
            osd: Layer::new(1, 1),
            canvas: Layer::new(1, 1),
            portrait: false,
            rotate_ccw: false,
            last_song: None,
            song_init: false,
            song_since: None,
            fps: VecDeque::new(),
            fps_samples: Vec::new(),
            peak_cpu: 0.0,
            peak_gpu: 0.0,
            session_start: Instant::now(),
            game_since: None,
            fps_last: None,
            alert_cpu: false,
            alert_gpu: false,
            alert_temps: (None, None),
            alert_since: Instant::now(),
            splash_since: None,
            splash_done: false,
        }
    }

    /// Fills the graphs with plausible values (preview only).
    pub fn seed_demo_history(&mut self) {
        // previews show the dashboard, not the start-up logo (TROFEO_SPLASH_PREVIEW shows it)
        self.splash_done = std::env::var_os("TROFEO_SPLASH_PREVIEW").is_none();
        if !self.splash_done {
            self.splash_since = Instant::now().checked_sub(std::time::Duration::from_secs(1));
        }
        for i in 0..HIST {
            let t = i as f32;
            self.cpu.push_back((22.0 + 14.0 * (t / 9.0).sin() + 9.0 * (t / 3.7).cos()).clamp(2.0, 100.0));
            self.gpu.push_back((40.0 + 25.0 * (t / 13.0).sin() + 8.0 * (t / 2.9).sin()).clamp(2.0, 100.0));
            self.net.push_back((700.0 + 600.0 * (t / 6.0).sin().max(0.0) + 150.0 * (t / 2.3).cos()).max(0.0));
        }
        for i in 0..HIST {
            let t = i as f32;
            self.fps.push_back((138.0 + 14.0 * (t / 7.0).sin() + 9.0 * (t / 2.3).cos() - if i % 37 == 0 { 38.0 } else { 0.0 }).max(30.0));
        }
        self.fps_samples = self.fps.iter().copied().collect();
        self.peak_cpu = 74.0;
        self.peak_gpu = 71.0;
        self.session_start = Instant::now().checked_sub(std::time::Duration::from_secs(47 * 60 + 12)).unwrap_or_else(Instant::now);
        self.game_since = Some(Instant::now().checked_sub(std::time::Duration::from_secs(60)).unwrap_or_else(Instant::now));
        self.hist_ver += 1;
        self.last_push = Some(Instant::now());
    }

    fn push_history(&mut self, d: &Data) {
        if self.last_push.is_some_and(|t| t.elapsed().as_millis() < 1000) {
            return;
        }
        self.last_push = Some(Instant::now());
        let fps = d.gpu.fps.filter(|f| *f > 0).map_or(0.0, |f| f as f32);
        for (q, v) in [(&mut self.cpu, d.cpu_pct), (&mut self.gpu, d.gpu_pct.unwrap_or(0.0)), (&mut self.net, d.net_kb.0 as f32), (&mut self.fps, fps)] {
            q.push_back(v);
            while q.len() > HIST {
                q.pop_front();
            }
        }
        if fps > 0.0 {
            self.fps_samples.push(fps);
            if self.fps_samples.len() > 7200 {
                self.fps_samples.remove(0);
            }
        }
        self.peak_cpu = self.peak_cpu.max(d.cpu_temp.unwrap_or(0.0));
        self.peak_gpu = self.peak_gpu.max(d.gpu.temp_edge_c.map_or(0.0, |t| t as f32));
        self.hist_ver += 1;
    }

    /// Tracks whether a game is running (FPS reported for 3 s, released after 10 s without).
    /// A new session starts the statistics from zero.
    fn update_game(&mut self, d: &Data) -> bool {
        let live = d.gpu.fps.is_some_and(|f| f > 0);
        if live {
            self.fps_last = Some(Instant::now());
            if self.game_since.is_none() {
                self.game_since = Some(Instant::now());
                self.fps_samples.clear();
                self.peak_cpu = 0.0;
                self.peak_gpu = 0.0;
                self.session_start = Instant::now();
            }
        } else if self.fps_last.is_some_and(|t| t.elapsed().as_secs() >= 10) {
            self.game_since = None;
            self.fps_last = None;
        }
        self.game_since.is_some_and(|t| t.elapsed().as_secs() >= 3)
    }

    fn session_stats(&self, gaming: bool) -> SessionStats {
        let mut v = self.fps_samples.clone();
        let n = v.len();
        let (avg, max) = if n == 0 { (0.0, 0.0) } else { (v.iter().sum::<f32>() / n as f32, v.iter().cloned().fold(0.0, f32::max)) };
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let k = (n / 100).max(1).min(n.max(1));
        let low1 = if n == 0 { 0.0 } else { v[..k].iter().sum::<f32>() / k as f32 };
        SessionStats { fps: self.fps.iter().copied().collect(), avg, max, low1, peak_cpu: self.peak_cpu, peak_gpu: self.peak_gpu, secs: self.session_start.elapsed().as_secs(), gaming }
    }

    /// Hot-temperature alerts with 3 °C of hysteresis.
    fn update_alerts(&mut self, d: &Data, s: &Settings) {
        let cpu = d.cpu_temp;
        let gpu = d.gpu.temp_edge_c.map(|t| t as f32);
        let st = |on: bool, t: Option<f32>, limit: u32| -> bool {
            match (limit, t) {
                (0, _) | (_, None) => false,
                (l, Some(t)) => {
                    if on {
                        t >= l as f32 - 3.0
                    } else {
                        t >= l as f32
                    }
                }
            }
        };
        let (c, g) = (st(self.alert_cpu, cpu, s.alert_cpu), st(self.alert_gpu, gpu, s.alert_gpu));
        if (c || g) && !(self.alert_cpu || self.alert_gpu) {
            self.alert_since = Instant::now();
        }
        self.alert_cpu = c;
        self.alert_gpu = g;
        self.alert_temps = (cpu, gpu);
    }

    fn alert_active(&self) -> bool {
        self.alert_cpu || self.alert_gpu
    }

    /// Pulsing red frame around the screen and a pill with the hot reading(s).
    fn paint_alert(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32)) {
        if !self.alert_active() {
            return;
        }
        let t = self.alert_since.elapsed().as_secs_f32();
        let pulse = 0.5 + 0.5 * (t * std::f32::consts::TAU / 1.1).sin();
        let a = (60.0 + 170.0 * pulse) as u8;
        let b = ((area.2.min(area.3) as f32 / 462.0).max(0.6) * 7.0).round().max(3.0) as u32;
        let (x, y, w, h) = area;
        for (rx, ry, rw, rh) in [(x, y, w, b), (x, y + h - b, w, b), (x, y + b, b, h - 2 * b), (x + w - b, y + b, b, h - 2 * b)] {
            blend_rect(fb, rx, ry, rw, rh, HOT, a);
        }
        let mut parts = Vec::new();
        if self.alert_cpu {
            parts.push(format!("CPU {:.0}°C", self.alert_temps.0.unwrap_or(0.0)));
        }
        if self.alert_gpu {
            parts.push(format!("GPU {:.0}°C", self.alert_temps.1.unwrap_or(0.0)));
        }
        let txt = format!("HOT  ·  {}", parts.join("  ·  "));
        let k = (area.3 as f32 / 462.0).min(area.2 as f32 / 1920.0 * 1.15).clamp(0.5, 2.0) * if self.portrait { 0.6 } else { 1.0 };
        let cap = 22.0 * k;
        let (tw, h_) = (crate::smooth_font::text_width(&txt, crate::smooth_font::px_for_cap(cap), crate::smooth_font::Weight::Medium), (48.0 * k).round());
        let w_ = (tw + 56.0 * k).round();
        if self.osd.w != w_ as usize || self.osd.h != h_ as usize {
            self.osd = Layer::new(w_ as u32, h_ as u32);
        } else {
            self.osd.clear();
        }
        let mut p = Painter { layer: &mut self.osd, font: FontStyle::Smooth };
        let c = lerp_rgb(HOT, (120, 10, 10), 0.45 * (1.0 - pulse));
        p.layer.fill_rrect((0.0, 0.0, w_, h_), h_ / 2.0, &move |_| c, 245);
        p.text(28.0 * k, h_ / 2.0 + cap / 2.0, &txt, cap, W::Medium, WHITE, 255);
        // top edge of the screen (the side that is "up" when the panel is turned)
        let (aw, ah) = (area.2 as f32, area.3 as f32);
        let margin = (b as f32 + 10.0 * k).round() as u32;
        if self.portrait {
            let mut rot = Layer::new(self.osd.h as u32, self.osd.w as u32);
            self.osd.rotate_into(&mut rot, self.rotate_ccw);
            let ox = if self.rotate_ccw { area.0 + margin } else { area.0 + (aw as u32).saturating_sub(rot.w as u32 + margin) };
            let oy = area.1 + ((ah - rot.h as f32) / 2.0).max(0.0) as u32;
            rot.composite_onto(fb, ox, oy);
        } else {
            let ox = area.0 + ((aw - self.osd.w as f32) / 2.0).max(0.0) as u32;
            self.osd.composite_onto(fb, ox, area.1 + margin);
        }
    }

    fn update_cover(&mut self, d: &Data) {
        let Some(cover) = d.track.and_then(|t| t.cover.as_ref()) else {
            self.cover = None;
            return;
        };
        let key = Arc::as_ptr(cover) as usize;
        if self.cover.as_ref().is_some_and(|c| c.key == key) {
            return;
        }
        // keep the whole picture: it is resized once per drawn size in `Cover::at`
        let size = cover.width.min(cover.height).clamp(1, 640);
        let rgb = if cover.width == size && cover.height == size {
            cover.rgb.clone()
        } else {
            scale_square(&cover.rgb, cover.width, cover.height, size)
        };
        self.cover = Some(Cover::new(key, size, rgb));
    }

    fn update_game_cover(&mut self, game: Option<&Arc<crate::game_info::GameInfo>>, mode: GameArtMode, bg: Rgb) {
        let Some(g) = game else {
            self.game_cover = None;
            return;
        };
        let key = format!(
            "{}|{:?}|{}|{}|{}|{:?}",
            g.key,
            mode,
            g.logo.is_some(),
            g.poster.is_some(),
            g.hero.is_some(),
            bg
        );
        if self.game_cover.as_ref().is_some_and(|(k, _)| k == &key) {
            return;
        }
        let rgb = match mode {
            GameArtMode::Logo => g
                .logo
                .as_ref()
                .map(|l| crate::game_info::logo_as_square_cover(l, bg.into(), 256))
                .or_else(|| {
                    g.poster
                        .as_ref()
                        .or(g.hero.as_ref())
                        .map(|p| crate::game_info::poster_as_square_cover(p, 256))
                }),
            GameArtMode::Cover => g
                .poster
                .as_ref()
                .or(g.hero.as_ref())
                .map(|p| crate::game_info::poster_as_square_cover(p, 256))
                .or_else(|| {
                    g.logo
                        .as_ref()
                        .map(|l| crate::game_info::logo_as_square_cover(l, bg.into(), 256))
                }),
        };
        self.game_cover = rgb.map(|px| (key, Cover::new(1, 256, px)));
    }

    /// Paints the screen on `fb`; `area` = the part of `fb` the interface may use.
    pub fn draw(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32), d: &Data) {
        let settings = opts().pro;
        CLOCK_12H.store(settings.clock12, std::sync::atomic::Ordering::Relaxed);
        let now = Local::now();
        let (mut theme, mut layout) = pick(&settings);
        self.push_history(d);
        self.update_cover(d);
        if self.logo.0 != settings.logo {
            let cover = (|| {
                let mut reader = image::ImageReader::open(&settings.logo).ok()?;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(4096);
                limits.max_image_height = Some(4096);
                reader.limits(limits);
                let image = reader.decode().ok()?.thumbnail(256, 256).to_rgba8();
                let mut square = image::RgbImage::new(256, 256);
                let x = (256 - image.width()) / 2;
                let y = (256 - image.height()) / 2;
                for (px, py, rgba) in image.enumerate_pixels() {
                    let alpha = u16::from(rgba[3]);
                    square.put_pixel(x + px, y + py, image::Rgb([
                        (u16::from(rgba[0]) * alpha / 255) as u8,
                        (u16::from(rgba[1]) * alpha / 255) as u8,
                        (u16::from(rgba[2]) * alpha / 255) as u8,
                    ]));
                }
                Some(Cover::new(0, 256, square.into_raw()))
            })();
            self.logo = (settings.logo.clone(), cover);
        }
        let gaming = self.update_game(d);
        self.update_alerts(d, &settings);
        let night = settings.night.is_some_and(|(a, b)| {
            let m = now.hour() * 60 + now.minute();
            if a <= b { m >= a && m < b } else { m >= a || m < b }
        });
        if night {
            if let Some(t) = settings.night_theme {
                theme = t;
            }
        }
        if gaming {
            if let Some(l) = settings.game_layout {
                layout = l;
            }
            if let Some(t) = settings.game_theme {
                theme = t;
            }
        }
        let use_game_media = settings.game_media && d.game.is_some();
        self.update_game_cover(use_game_media.then_some(d.game).flatten(), settings.game_art_mode, theme.panel);
        let game_track: Option<Arc<TrackInfo>> = if use_game_media {
            d.game.map(|g| {
                Arc::new(TrackInfo {
                    title: g.title.clone(),
                    artist: if g.source == "Detected" {
                        "In game".to_string()
                    } else {
                        format!("In game · {}", g.source)
                    },
                    album: String::new(),
                    cover: None,
                    timeline: None,
                })
            })
        } else {
            None
        };
        let eff_d = if use_game_media && layout != Layout::Showcase {
            Data { track: game_track.as_ref(), ..*d }
        } else {
            *d
        };
        let osd_d = if use_game_media {
            Data { track: game_track.as_ref(), ..*d }
        } else {
            *d
        };
        crate::smooth_font::set_family(if layout.clean() { crate::smooth_font::Family::Inter } else { crate::smooth_font::Family::Chakra });
        if settings.cover_color {
            let color_src = if use_game_media {
                self.game_cover.as_ref().map(|c| &c.1).or(self.cover.as_ref())
            } else {
                self.cover.as_ref()
            };
            if let Some(c) = color_src {
                let pair = |c: Rgb| [c, lerp_rgb(c, WHITE, 0.45)];
                theme.cpu = pair(c.palette[0]);
                theme.gpu = pair(c.palette[1]);
                theme.ram = pair(c.palette[2]);
                theme.ssd = pair(lerp_rgb(c.palette[0], c.palette[2], 0.5));
                theme.net = pair(c.palette[1]);
            }
        }
        CURRENT.with(|c| c.set((theme, settings.style)));
        // a new layout or new colors: keep the last frame to cross-fade from
        let look = format!("{}|{}|{:?}|{}", layout.name(), theme.name, theme.cpu, settings.custom_enabled);
        if look != self.look {
            if !self.look.is_empty() && self.splash_done {
                self.fade = Some((area_bytes(fb, area), Instant::now()));
            }
            self.look = look;
        }
        if !settings.custom_enabled && !(settings.edit_background && settings.edit_layout==layout.name()) { fb.clear(theme.bg.0, theme.bg.1, theme.bg.2); }
        // a new song (or newly launched game) opens its pop-up (not at start-up, not when the same item resumes)
        let key = osd_d.track.map(|t| (t.title.clone(), t.artist.clone()));
        if let Some(k) = key {
            if self.song_init && self.last_song.as_ref() != Some(&k) && settings.song_popup {
                self.song_since = Some(Instant::now());
            }
            self.last_song = Some(k);
        }
        self.song_init = true;
        let progress = eff_d.track.and_then(|t| t.timeline).map(|tl| (tl.position_now_ms() / 1000, tl.duration_ms / 1000));
        let game_sig = d.game.map(|g| (
            g.key.as_str(),
            g.title.as_str(),
            g.logo.is_some(),
            g.poster.is_some(),
            g.hero.is_some(),
        ));
        let sig = format!(
            "{:?}{:?}{:?}|{area:?}|{:?}|{}|{}|{:.0}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{:?}|{:.1}{:.1}|{:.0}{:.0}|{:?}|{}|{:?}|{:?}|{}",
            theme,
            settings.style,
            layout,
            d.cpu_fan_rpm,
            now.format("%H:%M:%S %F"),
            d.cpu_name,
            d.cpu_pct,
            d.cpu_temp.map(|v| v.round()),
            d.cpu_power.map(|v| v.round()),
            d.cpu_mhz.map(|m| m / 10),
            d.gpu_pct.map(|v| v.round()),
            d.gpu,
            d.ram_used_mb / 100,
            d.ram_total_mb,
            d.disk,
            d.disk_rate.0,
            d.disk_rate.1,
            d.net_kb.0,
            d.net_kb.1,
            d.weather.map(|w| (w.temp_c.round() as i32, w.code, w.humidity, w.city.clone())),
            d.uptime_s / 60,
            eff_d.track.map(|t| (t.title.clone(), t.artist.clone())),
            progress,
            self.hist_ver,
        ) + &format!(
            "|{}|{:?}|{}|{:?}|{}|{}",
            settings.clock12,
            game_sig,
            settings.game_media,
            settings.game_art_mode,
            settings.game_poster,
            settings.game_bg_dim
        );
        let sig = sig + &format!("|{}|{:?}",settings.custom_enabled,(&settings.custom_widgets,&settings.edit_layout,&settings.edit_widgets,&settings.edit_hidden,settings.edit_text_scale, &settings.logo));
        let portrait = !settings.custom_enabled && layout.portrait();
        if sig != self.sig {
            let (w, h) = (area.2, area.3);
            let (cw, ch) = if portrait { (h, w) } else { (w, h) };
            if self.layer.w != w as usize || self.layer.h != h as usize {
                self.layer = Layer::new(w, h);
            } else if !portrait {
                self.layer.clear();
            }
            if portrait {
                if self.canvas.w != cw as usize || self.canvas.h != ch as usize {
                    self.canvas = Layer::new(cw, ch);
                } else {
                    self.canvas.clear();
                }
            }
            if !settings.custom_enabled && layout == Layout::Showcase {
                showcase::prepare_bg(&mut self.showcase_bg, d, &settings.game_poster, settings.game_bg_dim, cw, ch);
            }
            let stats = self.session_stats(gaming);
            STATS.with(|s| *s.borrow_mut() = stats);
            let cover = if use_game_media {
                self.game_cover.as_ref().map(|c| &c.1).or_else(|| {
                    if !matches!(layout.name(), "axis" | "ledger" | "duet" | "orbit" | "atelier" | "panorama" | "cinema") {
                        self.cover.as_ref()
                    } else {
                        self.logo.1.as_ref().or(self.cover.as_ref())
                    }
                })
            } else if !matches!(layout.name(), "axis" | "ledger" | "duet" | "orbit" | "atelier" | "panorama" | "cinema") {
                self.cover.as_ref()
            } else {
                self.logo.1.as_ref().or(self.cover.as_ref())
            };
            let hist = (&self.cpu, &self.gpu, &self.net);
            let target_layer: &mut Layer = if portrait { &mut self.canvas } else { &mut self.layer };
            let mut p = Painter { layer: target_layer, font: FontStyle::Smooth };
            MARQ.with(|m| m.borrow_mut().clear());
            NO_MARQ.with(|c| c.set(portrait));
            let (wf, hf) = (cw as f32, ch as f32);
            let k = if portrait { (wf / 462.0).clamp(0.5, 2.0) } else { (hf / 462.0).min(wf / 1920.0 * 1.15).clamp(0.5, 2.0) };
            let sc = Scale::new(k);
            if settings.custom_enabled { designs::paint_custom(&mut p,wf,hf,&eff_d,&now,&settings.custom_widgets); } else { match layout {
                Layout::Classic => paint(&mut p, wf, hf, &eff_d, &now, hist, cover),
                Layout::Cockpit => paint_cockpit(&mut p, wf, hf, &sc, &eff_d, &now, cover),
                Layout::Focus => paint_focus(&mut p, wf, hf, &sc, &eff_d, &now, hist, cover),
                Layout::Showcase => showcase::paint(&mut p, wf, hf, &sc, d, &now, hist, cover, self.showcase_bg.as_ref().map(|(_, l)| l)),
                _ => designs::paint(layout, &mut p, wf, hf, &sc, &eff_d, &now, hist, cover),
            }}
            if portrait {
                self.canvas.rotate_into(&mut self.layer, settings.rotate_ccw);
            }
            let found = MARQ.with(|m| std::mem::take(&mut *m.borrow_mut()));
            let same_text = found.iter().map(|m| &m.text).eq(self.marquees.iter().map(|m| &m.text));
            if !same_text {
                self.marquee_since = Instant::now();
            }
            self.marquees = found;
            self.marquee_k = k;
            self.sig = sig;
        }
        self.portrait = portrait;
        self.rotate_ccw = settings.rotate_ccw;
        self.layer.composite_onto(fb, area.0, area.1);
        let t = self.marquee_since.elapsed().as_secs_f32();
        for m in &self.marquees {
            paint_marquee(fb, area.0, area.1, m, self.marquee_k, t);
        }
        if night && settings.night_dim < 100 {
            dim_area(fb, area, settings.night_dim);
        }
        self.paint_alert(fb, area);
        if self.vol_since.is_some_and(|t| t.elapsed().as_secs_f32() < OSD_SECS) {
            self.paint_volume_osd(fb, area);
        } else {
            self.paint_song_osd(fb, area, &osd_d);
        }
        if let Some((old, since)) = &self.fade {
            let t = since.elapsed().as_secs_f32() / FADE_SECS;
            if t >= 1.0 || old.len() != (area.2 * area.3 * 3) as usize {
                self.fade = None;
            } else {
                let e = t * t * (3.0 - 2.0 * t);
                blend_area(fb, area, old, e);
            }
        }
        self.paint_splash(fb, area);
        crate::smooth_font::set_family(crate::smooth_font::Family::Chakra);
    }

    /// The Aperture mark for a moment when the display starts.
    fn paint_splash(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32)) {
        if self.splash_done || self.portrait {
            self.splash_done = true;
            return;
        }
        let since = *self.splash_since.get_or_insert_with(Instant::now);
        let t = since.elapsed().as_secs_f32();
        if t >= SPLASH_SECS {
            self.splash_done = true;
            return;
        }
        crate::smooth_font::set_family(crate::smooth_font::Family::Inter);
        let (aw, ah) = (area.2, area.3);
        let mut layer = Layer::new(aw, ah);
        let (w, h) = (aw as f32, ah as f32);
        let k = (h / 462.0).min(w / 1920.0 * 1.15).clamp(0.5, 2.0);
        let mut p = Painter { layer: &mut layer, font: FontStyle::Smooth };
        p.layer.fill_rrect_solid((0.0, 0.0, w, h), 0.0, (0, 0, 0), 255);
        // the logo fades in, then everything fades out at the end
        let a_in = (t / 0.5).clamp(0.0, 1.0);
        let a_out = ((SPLASH_SECS - t) / 0.45).clamp(0.0, 1.0);
        let a = (255.0 * a_in * a_out) as u8;
        let mark = 96.0 * k;
        let name = "Aperture";
        let ncap = 44.0 * k;
        let sub = "for Thermalright Trofeo Vision";
        let scap = 15.0 * k;
        let nw = p.width(name, ncap, W::Medium);
        let total = mark + 34.0 * k + nw;
        let x0 = ((w - total) / 2.0).round();
        let cy = h / 2.0;
        let colors = [th().cpu[0], th().cpu[0]];
        let _ = colors;
        let ly = cy - mark / 2.0;
        let c = th().cpu[0];
        for (a0,b0) in [((0.12,0.38),(0.12,0.12)),((0.12,0.12),(0.38,0.12)),
            ((0.62,0.12),(0.88,0.12)),((0.88,0.12),(0.88,0.38)),
            ((0.88,0.62),(0.88,0.88)),((0.88,0.88),(0.62,0.88)),
            ((0.38,0.88),(0.12,0.88)),((0.12,0.88),(0.12,0.62)),
            ((0.38,0.62),(0.62,0.38))] {
            capsule(&mut p,(x0+a0.0*mark,ly+a0.1*mark),(x0+b0.0*mark,ly+b0.1*mark),2.8*k,c,a);
        }
        let tx = x0 + mark + 34.0 * k;
        p.text(tx, cy + 4.0 * k, name, ncap, W::Medium, WHITE, a);
        p.text(tx, cy + 4.0 * k + 18.0 * k + scap, sub, scap, W::Regular, (150, 155, 166), a);
        if a_out < 1.0 {
            // fade the black too, revealing the dashboard
            let fade = (255.0 * a_out) as u8;
            let mut out = Layer::new(aw, ah);
            out.fill_rrect_solid((0.0, 0.0, w, h), 0.0, (0, 0, 0), fade);
            out.composite_onto(fb, area.0, area.1);
            layer.clear();
            let mut p = Painter { layer: &mut layer, font: FontStyle::Smooth };
        let _ = colors;
        let ly = cy - mark / 2.0;
        let c = th().cpu[0];
        for (a0,b0) in [((0.12,0.38),(0.12,0.12)),((0.12,0.12),(0.38,0.12)),
            ((0.62,0.12),(0.88,0.12)),((0.88,0.12),(0.88,0.38)),
            ((0.88,0.62),(0.88,0.88)),((0.88,0.88),(0.62,0.88)),
            ((0.38,0.88),(0.12,0.88)),((0.12,0.88),(0.12,0.62)),
            ((0.38,0.62),(0.62,0.38))] {
            capsule(&mut p,(x0+a0.0*mark,ly+a0.1*mark),(x0+b0.0*mark,ly+b0.1*mark),2.8*k,c,a);
        }

            p.text(tx, cy + 4.0 * k, name, ncap, W::Medium, WHITE, a);
            p.text(tx, cy + 4.0 * k + 18.0 * k + scap, sub, scap, W::Regular, (150, 155, 166), a);
        }
        layer.composite_onto(fb, area.0, area.1);
    }

    /// Preview only: show the new-song pop-up as if the song just changed.
    pub fn demo_song_popup(&mut self) {
        self.song_since = Instant::now().checked_sub(std::time::Duration::from_millis(600));
    }

    fn song_osd_active(&self) -> bool {
        self.song_since.is_some_and(|t| t.elapsed().as_secs_f32() < SONG_SECS)
    }

    /// Cover + "now playing" + title + artist, centered, fading in and out.
    fn paint_song_osd(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32), d: &Data) {
        let (Some(since), Some(tr)) = (self.song_since, d.track) else { return };
        let t = since.elapsed().as_secs_f32();
        if t >= SONG_SECS {
            return;
        }
        let a = (t / 0.15).min(1.0) * ((SONG_SECS - t) / 0.4).min(1.0);
        let al = |x: f32| x as u8; // everything at full strength, faded once at the end
        let (aw, ah) = (area.2 as f32, area.3 as f32);
        let k = (ah / 462.0).min(aw / 1920.0 * 1.15).clamp(0.5, 2.0) * if self.portrait { 0.52 } else { 1.0 };
        let sc = Scale::new(k);
        let (w, h) = ((820.0 * k).round(), (164.0 * k).round());
        if self.osd.w != w as usize || self.osd.h != h as usize {
            self.osd = Layer::new(w as u32, h as u32);
        } else {
            self.osd.clear();
        }
        let cover = if d.game.is_some() {
            self.game_cover.as_ref().map(|c| &c.1).or(self.cover.as_ref())
        } else {
            self.cover.as_ref()
        };
        let accent = cover.map_or(th().text, |c| c.accent);
        let mut p = Painter { layer: &mut self.osd, font: FontStyle::Smooth };
        let r = 26.0 * k;
        let bg = lerp_rgb(th().bg, WHITE, 0.07);
        p.layer.fill_rrect((0.0, 0.0, w, h), r, &move |_| bg, al(255.0));
        p.layer.fill_rrect((0.0, 0.0, w, h), r, &move |t| lerp_rgb(accent, th().bg, t), al(26.0));
        p.layer.stroke_rrect((0.0, 0.0, w, h), r, 1.0, WHITE, al(40.0));
        let pad = 16.0 * k;
        let cs = h - 2.0 * pad;
        match cover {
            Some(c) => blit_cover(&mut p, c, pad, pad, cs, 16.0 * k),
            None => {
                p.layer.fill_rrect((pad, pad, cs, cs), 16.0 * k, &|t| lerp_rgb((40, 42, 52), (24, 25, 32), t), al(255.0));
                music_note(&mut p, pad + cs / 2.0, pad + cs / 2.0, cs * 0.34, th().muted);
            }
        }
        let tx = pad + cs + 26.0 * k;
        let tw = w - tx - 28.0 * k;
        circle(&mut p, tx + 5.0 * k, pad + 14.0 * k, 5.0 * k, accent, al(255.0));
        let badge = if d.game.is_some() { "IN GAME" } else { "NOW PLAYING" };
        spaced(&mut p, tx + 18.0 * k, pad + 14.0 * k + sc.s / 2.0, badge, sc.s, W::Medium, th().muted, 2.0 * k);
        let title = clean_title(&tr.title);
        let tcap = sc.l;
        let title = ellipsize(&p, &title, tcap, W::Medium, tw);
        let ty = pad + 30.0 * k + sc.s + tcap;
        p.text(tx, ty, &title, tcap, W::Medium, th().text, al(255.0));
        let artist = ellipsize(&p, &tr.artist, sc.m * 1.25, W::Regular, tw);
        p.text(tx, ty + 18.0 * k + sc.m * 1.25, &artist, sc.m * 1.25, W::Regular, th().muted, al(255.0));
        if a < 1.0 {
            p.layer.scale_alpha(a);
        }
        self.place_osd(fb, area);
    }

    /// Puts the pop-up in the middle of the panel (turned like the screen when it is vertical).
    fn place_osd(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32)) {
        let (aw, ah) = (area.2 as f32, area.3 as f32);
        if self.portrait {
            let mut rot = Layer::new(self.osd.h as u32, self.osd.w as u32);
            self.osd.rotate_into(&mut rot, self.rotate_ccw);
            let ox = area.0 + ((aw - rot.w as f32) / 2.0).max(0.0) as u32;
            let oy = area.1 + ((ah - rot.h as f32) / 2.0).max(0.0) as u32;
            rot.composite_onto(fb, ox, oy);
        } else {
            let ox = area.0 + ((aw - self.osd.w as f32) / 2.0).max(0.0) as u32;
            let oy = area.1 + ((ah - self.osd.h as f32) / 2.0).max(0.0) as u32;
            self.osd.composite_onto(fb, ox, oy);
        }
    }

    /// Feed the current master volume; a change (not the first reading) opens the pop-up.
    pub fn note_volume(&mut self, v: Option<(f32, bool)>) {
        if let Some(v) = v {
            if let Some(old) = self.vol {
                if (old.0 - v.0).abs() > 0.3 || old.1 != v.1 {
                    self.vol_since = Some(Instant::now());
                }
            }
            self.vol = Some(v);
        }
    }

    /// True while the volume pop-up is on screen (the main loop then draws smooth frames).
    pub fn osd_active(&self) -> bool {
        self.vol_since.is_some_and(|t| t.elapsed().as_secs_f32() < OSD_SECS) || self.song_osd_active() || self.alert_active() || !self.splash_done
    }

    /// A clean pill in the middle of the screen: speaker, level bar, percent.
    fn paint_volume_osd(&mut self, fb: &mut Framebuffer, area: (u32, u32, u32, u32)) {
        let (Some(since), Some((level, muted))) = (self.vol_since, self.vol) else { return };
        let t = since.elapsed().as_secs_f32();
        if t >= OSD_SECS {
            return;
        }
        let a = (t / 0.12).min(1.0) * ((OSD_SECS - t) / 0.35).min(1.0);
        let al = |x: f32| (x * a).round().clamp(0.0, 255.0) as u8;
        let (aw, ah) = (area.2 as f32, area.3 as f32);
        let k = (ah / 462.0).min(aw / 1920.0 * 1.15).clamp(0.5, 2.0) * if self.portrait { 0.66 } else { 1.0 };
        let sc = Scale::new(k);
        let (w, h) = ((640.0 * k).round(), (112.0 * k).round());
        if self.osd.w != w as usize || self.osd.h != h as usize {
            self.osd = Layer::new(w as u32, h as u32);
        } else {
            self.osd.clear();
        }
        let mut p = Painter { layer: &mut self.osd, font: FontStyle::Smooth };
        let bg = lerp_rgb(th().bg, WHITE, 0.07);
        p.layer.fill_rrect((0.0, 0.0, w, h), h / 2.0, &move |_| bg, al(255.0));
        p.layer.stroke_rrect((0.0, 0.0, w, h), h / 2.0, 1.0, WHITE, al(40.0));
        let colors = th().gpu;
        let ic = if muted { th().dim } else { th().text };
        // speaker: body + cone
        let (cx, cy) = (h * 0.5 + 4.0 * k, h / 2.0);
        p.layer.fill_rrect((cx - 24.0 * k, cy - 9.0 * k, 11.0 * k, 18.0 * k), 2.5 * k, &move |_| ic, al(255.0));
        let steps = (16.0 * k) as i32;
        for i in 0..=steps {
            let f = i as f32 / steps as f32;
            let x = cx - 13.0 * k + f * 14.0 * k;
            let hh = 9.0 * k + f * 11.0 * k;
            capsule(&mut p, (x, cy - hh), (x, cy + hh), 1.0 * k, ic, al(255.0));
        }
        if muted {
            let (x0, d) = (cx + 9.0 * k, 9.0 * k);
            capsule(&mut p, (x0, cy - d), (x0 + 2.0 * d, cy + d), 2.2 * k, HOT, al(255.0));
            capsule(&mut p, (x0, cy + d), (x0 + 2.0 * d, cy - d), 2.2 * k, HOT, al(255.0));
        } else {
            let waves = if level <= 0.5 { 0 } else if level < 34.0 { 1 } else if level < 67.0 { 2 } else { 3 };
            for i in 0..3 {
                let r = 12.0 * k + i as f32 * 8.0 * k;
                let on = i < waves;
                arc2(&mut p, cx + 2.0 * k, cy, r, 3.0 * k, 52.0, 76.0, &move |_| if on { ic } else { WHITE }, al(if on { 255.0 } else { 30.0 }));
            }
        }
        // level bar
        let bx = h + 30.0 * k;
        let bw = w - bx - 120.0 * k;
        let bh = 8.0 * k;
        let by = cy - bh / 2.0;
        p.layer.fill_rrect((bx, by, bw, bh), bh / 2.0, &|_| WHITE, al(30.0));
        let f = (level / 100.0).clamp(0.0, 1.0);
        if f > 0.0 {
            let fw = (bw * f).max(bh);
            let c = if muted { [th().dim, th().dim] } else { colors };
            p.layer.fill_rrect((bx, by, fw, bh), bh / 2.0, &move |t| lerp_rgb(c[0], c[1], t), al(255.0));
            circle(&mut p, bx + fw - bh / 2.0, cy, bh * 0.95, if muted { th().dim } else { WHITE }, al(255.0));
        }
        // percent
        let txt = format!("{:.0}", level.round());
        let cap = sc.l;
        let tw = p.width(&txt, cap, W::Medium);
        let uw = p.width("%", cap * 0.5, W::Regular);
        let tx = w - 34.0 * k - tw - uw - 4.0 * k;
        p.text(tx, cy + cap / 2.0, &txt, cap, W::Medium, if muted { th().dim } else { th().text }, al(255.0));
        p.text(tx + tw + 4.0 * k, cy + cap / 2.0, "%", cap * 0.5, W::Regular, th().muted, al(255.0));
        self.place_osd(fb, area);
    }

    /// True while a title is scrolling or a cross-fade runs (the main loop then raises the
    /// frame rate).
    pub fn animating(&self) -> bool {
        if self.fade.is_some() {
            return true;
        }
        let t = self.marquee_since.elapsed().as_secs_f32();
        self.marquees.iter().any(|m| {
            let tw = crate::smooth_font::text_width(&m.text, crate::smooth_font::px_for_cap(m.cap), sfw(m.weight));
            marquee_offset(tw, 70.0 * self.marquee_k, MARQ_SPEED * self.marquee_k, t).1
        })
    }
}

/// Seconds of the cross-fade between two layouts or two color themes.
const FADE_SECS: f32 = 0.6;

/// The RGB bytes of `area` of the frame buffer.
fn area_bytes(fb: &Framebuffer, area: (u32, u32, u32, u32)) -> Vec<u8> {
    let (x0, y0, w, h) = area;
    let fw = fb.width() as usize;
    let src = fb.as_bytes();
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for y in y0..y0 + h {
        let i = (y as usize * fw + x0 as usize) * 3;
        out.extend_from_slice(&src[i..i + w as usize * 3]);
    }
    out
}

/// Mixes `old` (the previous frame of `area`) under the new frame: `e` = 0 old, 1 new.
fn blend_area(fb: &mut Framebuffer, area: (u32, u32, u32, u32), old: &[u8], e: f32) {
    let (x0, y0, w, h) = area;
    let fw = fb.width() as usize;
    let a = (e.clamp(0.0, 1.0) * 256.0) as u32;
    let dst = fb.as_bytes_mut();
    for y in 0..h as usize {
        let di = ((y0 as usize + y) * fw + x0 as usize) * 3;
        let si = y * w as usize * 3;
        for (d, o) in dst[di..di + w as usize * 3].iter_mut().zip(&old[si..si + w as usize * 3]) {
            *d = ((u32::from(*d) * a + u32::from(*o) * (256 - a)) >> 8) as u8;
        }
    }
}

// ------------------------------------------------------------------- layout
//
// One type scale for the whole screen (cap heights at 462 px panel height):
//   XL 60  the clock and the ring percentages
//   L  34  every value (temperatures, watts, clocks, RPM, VRAM, FPS, RAM, net, ...)
//   M  17  names, sub-lines, date, weather text
//   S  13.5 labels / headers (spaced capitals) and units
// Spacing is on a 2 px grid; panels share the same header, subtitle and graph rows.

#[derive(Clone, Copy)]
struct Scale {
    k: f32,
    xl: f32,
    l: f32,
    m: f32,
    s: f32,
}

impl Scale {
    fn new(k: f32) -> Self {
        Scale { k, xl: 60.0 * k, l: 34.0 * k, m: 17.0 * k, s: 13.5 * k }
    }
}

fn paint(
    p: &mut Painter,
    w: f32,
    h: f32,
    d: &Data,
    now: &chrono::DateTime<Local>,
    hist: (&VecDeque<f32>, &VecDeque<f32>, &VecDeque<f32>),
    cover: Option<&Cover>,
) {
    let k = (h / 462.0).min(w / 1920.0 * 1.15).clamp(0.5, 2.0);
    let sc = Scale::new(k);
    let m = (10.0 * k).round();
    let gap = (12.0 * k).round();
    let ph = h - 2.0 * m;
    let weights = [268.0f32, 408.0, 584.0, 288.0, 304.0];
    let avail = w - 2.0 * m - gap * (weights.len() as f32 - 1.0);
    let sum: f32 = weights.iter().sum();
    let mut xs = Vec::new();
    let mut x = m;
    for wt in weights {
        let cw = (avail * wt / sum).floor();
        xs.push((x, cw));
        x += cw + gap;
    }
    let top = m;
    let half = ((ph - gap) / 2.0).floor();

    draw_time(p, (xs[0].0, top, xs[0].1, ph), &sc, d, now);
    draw_cpu(p, (xs[1].0, top, xs[1].1, ph), &sc, d, hist.0);
    draw_gpu(p, (xs[2].0, top, xs[2].1, ph), &sc, d, hist.1);
    draw_ram(p, (xs[3].0, top, xs[3].1, half), &sc, d);
    draw_disk(p, (xs[3].0, top + half + gap, xs[3].1, ph - half - gap), &sc, d);
    let media_h = ((ph - gap) * 0.60).floor();
    draw_media(p, (xs[4].0, top, xs[4].1, media_h), &sc, d, cover);
    draw_net(p, (xs[4].0, top + media_h + gap, xs[4].1, ph - media_h - gap), &sc, d, hist.2);
}

// --------------------------------------------------------------- primitives

fn panel(p: &mut Painter, r: R, k: f32, accent: Rgb) {
    let r = (r.0.round(), r.1.round(), r.2.round(), r.3.round());
    let rad = 18.0 * k;
    match style() {
        Style::Cards => {
            p.layer.fill_rrect_solid(r, rad, th().panel, 255);
            p.layer.stroke_rrect(r, rad, 1.0, WHITE, 20);
        }
        Style::Glass => {
            p.layer.fill_rrect_solid(r, rad, th().panel, 255);
            // a soft wash of the accent color fading out downwards
            let (x, y, w, h) = r;
            let glow_h = (h * 0.55).max(1.0);
            for py in (y as i32)..((y + glow_h) as i32) {
                let t = (py as f32 - y) / glow_h;
                let a = (34.0 * (1.0 - t) * (1.0 - t)) as u8;
                if a == 0 {
                    continue;
                }
                for px in (x as i32)..((x + w) as i32) {
                    let cov = rrect_cov(px as f32 + 0.5, py as f32 + 0.5, x, y, w, h, rad);
                    if cov > 0.0 {
                        p.layer.over(px, py, accent, (a as f32 * cov) as u8);
                    }
                }
            }
            p.layer.stroke_rrect(r, rad, 1.0, WHITE, 26);
            // bright top edge
            capsule(p, (x + rad, y + 1.0), (x + w - rad, y + 1.0), 0.6, accent, 120);
        }
        Style::Flat => {
            // hairline dividers between the sections (none on the outer edges)
            let (x, y, w, h) = r;
            if x > 24.0 * k {
                capsule(p, (x - 6.0 * k, y + 14.0 * k), (x - 6.0 * k, y + h - 14.0 * k), 0.5, WHITE, 40);
            }
            if y > 24.0 * k {
                capsule(p, (x + 14.0 * k, y - 6.0 * k), (x + w - 14.0 * k, y - 6.0 * k), 0.5, WHITE, 40);
            }
        }
    }
}

fn pad(k: f32) -> f32 {
    (18.0 * k).round()
}

fn circle(p: &mut Painter, cx: f32, cy: f32, r: f32, c: Rgb, a: u8) {
    p.layer.fill_rrect((cx - r, cy - r, 2.0 * r, 2.0 * r), r, &move |_| c, a);
}

/// A line with round ends, `r` = half the thickness.
fn capsule(p: &mut Painter, a: (f32, f32), b: (f32, f32), r: f32, c: Rgb, alpha: u8) {
    let (x0, x1) = ((a.0.min(b.0) - r - 1.0).floor() as i32, (a.0.max(b.0) + r + 1.0).ceil() as i32);
    let (y0, y1) = ((a.1.min(b.1) - r - 1.0).floor() as i32, (a.1.max(b.1) + r + 1.0).ceil() as i32);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = (dx * dx + dy * dy).max(1e-6);
    for py in y0..y1 {
        for px in x0..x1 {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let t = (((fx - a.0) * dx + (fy - a.1) * dy) / len2).clamp(0.0, 1.0);
            let (qx, qy) = (a.0 + dx * t, a.1 + dy * t);
            let dist = (fx - qx).hypot(fy - qy);
            let cov = (r + 0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                p.layer.over(px, py, c, (alpha as f32 * cov).round() as u8);
            }
        }
    }
}

/// Text with extra space between the letters (small capitals labels).
fn spaced(p: &mut Painter, x: f32, base: f32, text: &str, cap: f32, wt: W, c: Rgb, sp: f32) -> f32 {
    let mut pen = x;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        pen += p.text(pen, base, ch.encode_utf8(&mut buf), cap, wt, c, 255) + sp;
    }
    (pen - x - sp).max(0.0)
}

fn spaced_width(p: &Painter, text: &str, cap: f32, wt: W, sp: f32) -> f32 {
    let n = text.chars().count() as f32;
    p.width(text, cap, wt) + sp * (n - 1.0).max(0.0)
}

/// Panel header: accent dot + spaced title on the left, `right` (index) on the right.
/// Returns the baseline of the header.
fn header(p: &mut Painter, r: R, k: f32, accent: Rgb, title: &str, right: &str) -> f32 {
    let pd = pad(k);
    let cap = 13.5 * k;
    let base = r.1 + pd + cap;
    circle(p, r.0 + pd + 4.5 * k, base - cap / 2.0, 4.5 * k, accent, 255);
    spaced(p, r.0 + pd + 17.0 * k, base, title, cap, W::Medium, th().muted, 2.0 * k);
    if !right.is_empty() {
        let iw = spaced_width(p, right, cap, W::Medium, 1.2 * k);
        spaced(p, r.0 + r.2 - pd - iw, base, right, cap, W::Medium, th().dim, 1.2 * k);
    }
    base
}

/// Value + small unit, left-aligned at `x`; returns the width.
fn value_unit(p: &mut Painter, x: f32, base: f32, value: &str, unit: &str, cap: f32, wt: W, vc: Rgb) -> f32 {
    let vw = p.text(x, base, value, cap, wt, vc, 255);
    if unit.is_empty() {
        return vw;
    }
    let g = (cap * 0.12).max(2.0);
    vw + g + p.text(x + vw + g, base, unit, (cap * 0.40).max(7.0), W::Regular, th().muted, 255)
}

fn value_unit_width(p: &Painter, value: &str, unit: &str, cap: f32, wt: W) -> f32 {
    let mut t = p.width(value, cap, wt);
    if !unit.is_empty() {
        t += (cap * 0.12).max(2.0) + p.width(unit, (cap * 0.40).max(7.0), W::Regular);
    }
    t
}

/// Largest cap (<= `cap`) at which value + unit fit in `max_w`.
fn fit_vu(p: &Painter, value: &str, unit: &str, cap: f32, max_w: f32, wt: W) -> f32 {
    let mut c = cap;
    for _ in 0..30 {
        if value_unit_width(p, value, unit, c, wt) <= max_w || c < 8.0 {
            break;
        }
        c *= 0.93;
    }
    c
}

/// Cuts `text` with "..." so it is at most `max_w` wide.
fn ellipsize(p: &Painter, text: &str, cap: f32, wt: W, max_w: f32) -> String {
    if p.width(text, cap, wt) <= max_w {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while s.chars().count() > 1 && p.width(&format!("{}...", s.trim_end()), cap, wt) > max_w {
        s.pop();
    }
    format!("{}...", s.trim_end())
}

/// A ring with a gradient arc, the value in the middle and a small label under it.
fn ring(p: &mut Painter, cx: f32, cy: f32, d: f32, frac: Option<f32>, colors: [Rgb; 2], value: &str, unit: &str, label: &str, k: f32) {
    let thick = (d * 0.072).max(4.0);
    let r = d / 2.0;
    p.layer.arc(cx, cy, r, thick, 1.0, &|_| WHITE, 22);
    if let Some(f) = frac {
        let f = f.clamp(0.0, 1.0);
        if f > 0.004 {
            let span = f.max(0.02);
            p.layer.arc(cx, cy, r, thick, f, &move |t| gradient_at(&colors, t / span), 255);
        }
    }
    let inner = d - 2.0 * thick - 18.0 * k;
    let cap = fit_vu(p, value, unit, d * 0.31, inner, W::Medium);
    let tw = value_unit_width(p, value, unit, cap, W::Medium);
    let lcap = (d * 0.07).max(12.0 * k);
    let block = cap + if label.is_empty() { 0.0 } else { lcap + 9.0 * k };
    let base = cy - block / 2.0 + cap;
    value_unit(p, cx - tw / 2.0, base, value, unit, cap, W::Medium, th().text);
    if !label.is_empty() {
        let lw = spaced_width(p, label, lcap, W::Medium, 1.5 * k);
        spaced(p, cx - lw / 2.0, base + 9.0 * k + lcap, label, lcap, W::Medium, th().dim, 1.5 * k);
    }
}

/// Area graph of `hist` (newest on the right), scaled to `max`.
fn sparkline(p: &mut Painter, r: R, hist: &VecDeque<f32>, max: f32, colors: [Rgb; 2], k: f32) {
    let (x, y, w, h) = r;
    // baseline + a faint mid line
    capsule(p, (x, y + h), (x + w, y + h), 0.5, WHITE, 26);
    let mut dy = y + h / 2.0;
    while dy > y {
        let mut dx = x;
        while dx < x + w {
            capsule(p, (dx, dy), ((dx + 3.0 * k).min(x + w), dy), 0.5, WHITE, 14);
            dx += 9.0 * k;
        }
        dy -= h / 2.0;
    }
    if hist.len() < 2 || max <= 0.0 {
        return;
    }
    let step = w / (HIST as f32 - 1.0);
    let x_start = x + w - (hist.len() as f32 - 1.0) * step;
    let val = |px: f32| -> f32 {
        let f = ((px - x_start) / step).clamp(0.0, hist.len() as f32 - 1.0);
        let i = (f.floor() as usize).min(hist.len() - 2);
        let t = f - i as f32;
        hist[i] * (1.0 - t) + hist[i + 1] * t
    };
    let bottom = y + h;
    // filled area: vertical gradient, fading out downwards
    let col_x0 = x_start.max(x).floor() as i32;
    let col_x1 = (x + w).ceil() as i32;
    for px in col_x0..col_x1 {
        let fx = px as f32 + 0.5;
        let v = (val(fx) / max).clamp(0.0, 1.0);
        let ty = bottom - v * h;
        let c = gradient_at(&colors, (fx - x) / w);
        let mut py = ty.floor() as i32;
        while (py as f32) < bottom {
            let fy = py as f32 + 0.5;
            let cov = (fy - ty + 0.5).clamp(0.0, 1.0);
            let fade = 1.0 - ((fy - ty) / (bottom - ty + 1.0)).clamp(0.0, 1.0);
            let a = (70.0 * fade * fade * cov).round() as u8;
            if a > 0 {
                p.layer.over(px, py, c, a);
            }
            py += 1;
        }
    }
    // the line
    let n = hist.len();
    let pts: Vec<(f32, f32)> = (0..n)
        .map(|i| (x_start + i as f32 * step, bottom - (hist[i] / max).clamp(0.0, 1.0) * h))
        .collect();
    for wnd in pts.windows(2) {
        let c = gradient_at(&colors, ((wnd[1].0 - x) / w).clamp(0.0, 1.0));
        capsule(p, wnd[0], wnd[1], 1.5 * k, c, 255);
    }
    // the newest point
    if let Some(&last) = pts.last() {
        circle(p, last.0, last.1, 3.2 * k, colors[1], 255);
    }
}

fn temp_color(t: f32, warn: f32, hot: f32) -> Rgb {
    if t >= hot {
        HOT
    } else if t >= warn {
        WARN
    } else {
        th().text
    }
}

fn rate(kb: f64) -> (String, &'static str) {
    if kb >= 1024.0 * 100.0 {
        (format!("{:.0}", kb / 1024.0), "MB/s")
    } else if kb >= 1024.0 {
        (format!("{:.1}", kb / 1024.0), "MB/s")
    } else {
        (format!("{kb:.0}"), "KB/s")
    }
}

fn gb(mb: u64) -> String {
    let g = mb as f64 / 1024.0;
    if g >= 100.0 { format!("{g:.0}") } else { format!("{g:.1}") }
}

fn mmss(s: u64) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

// ------------------------------------------------------------------- panels

/// Label (S, spaced capitals) with a value (L) + unit under it. Height: [`stat_h`].
fn stat(p: &mut Painter, sc: &Scale, x: f32, top: f32, w: f32, label: &str, value: &str, unit: &str, vc: Rgb) {
    let k = sc.k;
    spaced(p, x, top + sc.s, label, sc.s, W::Medium, th().dim, 1.6 * k);
    let cap = fit_vu(p, value, unit, sc.l, w - 4.0 * k, W::Medium);
    value_unit(p, x, top + sc.s + 7.0 * k + sc.l, value, unit, cap, W::Medium, vc);
}

fn stat_h(sc: &Scale) -> f32 {
    sc.s + 7.0 * sc.k + sc.l
}

fn draw_time(p: &mut Painter, r: R, sc: &Scale, d: &Data, now: &chrono::DateTime<Local>) {
    let k = sc.k;
    panel(p, r, k, th().time);
    let (x, y, w, h) = r;
    let pd = pad(k);
    let inner = w - 2.0 * pd;
    // weekday + date
    let wd = i18n::weekday(now).to_uppercase();
    let wb = y + pd + sc.s;
    spaced(p, x + pd, wb, &wd, sc.s, W::Medium, th().muted, 2.2 * k);
    let date = crate::tiles::nice_case(&i18n::day_month_year(now));
    let dcap = p.fit(&date, sc.m, inner, W::Regular);
    let date_base = wb + 14.0 * k + sc.m;
    p.text(x + pd, date_base, &date, dcap, W::Regular, th().text, 255);
    // the clock
    let t = clock_parts(now).0;
    let tcap = p.fit(&t, sc.xl * 1.25, inner, W::Light);
    let tw = p.width(&t, tcap, W::Light);
    let tbase = date_base + 28.0 * k + tcap;
    let tx = x + (w - tw) / 2.0;
    p.text(tx, tbase, &t, tcap, W::Light, th().text, 255);
    // seconds: a thin line under the time
    let sy = tbase + 18.0 * k;
    let frac = now.second() as f32 / 59.0;
    p.layer.fill_rrect((x + pd, sy, inner, 4.0 * k), 2.0 * k, &|_| WHITE, 24);
    if frac > 0.0 {
        let fw = (inner * frac).max(4.0 * k);
        p.layer.fill_rrect((x + pd, sy, fw, 4.0 * k), 2.0 * k, &move |t| gradient_at(&[lerp_rgb(th().time, th().bg, 0.45), th().time], t), 255);
    }
    // weather
    let sep = (sy + 24.0 * k).round();
    capsule(p, (x + pd, sep), (x + w - pd, sep), 0.5, WHITE, 30);
    let lb = sep + 18.0 * k + sc.s;
    let Some(wt) = d.weather else {
        spaced(p, x + pd, lb, "WEATHER", sc.s, W::Medium, th().dim, 1.6 * k);
        p.text(x + pd, lb + 20.0 * k + sc.l, "--", sc.l, W::Medium, th().dim, 255);
        return;
    };
    let city = wt.city.clone().unwrap_or_else(|| "WEATHER".into()).to_uppercase();
    let city = ellipsize(p, &city, sc.s, W::Medium, inner - 30.0 * k);
    spaced(p, x + pd, lb, &city, sc.s, W::Medium, th().dim, 1.6 * k);
    let isz = 52.0 * k;
    let wy = lb + 14.0 * k;
    let night = !(6..19).contains(&now.hour());
    weather_icon(p, x + pd - 3.0 * k, wy, isz, wt.icon, night);
    let temp = format!("{:.0}\u{b0}", wt.temp_in(d.fahrenheit).round() + 0.0);
    let ttx = x + pd + isz + 12.0 * k;
    p.text(ttx, wy + isz / 2.0 + sc.l / 2.0, &temp, sc.l, W::Medium, th().text, 255);
    let cond = crate::tiles::nice_case(weather_icon::condition_name(wt.icon));
    let cond = match wt.humidity {
        Some(hm) => format!("{cond} \u{b7} {hm}%"),
        None => cond,
    };
    let cond = ellipsize(p, &cond, sc.m, W::Regular, inner);
    p.text(x + pd, wy + isz + 12.0 * k + sc.m, &cond, sc.m, W::Regular, th().muted, 255);
    // the next three days, at the bottom
    let days: Vec<_> = wt.forecast.iter().skip(1).take(3).collect();
    if !days.is_empty() {
        let cw = inner / days.len() as f32;
        let tb = y + h - pd;
        let db = tb - sc.m - 12.0 * k;
        for (i, f) in days.iter().enumerate() {
            let cx = x + pd + i as f32 * cw;
            let dn = i18n::weekday_short(f.weekday).to_uppercase();
            let dw = spaced(p, cx, db, &dn, sc.s, W::Medium, th().dim, 1.2 * k);
            let is = 22.0 * k;
            weather_icon(p, cx + dw + 4.0 * k, db - sc.s / 2.0 - is / 2.0, is, f.icon, false);
            let hi = format!("{:.0}\u{b0}", f.temp_max_in(d.fahrenheit).round() + 0.0);
            let lo = format!("{:.0}\u{b0}", f.temp_min_in(d.fahrenheit).round() + 0.0);
            let hw = p.text(cx, tb, &hi, sc.m, W::Medium, th().text, 255);
            p.text(cx + hw + 3.0 * k, tb, &lo, sc.s, W::Regular, th().dim, 255);
        }
    }
}

/// Geometry shared by the processor and graphics panels: header, name, ring on the
/// left, a 4-row grid of stats on the right, a thin graph along the bottom.
struct BigRows {
    stats_x: f32,
    row_tops: [f32; 4],
}

fn big_panel(p: &mut Painter, r: R, sc: &Scale, colors: [Rgb; 2], title: &str, index: &str, name: &str, frac: Option<f32>, value: &str, unit: &str, hist: &VecDeque<f32>) -> BigRows {
    let k = sc.k;
    panel(p, r, k, colors[0]);
    let (x, y, w, h) = r;
    let pd = pad(k);
    let hb = header(p, r, k, colors[0], title, index);
    let nb = hb + 14.0 * k + sc.m;
    let name = ellipsize(p, name, sc.m, W::Regular, w - 2.0 * pd);
    p.text(x + pd, nb, &name, sc.m, W::Regular, th().text, 255);
    // graph along the bottom
    let gh = 46.0 * k;
    let gb = y + h - pd;
    sparkline(p, (x + pd, gb - gh, w - 2.0 * pd, gh), hist, 100.0, colors, k);
    // ring + rows between the name and the graph
    let top = nb + 16.0 * k;
    let bottom = gb - gh - 14.0 * k;
    let area = bottom - top;
    let rd = 196.0 * k;
    ring(p, x + pd + rd / 2.0, top + area / 2.0, rd.min(area), frac, colors, value, unit, "LOAD", k);
    let row = area / 4.0;
    let off = ((row - stat_h(sc)) / 2.0).max(0.0);
    let mut row_tops = [0.0; 4];
    for (i, t) in row_tops.iter_mut().enumerate() {
        *t = top + i as f32 * row + off;
    }
    BigRows { stats_x: x + pd + rd + 22.0 * k, row_tops }
}

/// Uptime as ("H:MM", "h") or ("Dd H", "h").
fn uptime_str(s: u64) -> (String, String) {
    let (dd, hh, mm) = (s / 86_400, (s % 86_400) / 3600, (s % 3600) / 60);
    if dd > 0 { (format!("{dd}d {hh}"), "h".into()) } else { (format!("{hh}:{mm:02}"), "h".into()) }
}

fn draw_cpu(p: &mut Painter, r: R, sc: &Scale, d: &Data, hist: &VecDeque<f32>) {
    let pct = format!("{:.0}", d.cpu_pct);
    let b = big_panel(p, r, sc, th().cpu, "PROCESSOR", "01", d.cpu_name, Some(d.cpu_pct / 100.0), &pct, "%", hist);
    let cw = r.0 + r.2 - pad(sc.k) - b.stats_x;
    let (tv, tc) = match d.cpu_temp {
        Some(t) => (format!("{t:.0}"), temp_color(t, 80.0, 90.0)),
        None => ("--".into(), th().dim),
    };
    stat(p, sc, b.stats_x, b.row_tops[0], cw, "TEMP", &tv, "\u{b0}C", tc);
    let pw = d.cpu_power.map_or("--".into(), |v| format!("{v:.0}"));
    stat(p, sc, b.stats_x, b.row_tops[1], cw, "POWER", &pw, "W", th().text);
    let clk = d.cpu_mhz.map_or("--".into(), |m| format!("{:.2}", m as f32 / 1000.0));
    stat(p, sc, b.stats_x, b.row_tops[2], cw, "CLOCK", &clk, "GHz", th().text);
    match d.cpu_fan_rpm {
        Some(rpm) => stat(p, sc, b.stats_x, b.row_tops[3], cw, "FAN", &rpm.to_string(), "RPM", th().text),
        None => {
            let (uv, uu) = uptime_str(d.uptime_s);
            stat(p, sc, b.stats_x, b.row_tops[3], cw, "UPTIME", &uv, &uu, th().text)
        }
    }
}

fn draw_gpu(p: &mut Painter, r: R, sc: &Scale, d: &Data, hist: &VecDeque<f32>) {
    let g = d.gpu;
    let name = g.name.clone().unwrap_or_else(|| "GPU".into());
    let pct = d.gpu_pct.map_or("--".into(), |v| format!("{v:.0}"));
    let unit = if d.gpu_pct.is_some() { "%" } else { "" };
    let b = big_panel(p, r, sc, th().gpu, "GRAPHICS", "02", &name, d.gpu_pct.map(|v| v / 100.0), &pct, unit, hist);
    let cw = ((r.0 + r.2 - pad(sc.k) - b.stats_x) / 2.0).floor();
    let (x0, x1) = (b.stats_x, b.stats_x + cw);
    let t = |v: Option<i32>, warn: f32, hot: f32| match v {
        Some(t) => (t.to_string(), temp_color(t as f32, warn, hot)),
        None => ("--".to_string(), th().dim),
    };
    let (tv, tc) = t(g.temp_edge_c, 80.0, 90.0);
    stat(p, sc, x0, b.row_tops[0], cw, "TEMP", &tv, "\u{b0}C", tc);
    let (hv, hc) = t(g.temp_hotspot_c, 95.0, 105.0);
    stat(p, sc, x1, b.row_tops[0], cw, "HOTSPOT", &hv, "\u{b0}C", hc);
    let pw = g.power_w.map_or("--".into(), |v| v.to_string());
    stat(p, sc, x0, b.row_tops[1], cw, "POWER", &pw, "W", th().text);
    let fan = g.fan_rpm.map_or("--".into(), |v| v.max(0).to_string());
    stat(p, sc, x1, b.row_tops[1], cw, "FAN", &fan, "RPM", th().text);
    let clk = g.clock_mhz.filter(|c| *c > 0).map_or("--".into(), |v| v.to_string());
    stat(p, sc, x0, b.row_tops[2], cw, "CLOCK", &clk, "MHz", th().text);
    let (vv, vu) = match (g.vram_used_mb, g.vram_total_mb) {
        (Some(u), Some(t)) if t > 0 => (format!("{:.1}", u as f32 / 1024.0), format!("/{:.0} GB", t as f32 / 1024.0)),
        (Some(u), _) => (format!("{:.1}", u as f32 / 1024.0), "GB".to_string()),
        _ => ("--".into(), String::new()),
    };
    stat(p, sc, x1, b.row_tops[2], cw, "VRAM", &vv, &vu, th().text);
    match g.fps.filter(|f| *f > 0) {
        Some(f) => {
            stat(p, sc, x0, b.row_tops[3], cw, "FPS", &f.to_string(), "", th().fps);
            stat(p, sc, x1, b.row_tops[3], cw, "FRAME TIME", &format!("{:.1}", 1000.0 / f as f32), "ms", th().fps);
        }
        None => {
            stat(p, sc, x0, b.row_tops[3], cw, "FPS", "--", "", th().dim);
            stat(p, sc, x1, b.row_tops[3], cw, "FRAME TIME", "--", "", th().dim);
        }
    }
}

/// Small card: header, a ring on the right, a value (L) and two lines (M) on the left.
fn card(p: &mut Painter, r: R, sc: &Scale, colors: [Rgb; 2], title: &str, index: &str, frac: Option<f32>, ring_txt: &str, value: (&str, &str, Rgb), line1: &str, line2: &str) {
    let k = sc.k;
    panel(p, r, k, colors[0]);
    let (x, y, w, h) = r;
    let pd = pad(k);
    let hb = header(p, r, k, colors[0], title, index);
    let body_top = hb + 12.0 * k;
    let body_h = y + h - pd - body_top;
    let rd = body_h.min(w * 0.36).max(30.0);
    let cx = x + w - pd - rd / 2.0;
    let cy = body_top + body_h / 2.0;
    ring(p, cx, cy, rd, frac, colors, ring_txt, if frac.is_some() { "%" } else { "" }, "", k);
    let lw = cx - rd / 2.0 - 12.0 * k - (x + pd);
    let vcap = fit_vu(p, value.0, value.1, sc.l, lw, W::Medium);
    let l2cap = if line2.is_empty() { 0.0 } else { p.fit(line2, sc.m, lw, W::Regular) };
    let block = sc.l + 14.0 * k + sc.m + if line2.is_empty() { 0.0 } else { 10.0 * k + sc.m };
    let vb = cy - block / 2.0 + sc.l;
    value_unit(p, x + pd, vb, value.0, value.1, vcap, W::Medium, value.2);
    let s1 = ellipsize(p, line1, sc.m, W::Regular, lw);
    p.text(x + pd, vb + 14.0 * k + sc.m, &s1, sc.m, W::Regular, th().muted, 255);
    if !line2.is_empty() {
        let s2 = ellipsize(p, line2, l2cap, W::Regular, lw);
        p.text(x + pd, vb + 14.0 * k + 2.0 * sc.m + 10.0 * k, &s2, l2cap, W::Regular, th().dim, 255);
    }
}

fn draw_ram(p: &mut Painter, r: R, sc: &Scale, d: &Data) {
    let frac = if d.ram_total_mb > 0 { d.ram_used_mb as f32 / d.ram_total_mb as f32 } else { 0.0 };
    let pct = format!("{:.0}", frac * 100.0);
    let used = gb(d.ram_used_mb);
    let total = format!("of {} GB", gb(d.ram_total_mb));
    let free = format!("{} GB free", gb(d.ram_total_mb.saturating_sub(d.ram_used_mb)));
    card(p, r, sc, th().ram, "MEMORY", "03", Some(frac), &pct, (used.as_str(), "GB", th().text), &total, &free);
}

fn draw_disk(p: &mut Painter, r: R, sc: &Scale, d: &Data) {
    let (rd, wr) = d.disk_rate;
    let io = format!("R {:.1}  W {:.1} MB/s", rd, wr);
    match d.disk {
        Some(di) => {
            let pct = format!("{:.0}", di.used_frac() * 100.0);
            let (tv, tc) = match di.temp_c {
                Some(t) => (t.to_string(), temp_color(t as f32, 60.0, 70.0)),
                None => ("--".to_string(), th().dim),
            };
            let space = format!("{:.0}/{:.0} GB", di.used_gb, di.total_gb);
            card(p, r, sc, th().ssd, "STORAGE", "04", Some(di.used_frac()), &pct, (tv.as_str(), "\u{b0}C", tc), &space, &io);
        }
        None => card(p, r, sc, th().ssd, "STORAGE", "04", None, "--", ("--", "", th().dim), &io, ""),
    }
}

fn draw_media(p: &mut Painter, r: R, sc: &Scale, d: &Data, cover: Option<&Cover>) {
    let k = sc.k;
    let accent = cover.map_or(th().muted, |c| c.accent);
    panel(p, r, k, accent);
    let (x, y, w, h) = r;
    let pd = pad(k);
    let hb = header(p, r, k, accent, "NOW PLAYING", "05");
    let Some(t) = d.track else {
        let cx = x + w / 2.0;
        let cy = hb + (y + h - hb) / 2.0 - 10.0 * k;
        music_note(p, cx, cy - 8.0 * k, 34.0 * k, th().dim);
        let s = "Nothing playing";
        let sw = p.width(s, sc.m, W::Regular);
        p.text(cx - sw / 2.0, cy + 40.0 * k, s, sc.m, W::Regular, th().dim, 255);
        return;
    };
    let body_top = hb + 16.0 * k;
    let bar_zone = 44.0 * k;
    let cs = (y + h - pd - bar_zone - body_top).min(w * 0.36).max(24.0).floor();
    let (cx0, cy0) = (x + pd, body_top);
    match cover {
        Some(c) => blit_cover(p, c, cx0, cy0, cs, 12.0 * k),
        None => {
            p.layer.fill_rrect((cx0, cy0, cs, cs), 12.0 * k, &|t| lerp_rgb((40, 42, 52), (24, 25, 32), t), 255);
            music_note(p, cx0 + cs / 2.0, cy0 + cs / 2.0, cs * 0.32, th().muted);
        }
    }
    // title (one line, scrolls when too long) + artist + album
    let tx = cx0 + cs + 14.0 * k;
    let tw = x + w - pd - tx;
    let title = clean_title(&t.title);
    let tcap = sc.m * 1.12;
    let base = cy0 + tcap + 4.0 * k;
    title_line(p, tx, base, tw, &title, tcap, W::Medium, th().text);
    let artist = if t.artist.trim().is_empty() { "Unknown artist".to_string() } else { t.artist.clone() };
    let artist = ellipsize(p, &artist, sc.m, W::Regular, tw);
    p.text(tx, base + 14.0 * k + sc.m, &artist, sc.m, W::Regular, th().muted, 255);
    if !t.album.trim().is_empty() && t.album.trim() != t.title.trim() {
        let album = ellipsize(p, &t.album, sc.s, W::Regular, tw);
        p.text(tx, base + 14.0 * k + sc.m + 12.0 * k + sc.s, &album, sc.s, W::Regular, th().dim, 255);
    }
    // progress
    let bh = 5.0 * k;
    let by = y + h - pd - bh;
    let bw = w - 2.0 * pd;
    if let Some(tl) = t.timeline.filter(|tl| tl.duration_ms > 0) {
        let pos = tl.position_now_ms();
        let f = (pos as f32 / tl.duration_ms as f32).clamp(0.0, 1.0);
        p.layer.fill_rrect((x + pd, by, bw, bh), bh / 2.0, &|_| WHITE, 28);
        if f > 0.0 {
            let fw = (bw * f).max(bh);
            let (a0, a1) = (accent, lerp_rgb(accent, (255, 255, 255), 0.45));
            p.layer.fill_rrect((x + pd, by, fw, bh), bh / 2.0, &move |t| lerp_rgb(a0, a1, t), 255);
            circle(p, x + pd + fw, by + bh / 2.0, 6.5 * k, WHITE, 255);
        }
        let l = mmss(pos / 1000);
        let rr = mmss(tl.duration_ms / 1000);
        p.text(x + pd, by - 12.0 * k, &l, sc.s, W::Medium, th().muted, 255);
        let rw = p.width(&rr, sc.s, W::Medium);
        p.text(x + w - pd - rw, by - 12.0 * k, &rr, sc.s, W::Medium, th().dim, 255);
    } else {
        p.layer.fill_rrect((x + pd, by, bw, bh), bh / 2.0, &|_| WHITE, 20);
    }
}

fn draw_net(p: &mut Painter, r: R, sc: &Scale, d: &Data, hist: &VecDeque<f32>) {
    let k = sc.k;
    panel(p, r, k, th().net[0]);
    let (x, y, w, h) = r;
    let pd = pad(k);
    let hb = header(p, r, k, th().net[0], "NETWORK", "06");
    let top = hb + 10.0 * k;
    let bottom = y + h - pd;
    let row = (bottom - top) / 2.0;
    let vw = (w - 2.0 * pd) * 0.60;
    for (i, (kb, c, down)) in [(d.net_kb.0, th().net[1], true), (d.net_kb.1, th().up, false)].into_iter().enumerate() {
        let base = top + i as f32 * row + (row + sc.l) / 2.0;
        arrow(p, x + pd + 9.0 * k, base - sc.l / 2.0, 9.0 * k, down, c);
        let (v, u) = rate(kb);
        let vc = fit_vu(p, &v, u, sc.l, vw - 28.0 * k, W::Medium);
        value_unit(p, x + pd + 28.0 * k, base, &v, u, vc, W::Medium, th().text);
    }
    let peak = hist.iter().cloned().fold(64.0f32, f32::max) * 1.15;
    let gx = x + pd + vw + 8.0 * k;
    let gw = x + w - pd - gx;
    if gw > 30.0 * k {
        sparkline(p, (gx, top + 14.0 * k, gw, bottom - top - 14.0 * k), hist, peak, th().net, k);
    }
}

// -------------------------------------------------------------------- icons

fn arrow(p: &mut Painter, cx: f32, cy: f32, s: f32, down: bool, c: Rgb) {
    let t = 1.3 * s / 7.0 * 1.0;
    let sign = if down { 1.0 } else { -1.0 };
    let tip = (cx, cy + sign * s);
    capsule(p, (cx, cy - sign * s), tip, t, c, 255);
    capsule(p, (cx - s * 0.62, cy + sign * s * 0.38), tip, t, c, 255);
    capsule(p, (cx + s * 0.62, cy + sign * s * 0.38), tip, t, c, 255);
}

fn music_note(p: &mut Painter, cx: f32, cy: f32, s: f32, c: Rgb) {
    let r = s * 0.22;
    circle(p, cx - s * 0.28, cy + s * 0.32, r, c, 255);
    circle(p, cx + s * 0.32, cy + s * 0.2, r, c, 255);
    let t = s * 0.055;
    capsule(p, (cx - s * 0.28 + r - t, cy + s * 0.32), (cx - s * 0.28 + r - t, cy - s * 0.42), t, c, 255);
    capsule(p, (cx + s * 0.32 + r - t, cy + s * 0.2), (cx + s * 0.32 + r - t, cy - s * 0.54), t, c, 255);
    capsule(p, (cx - s * 0.28 + r - t, cy - s * 0.42), (cx + s * 0.32 + r - t, cy - s * 0.54), t * 1.6, c, 255);
}

fn cloud(p: &mut Painter, x: f32, y: f32, s: f32, c: Rgb) {
    // x, y = top-left of an s*s box; the cloud sits in its lower part
    circle(p, x + s * 0.36, y + s * 0.50, s * 0.20, c, 255);
    circle(p, x + s * 0.58, y + s * 0.44, s * 0.24, c, 255);
    p.layer.fill_rrect((x + s * 0.12, y + s * 0.50, s * 0.78, s * 0.24), s * 0.12, &move |_| c, 255);
}

fn sun(p: &mut Painter, cx: f32, cy: f32, r: f32, c: Rgb) {
    circle(p, cx, cy, r, c, 255);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let (s, co) = a.sin_cos();
        capsule(p, (cx + co * r * 1.45, cy + s * r * 1.45), (cx + co * r * 1.85, cy + s * r * 1.85), r * 0.13, c, 255);
    }
}

fn moon(p: &mut Painter, cx: f32, cy: f32, r: f32, c: Rgb) {
    circle(p, cx, cy, r, c, 255);
    circle(p, cx + r * 0.45, cy - r * 0.35, r * 0.85, under(), 255);
}

fn weather_icon(p: &mut Painter, x: f32, y: f32, s: f32, icon: WeatherIcon, night: bool) {
    let sun_c = (253, 200, 72);
    let moon_c = (226, 232, 240);
    let cloud_c = (214, 220, 230);
    let dark_cloud = (150, 158, 172);
    let rain_c = (96, 165, 250);
    match icon {
        WeatherIcon::Clear => {
            if night {
                moon(p, x + s * 0.5, y + s * 0.5, s * 0.30, moon_c)
            } else {
                sun(p, x + s * 0.5, y + s * 0.5, s * 0.22, sun_c)
            }
        }
        WeatherIcon::PartlyCloudy => {
            if night {
                moon(p, x + s * 0.66, y + s * 0.30, s * 0.19, moon_c);
            } else {
                sun(p, x + s * 0.66, y + s * 0.30, s * 0.15, sun_c);
            }
            cloud(p, x - s * 0.08, y + s * 0.20, s * 0.82, cloud_c);
        }
        WeatherIcon::Cloudy => {
            cloud(p, x - s * 0.06, y - s * 0.12, s * 0.8, dark_cloud);
            cloud(p, x + s * 0.08, y + s * 0.06, s * 0.92, cloud_c);
        }
        WeatherIcon::Fog => {
            for (i, wf) in [0.7f32, 0.85, 0.6].iter().enumerate() {
                let yy = y + s * (0.32 + i as f32 * 0.18);
                capsule(p, (x + s * 0.12, yy), (x + s * (0.12 + wf * 0.76), yy), s * 0.045, cloud_c, 255);
            }
        }
        WeatherIcon::Drizzle | WeatherIcon::Rain => {
            cloud(p, x, y - s * 0.12, s, cloud_c);
            let n = if icon == WeatherIcon::Rain { 3 } else { 2 };
            for i in 0..n {
                let xx = x + s * (0.32 + i as f32 * 0.18);
                capsule(p, (xx, y + s * 0.70), (xx - s * 0.06, y + s * 0.90), s * 0.035, rain_c, 255);
            }
        }
        WeatherIcon::Snow => {
            cloud(p, x, y - s * 0.12, s, cloud_c);
            for i in 0..3 {
                let xx = x + s * (0.32 + i as f32 * 0.18);
                circle(p, xx, y + s * (0.76 + (i % 2) as f32 * 0.1), s * 0.045, WHITE, 255);
            }
        }
        WeatherIcon::Thunder => {
            cloud(p, x, y - s * 0.12, s, dark_cloud);
            let c = (250, 204, 21);
            let t = s * 0.04;
            capsule(p, (x + s * 0.54, y + s * 0.62), (x + s * 0.44, y + s * 0.80), t, c, 255);
            capsule(p, (x + s * 0.44, y + s * 0.80), (x + s * 0.56, y + s * 0.80), t, c, 255);
            capsule(p, (x + s * 0.56, y + s * 0.80), (x + s * 0.46, y + s * 0.98), t, c, 255);
        }
    }
}

// -------------------------------------------------------------------- cover

/// Box-filter resize of a square-ish RGB image to `n` x `n`.
fn scale_square(rgb: &[u8], w: u32, h: u32, n: u32) -> Vec<u8> {
    let mut out = vec![0u8; (n * n * 3) as usize];
    if w == 0 || h == 0 || rgb.len() < (w * h * 3) as usize {
        return out;
    }
    for oy in 0..n {
        let y0 = oy * h / n;
        let y1 = ((oy + 1) * h / n).max(y0 + 1).min(h);
        for ox in 0..n {
            let x0 = ox * w / n;
            let x1 = ((ox + 1) * w / n).max(x0 + 1).min(w);
            let (mut r, mut g, mut b, mut c) = (0u32, 0u32, 0u32, 0u32);
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let i = ((yy * w + xx) * 3) as usize;
                    r += rgb[i] as u32;
                    g += rgb[i + 1] as u32;
                    b += rgb[i + 2] as u32;
                    c += 1;
                }
            }
            let o = ((oy * n + ox) * 3) as usize;
            out[o] = (r / c.max(1)) as u8;
            out[o + 1] = (g / c.max(1)) as u8;
            out[o + 2] = (b / c.max(1)) as u8;
        }
    }
    out
}

/// A vivid color taken from the cover (weighted by saturation), brightened enough
/// to read on the dark panels.
fn cover_accent(rgb: &[u8]) -> Rgb {
    let (mut r, mut g, mut b, mut wsum) = (0f32, 0f32, 0f32, 0f32);
    for px in rgb.chunks_exact(3).step_by(3) {
        let (pr, pg, pb) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let mx = pr.max(pg).max(pb);
        let mn = pr.min(pg).min(pb);
        let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        let wgt = sat * sat * (mx / 255.0) + 0.002;
        r += pr * wgt;
        g += pg * wgt;
        b += pb * wgt;
        wsum += wgt;
    }
    if wsum <= 0.0 {
        return (200, 204, 214);
    }
    let (mut r, mut g, mut b) = (r / wsum, g / wsum, b / wsum);
    let mx = r.max(g).max(b).max(1.0);
    if mx < 200.0 {
        let f = 200.0 / mx;
        r = (r * f).min(255.0);
        g = (g * f).min(255.0);
        b = (b * f).min(255.0);
    }
    (r as u8, g as u8, b as u8)
}

/// Three distinct, bright-enough colors of a cover: the strongest hues, or shades of
/// the accent when the cover has only one color.
fn cover_palette(rgb: &[u8], accent: Rgb) -> [Rgb; 3] {
    const BINS: usize = 18;
    let mut acc = [(0f32, 0f32, 0f32, 0f32); BINS];
    for px in rgb.chunks_exact(3).step_by(2) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        if mx < 40.0 || mx - mn < 30.0 {
            continue;
        }
        let d = mx - mn;
        let h = if mx == r { ((g - b) / d).rem_euclid(6.0) } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
        let bin = ((h / 6.0 * BINS as f32) as usize).min(BINS - 1);
        let wgt = (d / mx) * (mx / 255.0);
        let e = &mut acc[bin];
        e.0 += r * wgt;
        e.1 += g * wgt;
        e.2 += b * wgt;
        e.3 += wgt;
    }
    let total: f32 = acc.iter().map(|e| e.3).sum();
    let mut order: Vec<usize> = (0..BINS).filter(|&i| acc[i].3 > total * 0.04).collect();
    order.sort_by(|a, b| acc[*b].3.total_cmp(&acc[*a].3));
    let mut picked: Vec<usize> = Vec::new();
    for i in order {
        let far = picked.iter().all(|&j| {
            let dd = (i as i32 - j as i32).rem_euclid(BINS as i32);
            dd.min(BINS as i32 - dd) >= 2
        });
        if far {
            picked.push(i);
        }
        if picked.len() == 3 {
            break;
        }
    }
    let bright = |c: (f32, f32, f32)| -> Rgb {
        let mx = c.0.max(c.1).max(c.2).max(1.0);
        let f = (225.0 / mx).max(1.0);
        ((c.0 * f).min(255.0) as u8, (c.1 * f).min(255.0) as u8, (c.2 * f).min(255.0) as u8)
    };
    let mut out: Vec<Rgb> = picked.iter().map(|&i| { let e = acc[i]; bright((e.0 / e.3, e.1 / e.3, e.2 / e.3)) }).collect();
    let base = out.first().copied().unwrap_or(accent);
    while out.len() < 3 {
        let n = out.len();
        out.push(if n == 1 { lerp_rgb(base, WHITE, 0.35) } else { lerp_rgb(base, (120, 120, 140), 0.35) });
    }
    [out[0], out[1], out[2]]
}

fn blit_cover(p: &mut Painter, c: &Cover, x: f32, y: f32, size: f32, radius: f32) {
    let n = size.round() as i32;
    if n <= 0 || c.size == 0 {
        return;
    }
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    // pixel-exact: the picture is resized to the drawn size once, then copied 1:1
    let px = c.at(n as u32);
    let nu = n as usize;
    for py in 0..n {
        for pxi in 0..n {
            let cov = rrect_cov(pxi as f32 + 0.5, py as f32 + 0.5, 0.0, 0.0, n as f32, n as f32, radius);
            if cov <= 0.0 {
                continue;
            }
            let i = (py as usize * nu + pxi as usize) * 3;
            if i + 2 >= px.len() {
                continue;
            }
            p.layer.over(xi + pxi, yi + py, (px[i], px[i + 1], px[i + 2]), (255.0 * cov).round() as u8);
        }
    }
    p.layer.stroke_rrect((xi as f32, yi as f32, n as f32, n as f32), radius, 1.0, WHITE, 26);
}

/// Splits `text` into at most two lines that fit `max_w` (the second one cut with "...").
fn wrap2(p: &Painter, text: &str, cap: f32, wt: W, max_w: f32) -> Vec<String> {
    if p.width(text, cap, wt) <= max_w {
        return vec![text.to_string()];
    }
    let mut first = String::new();
    let mut rest_start = 0;
    for (i, word) in text.split_inclusive(' ').enumerate() {
        let cand = format!("{first}{word}");
        if p.width(cand.trim_end(), cap, wt) > max_w {
            if i == 0 {
                // one very long word: cut it
                return vec![ellipsize(p, text, cap, wt, max_w)];
            }
            break;
        }
        first = cand;
        rest_start += word.len();
    }
    let rest = text[rest_start..].trim();
    if rest.is_empty() {
        return vec![first.trim_end().to_string()];
    }
    vec![first.trim_end().to_string(), ellipsize(p, rest, cap, wt, max_w)]
}

// ------------------------------------------------------------- scrolling title
//
// A title too long for its box is not painted on the cached layer: its position is
// recorded here and `ProScreen::draw` scrolls it directly on the frame (hold 3 s,
// scroll, hold, ...), with soft fades at the edges. Only the scrolling phase asks
// for a higher frame rate, so a short title costs nothing extra.

#[derive(Clone, PartialEq)]
struct MarqueeSpec {
    x: f32,
    w: f32,
    base: f32,
    cap: f32,
    weight: W,
    color: Rgb,
    text: String,
}

thread_local! {
    static MARQ: std::cell::RefCell<Vec<MarqueeSpec>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Vertical screens do not scroll titles (the scroll runs on the unturned frame): they are cut with "...".
    static NO_MARQ: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

const MARQ_HOLD: f32 = 3.0;
const MARQ_SPEED: f32 = 42.0; // px/s at k = 1

fn sfw(w: W) -> crate::smooth_font::Weight {
    match w {
        W::Light => crate::smooth_font::Weight::Light,
        W::Regular => crate::smooth_font::Weight::Regular,
        W::Medium => crate::smooth_font::Weight::Medium,
    }
}

/// One line of text in a box `w` wide: drawn as-is when it fits, scrolled otherwise.
fn title_line(p: &mut Painter, x: f32, base: f32, w: f32, text: &str, cap: f32, wt: W, color: Rgb) {
    if p.width(text, cap, wt) <= w {
        p.text(x, base, text, cap, wt, color, 255);
    } else if NO_MARQ.with(|c| c.get()) {
        let t = ellipsize(p, text, cap, wt, w);
        p.text(x, base, &t, cap, wt, color, 255);
    } else {
        MARQ.with(|m| m.borrow_mut().push(MarqueeSpec { x, w, base, cap, weight: wt, color, text: text.to_string() }));
    }
}

/// (offset in px, scrolling now) for a text `tw` wide, `t` seconds after it appeared.
fn marquee_offset(tw: f32, gap: f32, speed: f32, t: f32) -> (f32, bool) {
    let run = (tw + gap) / speed;
    let phase = t % (MARQ_HOLD + run);
    if phase < MARQ_HOLD { (0.0, false) } else { ((phase - MARQ_HOLD) * speed, true) }
}

/// Draws a scrolling title straight onto the frame (`ox`, `oy` = where the layer sits).
fn paint_marquee(fb: &mut Framebuffer, ox: u32, oy: u32, m: &MarqueeSpec, k: f32, t: f32) {
    let px = crate::smooth_font::px_for_cap(m.cap);
    let wt = sfw(m.weight);
    let tw = crate::smooth_font::text_width(&m.text, px, wt);
    let gap = 70.0 * k;
    let (off, _) = marquee_offset(tw, gap, MARQ_SPEED * k, t);
    let fade = 22.0 * k;
    let (x0, x1) = (m.x, m.x + m.w);
    let fw = fb.width() as i32;
    let fh = fb.height() as i32;
    let (cr, cg, cb) = (m.color.0 as u32, m.color.1 as u32, m.color.2 as u32);
    let bytes = fb.as_bytes_mut();
    for copy in 0..2 {
        let start = m.x - off + copy as f32 * (tw + gap);
        if start > x1 || start + tw < x0 {
            continue;
        }
        crate::smooth_font::draw_with(&m.text, start, m.base, px, wt, |gx, gy, cov| {
            let fx = gx as f32 + 0.5;
            if fx < x0 || fx >= x1 {
                return;
            }
            let mut a = cov as f32 / 255.0;
            if off > 0.0 {
                a *= ((fx - x0) / fade).clamp(0.0, 1.0);
            }
            a *= ((x1 - fx) / fade).clamp(0.0, 1.0);
            if a <= 0.0 {
                return;
            }
            let (fx2, fy2) = (gx + ox as i32, gy + oy as i32);
            if fx2 < 0 || fy2 < 0 || fx2 >= fw || fy2 >= fh {
                return;
            }
            let i = ((fy2 * fw + fx2) * 3) as usize;
            let a8 = (a * 255.0) as u32;
            let inv = 255 - a8;
            bytes[i] = ((cr * a8 + bytes[i] as u32 * inv) / 255) as u8;
            bytes[i + 1] = ((cg * a8 + bytes[i + 1] as u32 * inv) / 255) as u8;
            bytes[i + 2] = ((cb * a8 + bytes[i + 2] as u32 * inv) / 255) as u8;
        });
    }
}

/// "Song (Official Music Video) [4K]" -> "Song".
fn clean_title(t: &str) -> String {
    const JUNK: &[&str] = &[
        "official", "video", "audio", "lyric", "mv", "m/v", "4k", "hd", "hq", "visualizer", "remaster", "explicit", "clip", "music",
    ];
    let mut out = String::new();
    let mut depth = 0;
    let mut chunk = String::new();
    for c in t.chars() {
        match c {
            '(' | '[' | '【' => {
                if depth == 0 {
                    chunk.clear();
                }
                depth += 1;
                chunk.push(c);
            }
            ')' | ']' | '】' if depth > 0 => {
                chunk.push(c);
                depth -= 1;
                if depth == 0 {
                    let lc = chunk.to_lowercase();
                    if !JUNK.iter().any(|j| lc.contains(j)) {
                        out.push_str(&chunk);
                    }
                    chunk.clear();
                }
            }
            _ if depth > 0 => chunk.push(c),
            _ => out.push(c),
        }
    }
    out.push_str(&chunk);
    let mut s = out.split_whitespace().collect::<Vec<_>>().join(" ");
    for tail in [" - Topic", " | Official", " |"] {
        if let Some(i) = s.find(tail) {
            if i > 0 {
                s.truncate(i);
            }
        }
    }
    let s = s.trim_matches(|c: char| c == '-' || c == '|' || c.is_whitespace()).to_string();
    if s.is_empty() { t.trim().to_string() } else { s }
}

// ------------------------------------------------------------------ shapes

/// Arc from `start` degrees (0 = 12 o'clock, clockwise) over `sweep` degrees, round ends.
/// `color(t)`: t = 0..1 along the drawn arc.
fn arc2(p: &mut Painter, cx: f32, cy: f32, r_out: f32, thick: f32, start: f32, sweep: f32, color: &dyn Fn(f32) -> Rgb, alpha: u8) {
    if sweep <= 0.1 || alpha == 0 {
        return;
    }
    let r_in = r_out - thick;
    let r_mid = r_out - thick / 2.0;
    let cap = thick / 2.0;
    let rad = |deg: f32| deg.to_radians();
    let end = start + sweep;
    let (sx, sy) = (cx + r_mid * rad(start).sin(), cy - r_mid * rad(start).cos());
    let (ex, ey) = (cx + r_mid * rad(end).sin(), cy - r_mid * rad(end).cos());
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
            let mut ang = dx.atan2(-dy).to_degrees();
            if ang < 0.0 {
                ang += 360.0;
            }
            let mut rel = ang - start;
            let centre = sweep / 2.0;
            while rel <= centre - 180.0 {
                rel += 360.0;
            }
            while rel > centre + 180.0 {
                rel -= 360.0;
            }
            let radial = (r_out + 0.5 - d).clamp(0.0, 1.0) * (d - r_in + 0.5).clamp(0.0, 1.0);
            let arc_px = d * std::f32::consts::PI / 180.0;
            let inside = ((rel * arc_px) + 0.5).clamp(0.0, 1.0) * (((sweep - rel) * arc_px) + 0.5).clamp(0.0, 1.0);
            let mut cov = radial * inside;
            for (qx, qy) in [(sx, sy), (ex, ey)] {
                cov = cov.max((cap + 0.5 - (fx - qx).hypot(fy - qy)).clamp(0.0, 1.0));
            }
            if cov > 0.0 {
                let t = (rel / sweep).clamp(0.0, 1.0);
                p.layer.over(px, py, color(t), (alpha as f32 * cov).round() as u8);
            }
        }
    }
}

/// Thin horizontal bar: track + gradient fill.
fn hbar(p: &mut Painter, x: f32, y: f32, w: f32, h: f32, frac: Option<f32>, colors: [Rgb; 2]) {
    p.layer.fill_rrect((x, y, w, h), h / 2.0, &|_| WHITE, 22);
    if let Some(f) = frac {
        let f = f.clamp(0.0, 1.0);
        if f > 0.0 {
            let fw = (w * f).max(h);
            p.layer.fill_rrect((x, y, fw, h), h / 2.0, &move |t| gradient_at(&colors, t * f.max(0.05)), 255);
        }
    }
}

/// Cover + title (scrolls) + artist + progress, in the box (x, y, w, h).
fn now_playing_compact(p: &mut Painter, r: R, sc: &Scale, d: &Data, cover: Option<&Cover>, show_idle: bool) {
    let k = sc.k;
    let (x, y, w, h) = r;
    let accent = cover.map_or(th().muted, |c| c.accent);
    let Some(t) = d.track else {
        if show_idle {
            music_note(p, x + h * 0.45, y + h / 2.0, h * 0.32, th().dim);
            p.text(x + h + 6.0 * k, y + h / 2.0 + sc.m / 2.0, "Nothing playing", sc.m, W::Regular, th().dim, 255);
        }
        return;
    };
    let cs = h;
    match cover {
        Some(c) => blit_cover(p, c, x, y, cs, 10.0 * k),
        None => {
            p.layer.fill_rrect((x, y, cs, cs), 10.0 * k, &|t| lerp_rgb((40, 42, 52), (24, 25, 32), t), 255);
            music_note(p, x + cs / 2.0, y + cs / 2.0, cs * 0.34, th().muted);
        }
    }
    let tx = x + cs + 14.0 * k;
    let tw = x + w - tx;
    let title = clean_title(&t.title);
    title_line(p, tx, y + sc.m + 2.0 * k, tw, &title, sc.m, W::Medium, th().text);
    let artist = ellipsize(p, &t.artist, sc.s, W::Regular, tw);
    p.text(tx, y + sc.m + 2.0 * k + 10.0 * k + sc.s, &artist, sc.s, W::Regular, th().muted, 255);
    if let Some(tl) = t.timeline.filter(|tl| tl.duration_ms > 0) {
        let f = (tl.position_now_ms() as f32 / tl.duration_ms as f32).clamp(0.0, 1.0);
        let by = y + h - 4.0 * k;
        hbar(p, tx, by, tw, 4.0 * k, Some(f), [accent, lerp_rgb(accent, WHITE, 0.45)]);
        // elapsed / total, above the bar (only where the box is tall enough)
        if h >= 80.0 * k {
            let (el, tot) = (mmss(tl.position_now_ms().min(tl.duration_ms) / 1000), mmss(tl.duration_ms / 1000));
            let tsz = sc.s * 1.05;
            let ty = by - 12.0 * k;
            let (tw1, tw2) = (p.width(&el, tsz, W::Medium), p.width(&format!(" / {tot}"), tsz, W::Regular));
            let x0 = tx + tw - tw1 - tw2;
            p.text(x0, ty, &el, tsz, W::Medium, th().text, 255);
            p.text(x0 + tw1, ty, &format!(" / {tot}"), tsz, W::Regular, th().muted, 255);
        }
    }
}

fn clock_parts(now: &chrono::DateTime<Local>) -> (String, String, String) {
    let twelve = CLOCK_12H.load(std::sync::atomic::Ordering::Relaxed);
    let t = now.format(if twelve { "%I:%M" } else { "%H:%M" }).to_string();
    let mut wd = i18n::weekday(now).to_uppercase();
    if twelve {
        wd = format!("{wd} {}", if now.hour() < 12 { "AM" } else { "PM" });
    }
    let date = crate::tiles::nice_case(&i18n::day_month_year(now));
    (t, wd, date)
}

fn weather_text(d: &Data) -> Option<(WeatherIcon, String, String)> {
    let wt = d.weather?;
    let temp = format!("{:.0}\u{b0}", wt.temp_in(d.fahrenheit).round() + 0.0);
    let cond = crate::tiles::nice_case(weather_icon::condition_name(wt.icon));
    let city = wt.city.clone().unwrap_or_default();
    let line = if city.is_empty() { cond } else { format!("{cond} \u{b7} {city}") };
    Some((wt.icon, temp, line))
}

fn vram_text(g: &GpuAmdData) -> (String, String, Option<f32>) {
    match (g.vram_used_mb, g.vram_total_mb) {
        (Some(u), Some(t)) if t > 0 => (format!("{:.1}", u as f32 / 1024.0), format!("/{:.0} GB", t as f32 / 1024.0), Some(u as f32 / t as f32)),
        (Some(u), _) => (format!("{:.1}", u as f32 / 1024.0), "GB".into(), None),
        _ => ("--".into(), String::new(), None),
    }
}

fn tval(v: Option<f32>, warn: f32, hot: f32) -> (String, Rgb) {
    match v {
        Some(t) => (format!("{t:.0}"), temp_color(t, warn, hot)),
        None => ("--".into(), th().dim),
    }
}

// --------------------------------------------------------------- COCKPIT
//
// Car instrument cluster: two 240° gauges (CPU left, GPU right) with ticks, the clock
// and the rest of the readings in the middle, now playing in a pill at the bottom.

fn gauge(p: &mut Painter, cx: f32, cy: f32, r: f32, sc: &Scale, colors: [Rgb; 2], label: &str, frac: Option<f32>, value: &str, name: &str, stats: [(&str, String, &str, Rgb); 3]) {
    let k = sc.k;
    let thick = 15.0 * k;
    let (start, sweep) = (240.0f32, 240.0f32);
    // ticks
    let rt = r - thick - 12.0 * k;
    for i in 0..=48 {
        let a = (start + sweep * i as f32 / 48.0).to_radians();
        let major = i % 12 == 0;
        let len = if major { 12.0 * k } else { 6.0 * k };
        let (s, c) = a.sin_cos();
        capsule(p, (cx + s * rt, cy - c * rt), (cx + s * (rt - len), cy - c * (rt - len)), if major { 1.3 * k } else { 0.8 * k }, th().text, if major { 150 } else { 60 });
    }
    arc2(p, cx, cy, r, thick, start, sweep, &|_| WHITE, 22);
    if let Some(f) = frac.map(|f| f.clamp(0.0, 1.0)) {
        if f > 0.003 {
            arc2(p, cx, cy, r, thick, start, sweep * f, &move |t| gradient_at(&colors, t), 255);
            let a = (start + sweep * f).to_radians();
            let rm = r - thick / 2.0;
            let (ex, ey) = (cx + a.sin() * rm, cy - a.cos() * rm);
            circle(p, ex, ey, thick * 1.05, colors[1], 70);
            circle(p, ex, ey, thick * 0.42, WHITE, 255);
        }
    }
    // label + value + name in the middle
    let lw = spaced_width(p, label, sc.s, W::Medium, 2.0 * k);
    spaced(p, cx - lw / 2.0, cy - 58.0 * k, label, sc.s, W::Medium, colors[1], 2.0 * k);
    let vcap = 78.0 * k;
    let vw = value_unit_width(p, value, if frac.is_some() { "%" } else { "" }, vcap, W::Light);
    value_unit(p, cx - vw / 2.0, cy + vcap / 2.0 - 6.0 * k, value, if frac.is_some() { "%" } else { "" }, vcap, W::Light, th().text);
    let name = ellipsize(p, name, sc.s, W::Regular, r * 1.3);
    let nw = p.width(&name, sc.s, W::Regular);
    p.text(cx - nw / 2.0, cy + vcap / 2.0 + 24.0 * k, &name, sc.s, W::Regular, th().dim, 255);
    // three stats in the opening at the bottom
    let open_w = 2.0 * r * (60.0f32).to_radians().sin() + 40.0 * k;
    let cw = open_w / 3.0;
    let top = cy + r * 0.5 + 4.0 * k;
    for (i, (l, v, u, c)) in stats.iter().enumerate() {
        let x = cx - open_w / 2.0 + i as f32 * cw;
        let lw = spaced_width(p, l, sc.s * 0.9, W::Medium, 1.4 * k);
        spaced(p, x + (cw - lw) / 2.0, top + sc.s, l, sc.s * 0.9, W::Medium, th().dim, 1.4 * k);
        let cap = fit_vu(p, v, u, 28.0 * k, cw - 6.0 * k, W::Medium);
        let vw = value_unit_width(p, v, u, cap, W::Medium);
        value_unit(p, x + (cw - vw) / 2.0, top + sc.s + 8.0 * k + 28.0 * k, v, u, cap, W::Medium, *c);
    }
}

fn paint_cockpit(p: &mut Painter, w: f32, h: f32, sc: &Scale, d: &Data, now: &chrono::DateTime<Local>, cover: Option<&Cover>) {
    let k = sc.k;
    let r = 186.0 * k;
    let cy = 22.0 * k + r;
    let lcx = 34.0 * k + r;
    let rcx = w - 34.0 * k - r;
    if style() != Style::Flat {
        for cx in [lcx, rcx] {
            circle(p, cx, cy, r + 14.0 * k, th().panel, 255);
        }
    }
    let (tv, tc) = tval(d.cpu_temp, 80.0, 90.0);
    let pw = d.cpu_power.map_or("--".into(), |v| format!("{v:.0}"));
    let clk = d.cpu_mhz.map_or("--".into(), |m| format!("{:.2}", m as f32 / 1000.0));
    gauge(p, lcx, cy, r, sc, th().cpu, "CPU", Some(d.cpu_pct / 100.0), &format!("{:.0}", d.cpu_pct), d.cpu_name,
        [("TEMP", tv, "\u{b0}C", tc), ("POWER", pw, "W", th().text), ("CLOCK", clk, "GHz", th().text)]);
    let g = d.gpu;
    let (gt, gtc) = tval(g.temp_edge_c.map(|v| v as f32), 80.0, 90.0);
    let (ht, htc) = tval(g.temp_hotspot_c.map(|v| v as f32), 95.0, 105.0);
    let gp = g.power_w.map_or("--".into(), |v| v.to_string());
    let name = g.name.clone().unwrap_or_else(|| "GPU".into());
    gauge(p, rcx, cy, r, sc, th().gpu, "GPU", d.gpu_pct.map(|v| v / 100.0), &d.gpu_pct.map_or("--".into(), |v| format!("{v:.0}")), &name,
        [("TEMP", gt, "\u{b0}C", gtc), ("HOTSPOT", ht, "\u{b0}C", htc), ("POWER", gp, "W", th().text)]);

    // centre column
    let x0 = lcx + r + 46.0 * k;
    let x1 = rcx - r - 46.0 * k;
    let cw = x1 - x0;
    let mid = (x0 + x1) / 2.0;
    let (t, wd, date) = clock_parts(now);
    let tcap = 104.0 * k;
    let tw = p.width(&t, tcap, W::Light);
    p.text(mid - tw / 2.0, 22.0 * k + tcap, &t, tcap, W::Light, th().text, 255);
    let line = match weather_text(d) {
        Some((_, temp, cond)) => format!("{wd}  \u{b7}  {date}  \u{b7}  {temp} {cond}"),
        None => format!("{wd}  \u{b7}  {date}"),
    };
    let line = ellipsize(p, &line, sc.s, W::Medium, cw);
    let lw = p.width(&line, sc.s, W::Medium);
    p.text(mid - lw / 2.0, 22.0 * k + tcap + 30.0 * k, &line, sc.s, W::Medium, th().muted, 255);

    // chips row
    let (vv, vu, vf) = vram_text(g);
    let ram_f = if d.ram_total_mb > 0 { Some(d.ram_used_mb as f32 / d.ram_total_mb as f32) } else { None };
    let (dn, du) = rate(d.net_kb.0);
    let fps = g.fps.filter(|f| *f > 0);
    let chips: [(&str, String, String, Option<f32>, [Rgb; 2], Rgb); 6] = [
        ("RAM", gb(d.ram_used_mb), "GB".into(), ram_f, th().ram, th().text),
        ("VRAM", vv, vu, vf, th().gpu, th().text),
        ("SSD", d.disk.and_then(|x| x.temp_c).map_or("--".into(), |t| t.to_string()), "\u{b0}C".into(), d.disk.map(|x| x.used_frac()), th().ssd, th().text),
        ("GPU FAN", g.fan_rpm.map_or("--".into(), |v| v.max(0).to_string()), "RPM".into(), None, th().gpu, th().text),
        ("DOWN", dn, du.into(), None, th().net, th().text),
        ("FPS", fps.map_or("--".into(), |f| f.to_string()), fps.map_or(String::new(), |f| format!("{:.1} ms", 1000.0 / f as f32)), None, th().gpu, if fps.is_some() { th().fps } else { th().dim }),
    ];
    let n = chips.len() as f32;
    let chw = cw / n;
    let top = 214.0 * k;
    for (i, (l, v, u, f, cols, vc)) in chips.iter().enumerate() {
        let x = x0 + i as f32 * chw + 8.0 * k;
        let iw = chw - 16.0 * k;
        if i > 0 {
            capsule(p, (x - 8.0 * k, top + 4.0 * k), (x - 8.0 * k, top + 66.0 * k), 0.5, WHITE, 34);
        }
        spaced(p, x, top + sc.s, l, sc.s, W::Medium, th().dim, 1.4 * k);
        let cap = fit_vu(p, v, u, 30.0 * k, iw, W::Medium);
        value_unit(p, x, top + sc.s + 10.0 * k + 30.0 * k, v, u, cap, W::Medium, *vc);
        if f.is_some() {
            hbar(p, x, top + sc.s + 10.0 * k + 30.0 * k + 12.0 * k, iw, 4.0 * k, *f, *cols);
        }
    }

    // now playing pill
    let ph = 84.0 * k;
    let py = h - 14.0 * k - ph;
    let pr = (x0, py, cw, ph);
    if style() == Style::Flat {
        capsule(p, (x0, py - 10.0 * k), (x1, py - 10.0 * k), 0.5, WHITE, 34);
    } else {
        p.layer.fill_rrect(pr, ph / 2.0, &|_| th().panel, 255);
        p.layer.stroke_rrect(pr, ph / 2.0, 1.0, WHITE, 20);
    }
    let inner = (x0 + 14.0 * k, py + 12.0 * k, cw - 40.0 * k, ph - 24.0 * k);
    now_playing_compact(p, inner, sc, d, cover, true);
}

// ------------------------------------------------------------------ FOCUS
//
// A few huge numbers: the temperatures of CPU and GPU, memory use and the clock,
// each with a short row of details and a soft load graph underneath.

fn focus_section(p: &mut Painter, r: R, sc: &Scale, vcap: f32, wide_last: bool, colors: [Rgb; 2], label: &str, hero: &str, hero_unit: &str, hero_color: Rgb, side: &str, stats: &[(&str, String, String, Rgb)], hist: Option<(&VecDeque<f32>, f32)>) {
    let k = sc.k;
    let (x, y, w, h) = r;
    circle(p, x + 5.0 * k, y + sc.s * 0.5, 5.0 * k, colors[0], 255);
    spaced(p, x + 18.0 * k, y + sc.s, label, sc.s, W::Medium, th().muted, 2.0 * k);
    let hcap = 108.0 * k;
    let hb = y + sc.s + 18.0 * k + hcap;
    let cap = fit_vu(p, hero, hero_unit, hcap, w * 0.6 - 14.0 * k, W::Light);
    // a small history line in the free corner next to the big number
    if let Some((hq, max)) = hist {
        sparkline(p, (x + w * 0.6, y + sc.s + 40.0 * k, w * 0.4, 40.0 * k), hq, max, colors, k);
    }
    let hw = p.text(x - 6.0 * k, hb, hero, cap, W::Light, hero_color, 255);
    if !hero_unit.is_empty() {
        p.text(x - 6.0 * k + hw + 4.0 * k, hb - cap + sc.l, hero_unit, sc.l, W::Light, th().muted, 255);
    }
    if !side.is_empty() {
        let sw = p.width(side, sc.m, W::Regular);
        p.text(x + w - sw, hb, side, sc.m, W::Regular, th().muted, 255);
    }
    // stats: two per row, three rows spread evenly to the bottom, all one size
    let _ = wide_last;
    let cw = w / 2.0;
    let top = hb + 18.0 * k;
    let row_h = sc.s + 8.0 * k + vcap;
    let pitch = ((y + h) - top - row_h) / 2.0;
    for (i, (l, v, u, c)) in stats.iter().enumerate() {
        let sx = x + (i % 2) as f32 * cw;
        let sy = top + (i / 2) as f32 * pitch;
        spaced(p, sx, sy + sc.s, l, sc.s, W::Medium, th().dim, 1.5 * k);
        // a "A / B" pair is drawn a little smaller so both numbers fit
        let cap = fit_vu(p, v, u, if l.contains(" / ") { vcap * 0.78 } else { vcap }, cw - 10.0 * k, W::Medium);
        value_unit(p, sx, sy + row_h, v, u, cap, W::Medium, *c);
    }
}

fn paint_focus(p: &mut Painter, w: f32, h: f32, sc: &Scale, d: &Data, now: &chrono::DateTime<Local>, hist: (&VecDeque<f32>, &VecDeque<f32>, &VecDeque<f32>), cover: Option<&Cover>) {
    let k = sc.k;
    let m = 24.0 * k;
    let gap = 36.0 * k;
    let weights = [386.0f32, 452.0, 482.0, 452.0];
    let avail = w - 2.0 * m - gap * 3.0;
    let sum: f32 = weights.iter().sum();
    let mut xs = Vec::new();
    let mut x = m;
    for wt in weights {
        let cw = avail * wt / sum;
        xs.push((x, cw));
        x += cw + gap;
    }
    for (x, _) in xs.iter().skip(1) {
        capsule(p, (x - gap / 2.0, m), (x - gap / 2.0, h - m), 0.5, WHITE, 34);
    }
    let r = |i: usize| (xs[i].0, m, xs[i].1, h - 2.0 * m);
    // one value size for every stat of the three sections (the widest values must fit)
    let mut vcap = 60.0 * k;
    for (i, tpl) in [(1, [("4.35", "GHz"), ("1480", "RPM"), ("23:59", "h")]), (2, [("2491", "MHz"), ("1480", "RPM"), ("16.0", "GB")]), (3, [("47.9", "GB"), ("12.4", "MB/s"), ("850", "KB/s")])] {
        for (v, u) in tpl {
            vcap = vcap.min(fit_vu(p, v, u, 60.0 * k, xs[i].1 / 2.0 - 10.0 * k, W::Medium));
        }
    }
    // CPU
    let (tv, tc) = tval(d.cpu_temp, 80.0, 90.0);
    let (uv, uu) = uptime_str(d.uptime_s);
    let mut cpu_stats = vec![
        ("LOAD", format!("{:.0}", d.cpu_pct), "%".to_string(), th().text),
        ("POWER", d.cpu_power.map_or("--".into(), |v| format!("{v:.0}")), "W".into(), th().text),
        ("CLOCK", d.cpu_mhz.map_or("--".into(), |m| format!("{:.2}", m as f32 / 1000.0)), "GHz".into(), th().text),
        ("UPTIME", uv, uu, th().text),
        match d.cpu_fan_rpm {
            Some(rpm) => ("FAN", rpm.to_string(), "RPM".into(), th().text),
            None => ("FAN", "--".into(), String::new(), th().dim),
        },
    ];
    let (dn, up) = d.net_kb;
    let (nv, nu) = if dn.max(up) >= 1024.0 {
        let f = |v: f64| if v >= 100.0 * 1024.0 { format!("{:.0}", v / 1024.0) } else { format!("{:.1}", v / 1024.0) };
        (format!("{}/{}", f(dn), f(up)), "MB/s")
    } else {
        (format!("{dn:.0}/{up:.0}"), "KB/s")
    };
    cpu_stats.push(("DOWN / UP", nv, nu.to_string(), th().text));
    focus_section(p, r(1), sc, vcap, false, th().cpu, "CPU TEMPERATURE", &tv, "\u{b0}C", tc, "", &cpu_stats, Some((hist.0, 100.0)));
    // GPU: FPS has its own cell, it never replaces another value
    let g = d.gpu;
    let (gt, gtc) = tval(g.temp_edge_c.map(|v| v as f32), 80.0, 90.0);
    let fps = g.fps.filter(|f| *f > 0);
    let side = g.temp_hotspot_c.map_or(String::new(), |t| format!("hotspot {t}\u{b0}"));
    let gpu_stats = [
        ("LOAD", d.gpu_pct.map_or("--".into(), |v| format!("{v:.0}")), "%".to_string(), th().text),
        ("POWER", g.power_w.map_or("--".into(), |v| v.to_string()), "W".into(), th().text),
        ("FAN", g.fan_rpm.map_or("--".into(), |v| v.max(0).to_string()), "RPM".into(), th().text),
        ("CLOCK", g.clock_mhz.filter(|c| *c > 0).map_or("--".into(), |v| v.to_string()), "MHz".into(), th().text),
        ("VRAM", vram_text(g).0, "GB".into(), th().text),
        match fps {
            Some(f) => ("FPS", f.to_string(), format!("{:.1}ms", 1000.0 / f as f32), th().fps),
            None => ("FPS", "--".to_string(), String::new(), th().dim),
        },
    ];
    focus_section(p, r(2), sc, vcap, false, th().gpu, "GPU TEMPERATURE", &gt, "\u{b0}C", gtc, &side, &gpu_stats, Some((hist.1, 100.0)));
    // memory + storage + network
    let ram_pct = if d.ram_total_mb > 0 { d.ram_used_mb as f32 * 100.0 / d.ram_total_mb as f32 } else { 0.0 };
    let (rv, ru) = rate(d.disk_rate.0 * 1024.0);
    let (wv, wu) = rate(d.disk_rate.1 * 1024.0);
    let mem_stats = [
        ("USED", gb(d.ram_used_mb), "GB".to_string(), th().text),
        ("FREE", gb(d.ram_total_mb.saturating_sub(d.ram_used_mb)), "GB".into(), th().text),
        ("SSD", d.disk.and_then(|x| x.temp_c).map_or("--".into(), |t| t.to_string()), "\u{b0}C".into(), d.disk.and_then(|x| x.temp_c).map_or(th().dim, |t| temp_color(t as f32, 60.0, 70.0))),
        ("DISK", d.disk.map_or("--".into(), |x| format!("{:.0}", x.used_frac() * 100.0)), "%".into(), th().text),
        ("READ", rv, ru.to_string(), th().text),
        ("WRITE", wv, wu.to_string(), th().text),
    ];
    let side = format!("of {} GB", gb(d.ram_total_mb));
    focus_section(p, r(3), sc, vcap, false, th().ram, "MEMORY", &format!("{ram_pct:.0}"), "%", th().text, &side, &mem_stats, Some((hist.2, hist.2.iter().cloned().fold(256.0f32, f32::max) * 1.1)));
    // time + weather + now playing
    let (x, y, sw, sh) = r(0);
    let (t, wd, date) = clock_parts(now);
    spaced(p, x, y + sc.s, &format!("{wd}  \u{b7}  {}", date.to_uppercase()), sc.s, W::Medium, th().muted, 2.0 * k);
    let hcap = 108.0 * k;
    let cap = p.fit(&t, hcap, sw, W::Light);
    let hb = y + sc.s + 18.0 * k + hcap;
    p.text(x - 6.0 * k, hb, &t, cap, W::Light, th().text, 255);
    if let Some((icon, temp, line)) = weather_text(d) {
        let wy = hb + 20.0 * k;
        weather_icon(p, x - 3.0 * k, wy, 46.0 * k, icon, !(6..19).contains(&now.hour()));
        let base = wy + 23.0 * k + sc.l / 2.0;
        let tw = p.text(x + 52.0 * k, base, &temp, sc.l, W::Medium, th().text, 255);
        let line = ellipsize(p, &line, sc.m, W::Regular, sw - 66.0 * k - tw);
        p.text(x + 64.0 * k + tw, base, &line, sc.m, W::Regular, th().muted, 255);
    }
    // the next three days: day + icon, then high / low
    if let Some(wt) = d.weather {
        let days: Vec<_> = wt.forecast.iter().skip(1).take(3).collect();
        let fy = hb + 110.0 * k;
        let cw = sw / 3.0;
        for (i, f) in days.iter().enumerate() {
            let cx = x + i as f32 * cw;
            let dn = i18n::weekday_short(f.weekday).to_uppercase();
            let dw = spaced(p, cx, fy, &dn, sc.s, W::Medium, th().dim, 1.4 * k);
            weather_icon(p, cx + dw + 8.0 * k, fy - sc.s / 2.0 - 15.0 * k, 30.0 * k, f.icon, false);
            let hi = format!("{:.0}\u{b0}", f.temp_max_in(d.fahrenheit).round() + 0.0);
            let lo = format!("{:.0}\u{b0}", f.temp_min_in(d.fahrenheit).round() + 0.0);
            let ty = fy + 14.0 * k + sc.l * 0.8;
            let hw = p.text(cx, ty, &hi, sc.l * 0.8, W::Medium, th().text, 255);
            p.text(cx + hw + 8.0 * k, ty, &lo, sc.m * 1.15, W::Regular, th().dim, 255);
        }
    }
    let big = Scale { m: sc.m * 1.3, s: sc.s * 1.25, ..*sc };
    now_playing_compact(p, (x, y + sh - 90.0 * k, sw, 90.0 * k), &big, d, cover, true);
}

#[cfg(test)]
mod aperture_tests {
    #[test]
    fn empty_optional_game_overrides_are_disabled() {
        let file = super::ConfigFile::parse("pro_game_layout = \npro_game_theme = \npro_night_theme = \n").unwrap();
        let settings = super::parse(&file).unwrap();
        assert!(settings.game_layout.is_none());
        assert!(settings.game_theme.is_none());
        assert!(settings.night_theme.is_none());
    }
    use super::*;
    #[test]
    fn collection_rotation_and_legacy_keys_are_compatible() {
        let s=parse(&ConfigFile::parse("pro_layout = all\npro_theme = all\n").unwrap()).unwrap();
        assert_eq!(s.layouts, Layout::CURATED.to_vec());
        assert_eq!(s.themes.len(),APERTURE_THEMES.len());
        for key in ["focus","grid","cockpit","stack","blueprint"] {
            let text=format!("pro_layout = {key}\npro_theme = neon\n");
            assert!(parse(&ConfigFile::parse(&text).unwrap()).is_ok());
        }
    }
    #[test]
    fn dark_custom_accents_are_legible_and_coordinated() {
        let t=Theme::from_accent((0,0,0));
        assert!(t.cpu[0].0>=120);
        assert_ne!(t.cpu[0],t.gpu[0]);
    }
}
