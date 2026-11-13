//! `aperture` — the settings app of the dashboard (English / Tiếng Việt), by HuyBruh.
//!
//! It edits `trofeo.conf` next to it; the running `aperture-display` re-reads the file every
//! second, so every change shows on the display right away. Pictures (the live preview and
//! the layout thumbnails) are drawn by `aperture-display.exe --preview-png`, so they always
//! match the display. The Sensors page reads `trofeo-status.txt`, which the display program
//! keeps up to date, and shows live readings and where each one comes from.
#![cfg_attr(windows, windows_subsystem = "windows")]

use aperture::conf_edit::ConfDoc;
use aperture::sensors_ext::{self, Snapshot, Source};
use eframe::egui::{self, Align, Color32, FontFamily, FontId, Layout as ELayout, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "../custom_editor.rs"]
mod custom_editor;

// ------------------------------------------------------------------------ look

const BG: Color32 = Color32::from_rgb(13, 14, 17);
const SIDEBAR: Color32 = Color32::from_rgb(18, 19, 23);
const CARD: Color32 = Color32::from_rgb(23, 25, 30);
const CARD_HI: Color32 = Color32::from_rgb(30, 32, 39);
const FIELD: Color32 = Color32::from_rgb(15, 16, 20);
const LINE: Color32 = Color32::from_rgb(40, 43, 52);
const TEXT: Color32 = Color32::from_rgb(238, 239, 243);
const MUTED: Color32 = Color32::from_rgb(156, 160, 172);
const DIM: Color32 = Color32::from_rgb(104, 109, 121);
const ACCENT: Color32 = Color32::from_rgb(134, 114, 255);
const ACCENT_SOFT: Color32 = Color32::from_rgb(42, 37, 72);
const GOOD: Color32 = Color32::from_rgb(52, 199, 123);
const WARN: Color32 = Color32::from_rgb(255, 176, 32);
const BAD: Color32 = Color32::from_rgb(255, 92, 92);

const R: f32 = 14.0;
const GAP: f32 = 16.0;

fn font(size: f32, w: Weight) -> FontId {
    FontId::new(size, FontFamily::Name(w.name().into()))
}

#[derive(Clone, Copy)]
enum Weight {
    Regular,
    Medium,
    Semibold,
}

impl Weight {
    fn name(self) -> &'static str {
        match self {
            Weight::Regular => "inter",
            Weight::Medium => "inter-medium",
            Weight::Semibold => "inter-semibold",
        }
    }
}

fn setup_style(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let add = |fonts: &mut egui::FontDefinitions, name: &str, data: &'static [u8]| {
        fonts.font_data.insert(name.to_string(), egui::FontData::from_static(data));
        let mut fam = vec![name.to_string()];
        fam.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
        fonts.families.insert(FontFamily::Name(name.into()), fam);
    };
    add(&mut fonts, "inter", include_bytes!("../../assets/Inter-Regular-subset.ttf"));
    add(&mut fonts, "inter-medium", include_bytes!("../../assets/Inter-Medium-subset.ttf"));
    add(&mut fonts, "inter-semibold", include_bytes!("../../assets/Inter-SemiBold-subset.ttf"));
    fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "inter".into());
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::new(22.0, FontFamily::Name("inter-semibold".into()))),
        (egui::TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (egui::TextStyle::Button, FontId::new(14.0, FontFamily::Name("inter-medium".into()))),
        (egui::TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (egui::TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(14.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.slider_width = 240.0;
    style.spacing.combo_width = 240.0;
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD_HI;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = CARD_HI;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = Rounding::same(9.0);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.hovered.bg_fill = Color32::from_rgb(38, 41, 50);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(38, 41, 50);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(78, 82, 98));
    v.widgets.active.bg_fill = Color32::from_rgb(46, 50, 61);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(46, 50, 61);
    v.window_rounding = Rounding::same(R);
    v.menu_rounding = Rounding::same(10.0);
    v.window_stroke = Stroke::new(1.0_f32, LINE);
    ctx.set_style(style);
}

// ------------------------------------------------------------------------ language

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Vi,
}

// ------------------------------------------------------------------------ catalog

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    New,
    Aperture,
    Signature,
    Classic,
    Portrait,
}

#[derive(Clone, Copy)]
struct LayoutInfo {
    key: &'static str,
    name: &'static str,
    en: &'static str,
    vi: &'static str,
    group: Group,
}

impl LayoutInfo {
    fn portrait(&self) -> bool {
        self.group == Group::Portrait
    }
}

const fn li(key: &'static str, name: &'static str, en: &'static str, vi: &'static str, group: Group) -> LayoutInfo {
    LayoutInfo { key, name, en, vi, group }
}

/// Every layout of the display program, none hidden.
const LAYOUTS: &[LayoutInfo] = &[
    li("board", "Board", "Airport departure board: every number on split-flap tiles", "Bảng giờ bay sân bay: mọi con số trên ô lật", Group::New),
    li("studio", "Studio", "Paper and ink: a light spec sheet, black numbers, one orange accent", "Giấy và mực: nền sáng như bảng thông số, số đen, nhấn cam", Group::New),
    li("pulse", "Pulse", "A live chart of the last two minutes for every section", "Biểu đồ trực tiếp 2 phút gần nhất cho từng mục", Group::New),
    li("cluster", "Cluster", "Car instrument cluster: three dials with a real scale", "Cụm đồng hồ xe hơi: ba mặt đồng hồ có vạch chia", Group::New),
    li("panorama", "Panorama", "Three large instruments and a compact footer", "Ba bảng lớn và thanh ngữ cảnh gọn", Group::Aperture),
    li("cinema", "Cinema", "Large album artwork and a dense hardware console", "Ảnh bìa lớn, bảng thông số gọn", Group::Aperture),
    li("axis", "Axis", "A quiet header, three clear instruments", "Thanh ngữ cảnh, ba bảng thông số rõ ràng", Group::Aperture),
    li("ledger", "Ledger", "Three precise rows, every reading in place", "Ba hàng thẳng, mỗi thông số một vị trí", Group::Aperture),
    li("duet", "Duet", "CPU and GPU side by side, a memory rail", "CPU và GPU song song, bộ nhớ thành cột riêng", Group::Aperture),
    li("orbit", "Orbit", "Gauge arcs, a central clock, balanced wings", "Vòng cung, đồng hồ giữa, hai cánh cân đối", Group::Aperture),
    li("atelier", "Atelier", "Album-led, with a full hardware desk", "Ảnh bìa lớn cùng bàn thông số đầy đủ", Group::Aperture),
    li("showcase", "Showcase", "Game poster backdrop, transparent logo and telemetry", "Poster game toàn màn hình, logo và thông số", Group::Aperture),
    li("focus", "Focus", "Huge numbers, nothing else", "Số cực lớn, tối giản", Group::Signature),
    li("grid", "Grid", "Even tiles like phone widgets", "Ô đều nhau như widget điện thoại", Group::Signature),
    li("stack", "Stack", "Three full-width rows, six readings each", "Ba hàng ngang, mỗi hàng sáu thông số", Group::Signature),
    li("mosaic", "Mosaic", "Eight tiles, one big number each", "Tám ô, mỗi ô một con số lớn", Group::Signature),
    li("mixer", "Mixer", "Level faders like a mixing console", "Cần gạt mức như bàn mixer", Group::Signature),
    li("chart", "Chart", "Temperature chart and a full table", "Biểu đồ nhiệt độ và bảng thông số", Group::Signature),
    li("player", "Player", "Big album cover, music first", "Ảnh bìa lớn, ưu tiên nhạc", Group::Signature),
    li("tint", "Tint", "Tiles tinted with each section's color", "Ô nhuộm màu theo từng mục", Group::Signature),
    li("glass", "Glass", "Frosted tiles over a soft color wallpaper", "Ô kính mờ trên nền màu dịu", Group::Signature),
    li("gauge", "Gauge", "A big arc gauge for CPU, GPU and memory", "Đồng hồ vòng cung lớn cho CPU, GPU, RAM", Group::Signature),
    li("swiss", "Swiss", "Swiss typographic grid", "Lưới chữ phong cách Thụy Sĩ", Group::Signature),
    li("classic", "Classic", "Panels in a row", "Các ô xếp ngang", Group::Classic),
    li("cockpit", "Cockpit", "Two gauges, the clock in the middle", "Hai đồng hồ đo, giờ ở giữa", Group::Classic),
    li("bento", "Bento", "Colored tiles", "Ô màu kiểu bento", Group::Classic),
    li("terminal", "Terminal", "Green console", "Màn hình console xanh", Group::Classic),
    li("chamfer", "Chamfer", "Angular game HUD", "Khung vát góc kiểu game", Group::Classic),
    li("rings", "Rings", "Activity rings", "Vòng tròn hoạt động", Group::Classic),
    li("dots", "Dots", "LED dot matrix", "Chữ ma trận chấm LED", Group::Classic),
    li("lock", "Lock", "Phone lock screen", "Màn hình khóa điện thoại", Group::Portrait),
    li("equalizer", "Equalizer", "LED level meters", "Cột đèn LED", Group::Portrait),
    li("receipt", "Receipt", "Paper receipt", "Hóa đơn giấy", Group::Portrait),
    li("synthwave", "Synthwave", "Retro sunset", "Hoàng hôn retro", Group::Portrait),
    li("blueprint", "Blueprint", "Technical drawing", "Bản vẽ kỹ thuật", Group::Portrait),
];

#[derive(Clone, Copy)]
struct ThemeInfo {
    key: &'static str,
    name: &'static str,
    /// Background, then three accents.
    bg: (u8, u8, u8),
    colors: [(u8, u8, u8); 3],
    soft: bool,
}

const fn ti(key: &'static str, name: &'static str, bg: (u8, u8, u8), colors: [(u8, u8, u8); 3], soft: bool) -> ThemeInfo {
    ThemeInfo { key, name, bg, colors, soft }
}

/// Every palette of the display program: the soft ones and the vivid ones.
const THEMES: &[ThemeInfo] = &[
    ti("mineral", "Mineral", (16, 18, 19), [(190, 217, 197), (168, 176, 178), (25, 28, 30)], true),
    ti("frost", "Frost", (16, 20, 25), [(169, 204, 232), (168, 185, 200), (25, 32, 39)], true),
    ti("blush", "Blush", (23, 19, 22), [(228, 170, 197), (192, 171, 181), (35, 29, 33)], true),
    ti("sand", "Sand", (23, 21, 18), [(223, 189, 135), (192, 181, 164), (36, 32, 26)], true),
    ti("iris", "Iris", (20, 19, 26), [(196, 179, 232), (180, 173, 200), (32, 30, 41)], true),
    ti("lagoon", "Lagoon", (12, 24, 27), [(130, 230, 203), (174, 208, 202), (20, 39, 43)], true),
    ti("nzxt", "NZXT", (0, 0, 0), [(124, 92, 255), (34, 211, 238), (244, 114, 182)], false),
    ti("graphite", "Graphite", (0, 0, 0), [(150, 150, 158), (210, 210, 214), (255, 255, 255)], false),
    ti("crimson", "Crimson", (0, 0, 0), [(220, 20, 50), (255, 99, 99), (251, 146, 60)], false),
    ti("arctic", "Arctic", (0, 0, 0), [(56, 189, 248), (45, 212, 191), (186, 230, 253)], false),
    ti("ember", "Ember", (0, 0, 0), [(251, 146, 60), (250, 204, 21), (254, 205, 140)], false),
    ti("neon", "Neon", (0, 0, 0), [(255, 0, 153), (0, 229, 255), (250, 255, 0)], false),
];

// ------------------------------------------------------------------------ rendering

fn find_main_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "aperture-display.exe" } else { "aperture-display" };
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let p = dir.join(name);
    p.is_file().then_some(p)
}

/// Draws `text` (a configuration) with `layout` forced, through the main program.
fn render(exe: &Path, workdir: &Path, text: &str, layout: Option<&str>, music: bool, tag: &str) -> Option<egui::ColorImage> {
    let tmp = std::env::temp_dir();
    let id = std::process::id();
    let conf = tmp.join(format!("aperture_{id}_{tag}.conf"));
    let png = tmp.join(format!("aperture_{id}_{tag}.png"));
    let mut doc = ConfDoc::parse(text);
    doc.set("layout", "pro");
    doc.set("flip", "false");
    if let Some(l) = layout {
        doc.set("pro_layout", l);
        doc.set("pro_custom_enabled", "false");
        doc.set("pro_edit_layout", "");
        doc.set("background", "");
    }
    // a single theme in the pictures (not the whole rotation)
    if let Some(t) = doc.get("pro_theme") {
        let first = t.split(',').next().unwrap_or("mineral").trim().to_string();
        doc.set("pro_theme", if first == "all" { "mineral" } else { first.as_str() });
    }
    let first_layout = doc.get("pro_layout").unwrap_or_default();
    let first_layout = first_layout.split(',').next().unwrap_or("").trim().to_string();
    match first_layout.as_str() {
        "all" => doc.set("pro_layout", "board"),
        "portrait" => doc.set("pro_layout", "lock"),
        "" => {}
        l => doc.set("pro_layout", l),
    }
    std::fs::write(&conf, doc.to_text()).ok()?;
    let _ = std::fs::remove_file(&png);
    let shown = layout.map(str::to_string).unwrap_or_else(|| first_layout.clone());
    let preview_state = if !music {
        "idle"
    } else if shown == "showcase" {
        "game"
    } else {
        "music"
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--config").arg(&conf).arg("--preview-png").arg(&png).arg("--preview-state").arg(preview_state);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if workdir.is_dir() {
        cmd.current_dir(workdir);
    }
    hidden(&mut cmd);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    let _ = std::fs::remove_file(&conf);
    if !ok {
        return None;
    }
    let bytes = std::fs::read(&png).ok()?;
    let _ = std::fs::remove_file(&png);
    let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    // portrait pictures come out turned for the panel: show them upright
    if LAYOUTS.iter().any(|x| x.key == shown && x.portrait()) {
        img = image::imageops::rotate270(&img);
    }
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

enum Job {
    Preview { text: String, music: bool, rev: u64 },
    Thumbs { text: String, rev: u64, first: Vec<&'static str> },
}

enum Done {
    Preview(u64, Option<egui::ColorImage>),
    Thumb(u64, String, Option<egui::ColorImage>),
}

fn spawn_renderer(exe: PathBuf, workdir: PathBuf, ctx: egui::Context) -> (mpsc::Sender<Job>, mpsc::Receiver<Done>) {
    let (tx, rx_jobs) = mpsc::channel::<Job>();
    let (tx_done, rx) = mpsc::channel::<Done>();
    std::thread::spawn(move || {
        let mut preview: Option<(String, bool, u64)> = None;
        // (config, revision, keys still to draw)
        let mut thumbs: Option<(String, u64, Vec<&'static str>)> = None;
        loop {
            let take = |j: Job, preview: &mut Option<(String, bool, u64)>, thumbs: &mut Option<(String, u64, Vec<&'static str>)>| match j {
                Job::Preview { text, music, rev } => *preview = Some((text, music, rev)),
                Job::Thumbs { text, rev, first } => {
                    // the visible group first, then the rest
                    let mut keys = first.clone();
                    keys.extend(LAYOUTS.iter().map(|l| l.key).filter(|k| !first.contains(k)));
                    *thumbs = Some((text, rev, keys));
                }
            };
            if preview.is_none() && thumbs.is_none() {
                match rx_jobs.recv() {
                    Ok(j) => take(j, &mut preview, &mut thumbs),
                    Err(_) => return,
                }
            }
            while let Ok(j) = rx_jobs.try_recv() {
                take(j, &mut preview, &mut thumbs);
            }
            if let Some((text, music, rev)) = preview.take() {
                let img = render(&exe, &workdir, &text, None, music, "p");
                let _ = tx_done.send(Done::Preview(rev, img));
                ctx.request_repaint();
                continue;
            }
            // one thumbnail at a time, so a new preview request is served quickly
            if let Some((text, rev, mut keys)) = thumbs.take() {
                if !keys.is_empty() {
                    let key = keys.remove(0);
                    let img = render(&exe, &workdir, &text, Some(key), true, "t");
                    let _ = tx_done.send(Done::Thumb(rev, key.to_string(), img));
                    ctx.request_repaint();
                    if !keys.is_empty() {
                        thumbs = Some((text, rev, keys));
                    }
                }
            }
        }
    });
    (tx, rx)
}

// ------------------------------------------------------------------------ status

/// What the display program reports in `trofeo-status.txt`, plus a few checks of our own.
#[derive(Default, Clone)]
struct Status {
    map: HashMap<String, String>,
    /// The display program wrote its file in the last 30 s and is running.
    alive: bool,
    pawnio: Option<bool>,
    ext: Option<(Source, Snapshot)>,
}

impl Status {
    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    /// Fan headers of the motherboard: (selector, rpm).
    fn board_fans(&self) -> Vec<(String, u32)> {
        self.get("board_fans")
            .map(|s| s.split('|').filter_map(|f| f.rsplit_once(':')).filter_map(|(n, r)| Some((n.to_string(), r.parse().ok()?))).collect())
            .unwrap_or_default()
    }
}

fn read_status(dir: &Path) -> Status {
    let mut s = Status::default();
    if let Ok(t) = std::fs::read_to_string(dir.join("trofeo-status.txt")) {
        for line in t.lines() {
            if let Some((k, v)) = line.split_once('=') {
                s.map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        let updated: u64 = s.get("updated").and_then(|v| v.parse().ok()).unwrap_or(0);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        s.alive = now.saturating_sub(updated) < 30;
    }
    s.alive = s.alive && process_running();
    s.pawnio = pawnio_installed();
    s.ext = sensors_ext::read_any();
    s
}

#[cfg(windows)]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

#[cfg(not(windows))]
fn hidden(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd
}

#[cfg(windows)]
fn process_running() -> bool {
    let out = hidden(std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq aperture-display.exe", "/NH"])).output();
    out.map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("aperture-display.exe")).unwrap_or(false)
}

#[cfg(not(windows))]
fn process_running() -> bool {
    true
}

#[cfg(windows)]
fn pawnio_installed() -> Option<bool> {
    let out = hidden(std::process::Command::new("sc").args(["query", "PawnIO"])).output().ok()?;
    Some(out.status.success())
}

#[cfg(not(windows))]
fn pawnio_installed() -> Option<bool> {
    None
}

/// Base64 (for PowerShell's -EncodedCommand, which takes UTF-16LE text).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Runs a PowerShell script as Administrator (one UAC prompt). The script travels encoded,
/// so its quotes and `$` signs reach the elevated PowerShell untouched.
fn run_elevated(script: &str) {
    #[cfg(windows)]
    {
        let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let arg = format!(
            "Start-Process powershell -Verb RunAs -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-EncodedCommand','{}'",
            base64(&utf16)
        );
        let _ = hidden(std::process::Command::new("powershell").args(["-NoProfile", "-Command", &arg])).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = script;
    }
}

/// Finds the autostart task of the display program (named TrofeoLCD or ApertureDisplay
/// depending on the version that installed it), stops it and the program, starts it again.
const RESTART_SCRIPT: &str = "$t = Get-ScheduledTask -ErrorAction SilentlyContinue | Where-Object { (($_.Actions | ForEach-Object { $_.Execute }) -join ' ') -match 'aperture-display' } | Select-Object -First 1; if ($t) { $t | Stop-ScheduledTask -ErrorAction SilentlyContinue }; Get-Process aperture-display -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Seconds 2; if ($t) { $t | Start-ScheduledTask }";
const PAWNIO_SCRIPT: &str = "winget install --id namazso.PawnIO -e --silent --accept-source-agreements --accept-package-agreements; $t = Get-ScheduledTask -ErrorAction SilentlyContinue | Where-Object { (($_.Actions | ForEach-Object { $_.Execute }) -join ' ') -match 'aperture-display' } | Select-Object -First 1; if ($t) { $t | Stop-ScheduledTask -ErrorAction SilentlyContinue }; Get-Process aperture-display -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep -Seconds 2; if ($t) { $t | Start-ScheduledTask }";

/// Starts the display: the autostart task when there is one, else the installer (which
/// creates it), else the program by itself.
const START_SCRIPT: &str = "$t = Get-ScheduledTask -ErrorAction SilentlyContinue | Where-Object { (($_.Actions | ForEach-Object { $_.Execute }) -join ' ') -match 'aperture-display' } | Select-Object -First 1; if ($t) { $t | Start-ScheduledTask } else { $d = '__DIR__'; $s = Join-Path $d 'scripts\\setup.ps1'; if (Test-Path -LiteralPath $s) { Start-Process powershell -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File',$s } else { Start-Process -FilePath (Join-Path $d 'aperture-display.exe') -ArgumentList '--keep-alive','--hide-console' -WindowStyle Hidden } }";

fn ps_quote(p: &Path) -> String {
    p.display().to_string().replace('\'', "''")
}

// ------------------------------------------------------------------------ updates

const RELEASES_API: &str = "https://api.github.com/repos/HuyBruh/Aperture/releases/latest";

#[derive(Clone)]
struct Release {
    tag: String,
    zip_url: String,
    zip_size: u64,
    sums_url: Option<String>,
    page: String,
    notes: Vec<String>,
}

enum UpdateMsg {
    Checked(Result<Option<Release>, String>),
    Progress(f32),
    Ready(PathBuf),
    Failed(String),
}

#[derive(Clone, PartialEq)]
enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Downloading(f32),
    Failed(String),
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.trim().parse::<u32>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Aperture/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// The latest release on GitHub when it is newer than this app.
fn check_latest() -> Result<Option<Release>, String> {
    use sensors_ext::json::{self, Value};
    let body = agent()
        .get(&std::env::var("APERTURE_UPDATE_URL").unwrap_or_else(|_| RELEASES_API.to_string()))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v = json::parse(&body).ok_or("unexpected answer from GitHub")?;
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("no release found")?.to_string();
    let latest = parse_version(&tag).ok_or("unexpected release tag")?;
    if latest <= parse_version(env!("CARGO_PKG_VERSION")).unwrap_or_default() {
        return Ok(None);
    }
    let assets = v.get("assets").and_then(Value::as_arr).unwrap_or(&[]);
    let find = |n: &str| assets.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(n));
    let zip = find("aperture-windows.zip").ok_or("the release has no aperture-windows.zip")?;
    let url = |a: &Value| a.get("browser_download_url").and_then(Value::as_str).map(str::to_string);
    let notes = v
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- ") || l.starts_with("* "))
        .map(|l| l[2..].replace("**", "").replace('`', ""))
        .take(4)
        .collect();
    Ok(Some(Release {
        tag,
        zip_url: url(zip).ok_or("no download link")?,
        zip_size: match zip.get("size") {
            Some(Value::Num(n)) => *n as u64,
            _ => 0,
        },
        sums_url: find("SHA256SUMS.txt").and_then(url),
        page: v.get("html_url").and_then(Value::as_str).unwrap_or("https://github.com/HuyBruh/Aperture/releases/latest").to_string(),
        notes,
    }))
}

/// Downloads the release ZIP to the temp folder and checks its SHA-256 against
/// SHA256SUMS.txt of the same release.
fn download(r: &Release, tx: &mpsc::Sender<UpdateMsg>, ctx: &egui::Context) -> Result<PathBuf, String> {
    use std::io::{Read, Write};
    let dir = std::env::temp_dir().join("aperture-update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("aperture-windows.zip");
    let resp = agent().get(&r.zip_url).call().map_err(|e| e.to_string())?;
    let total = if r.zip_size > 0 { r.zip_size } else { resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0) };
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut sha = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0u64;
    let mut shown = Instant::now();
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        sha.update(&buf[..n]);
        got += n as u64;
        if total > 0 && shown.elapsed() > Duration::from_millis(100) {
            shown = Instant::now();
            let _ = tx.send(UpdateMsg::Progress(got as f32 / total as f32));
            ctx.request_repaint();
        }
    }
    drop(file);
    if r.zip_size > 0 && got != r.zip_size {
        return Err("the download was cut off".into());
    }
    if let Some(u) = &r.sums_url {
        let sums = agent().get(u).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
        let have: String = sha.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
        let want = sums.lines().find(|l| l.contains("aperture-windows.zip")).and_then(|l| l.split_whitespace().next());
        if want.is_some_and(|w| !w.eq_ignore_ascii_case(&have)) {
            return Err("checksum does not match, nothing was changed".into());
        }
    }
    Ok(path)
}

/// Replaces the program files with the ones in the ZIP once this app has closed, keeps
/// trofeo.conf, asks the display program to restart and opens the new app. Running
/// programs are renamed to *.old.exe first (Windows allows that); the display program
/// removes those at its next start. No administrator rights needed.
fn apply_update(zip: &Path, app_dir: &Path) -> Result<(), String> {
    let script = format!(
        r#"$ErrorActionPreference = 'Continue'
Wait-Process -Id {pid} -Timeout 20 -ErrorAction SilentlyContinue
$app = '{app}'; $zip = '{zip}'
$tmp = Join-Path (Split-Path $zip) 'files'
Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
Expand-Archive -LiteralPath $zip -DestinationPath $tmp -Force
$exe = Get-ChildItem -LiteralPath $tmp -Recurse -Filter 'aperture-display.exe' | Select-Object -First 1
if ($exe) {{
  $src = $exe.DirectoryName
  Get-ChildItem -LiteralPath $src -Recurse -File | Unblock-File -ErrorAction SilentlyContinue
  foreach ($f in Get-ChildItem -LiteralPath $src -Recurse -File) {{
    $rel = $f.FullName.Substring($src.Length).TrimStart('')
    if ($rel -ieq 'trofeo.conf') {{ continue }}
    $dst = Join-Path $app $rel
    New-Item -ItemType Directory -Path (Split-Path $dst) -Force | Out-Null
    if ($f.Extension -ieq '.exe' -and (Test-Path -LiteralPath $dst)) {{
      Rename-Item -LiteralPath $dst -NewName ($f.BaseName + '.' + (Get-Random) + '.old.exe') -Force
    }}
    Copy-Item -LiteralPath $f.FullName -Destination $dst -Force
  }}
  Set-Content -LiteralPath (Join-Path $app 'trofeo-restart.request') -Value 'update'
}}
Start-Process -FilePath (Join-Path $app 'aperture.exe')
Remove-Item -LiteralPath (Split-Path $zip) -Recurse -Force -ErrorAction SilentlyContinue
"#,
        pid = std::process::id(),
        app = ps_quote(app_dir),
        zip = ps_quote(zip)
    );
    #[cfg(windows)]
    {
        let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        hidden(std::process::Command::new("powershell").args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-EncodedCommand", &base64(&utf16)]))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = script;
        Err("updates are installed on Windows only".into())
    }
}

/// Ready-made combinations of a layout and a palette.
const LOOKS: &[(&str, &str, &str, &str)] = &[
    ("Calm", "Dịu mắt", "grid", "mineral"),
    ("Paper", "Giấy", "studio", "sand"),
    ("Departures", "Sân bay", "board", "ember"),
    ("Race", "Đua xe", "cluster", "crimson"),
    ("Listening", "Nghe nhạc", "atelier", "iris"),
    ("Analyst", "Theo dõi", "pulse", "arctic"),
];

// ------------------------------------------------------------------------ app

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Layouts,
    Colors,
    Canvas,
    Display,
    Features,
    Sensors,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Only(Group),
}

struct App {
    lang: Lang,
    page: Page,
    filter: Filter,
    path: PathBuf,
    dir: PathBuf,
    doc: ConfDoc,
    original: String,
    rev: u64,
    saved_rev: u64,
    changed_at: Option<Instant>,
    save_error: Option<String>,
    restart_needed: bool,
    // pictures
    jobs: Option<mpsc::Sender<Job>>,
    done: Option<mpsc::Receiver<Done>>,
    preview: Option<egui::TextureHandle>,
    preview_rev: u64,
    preview_music: bool,
    preview_asked: Option<(u64, bool)>,
    thumbs: HashMap<String, egui::TextureHandle>,
    thumbs_key: String,
    thumbs_asked_at: Option<Instant>,
    // status
    status: Status,
    status_rx: Option<mpsc::Receiver<Status>>,
    status_at: Option<Instant>,
    // text boxes
    city: String,
    ignore: String,
    adapter: String,
    sgdb_key: String,
    custom: [u8; 3],
    logo: Option<egui::TextureHandle>,
    // the live view of the panel
    live: bool,
    live_tex: Option<egui::TextureHandle>,
    live_stamp: Option<std::time::SystemTime>,
    live_at: Option<Instant>,
    // updates
    update_rx: Option<mpsc::Receiver<UpdateMsg>>,
    release: Option<Release>,
    update_state: UpdateState,
    update_hidden: bool,
    note: Option<(String, Instant)>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        setup_style(&cc.egui_ctx);
        let path = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
            std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("trofeo.conf"))).unwrap_or_else(|| PathBuf::from("trofeo.conf"))
        });
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| "layout = pro\npro_layout = board\npro_theme = mineral\n".to_string());
        let doc = ConfDoc::parse(&text);
        let lang = match doc.get("studio_language").as_deref() {
            Some("vi") => Lang::Vi,
            Some("en") => Lang::En,
            _ => {
                let sys = std::env::var("LANG").unwrap_or_default();
                if sys.starts_with("vi") || system_is_vietnamese() { Lang::Vi } else { Lang::En }
            }
        };
        let city = doc.get("weather_city").unwrap_or_default();
        let ignore = doc.get("fps_ignore").unwrap_or_default();
        let adapter = doc.get("pro_amd_adapter").unwrap_or_default();
        let sgdb_key = doc.get("steamgriddb_key").unwrap_or_default();
        let custom = doc.get("pro_theme").and_then(|t| parse_hex(&t)).or_else(|| doc.get("pro_accent").and_then(|t| parse_hex(&t))).unwrap_or([255, 0, 240]);
        let (jobs, done) = match find_main_exe() {
            Some(exe) => {
                let (a, b) = spawn_renderer(exe, dir.clone(), cc.egui_ctx.clone());
                (Some(a), Some(b))
            }
            None => (None, None),
        };
        // start on the group of the layout in use
        let current = doc.get("pro_layout").unwrap_or_default();
        let filter = LAYOUTS.iter().find(|l| l.key == current).map_or(Filter::All, |l| Filter::Only(l.group));
        let mut app = App {
            lang,
            page: match std::env::var("TROFEO_STUDIO_PAGE").as_deref() {
                Ok("colors") => Page::Colors,
                Ok("canvas") => Page::Canvas,
                Ok("display") => Page::Display,
                Ok("features") => Page::Features,
                Ok("help") | Ok("sensors") => Page::Sensors,
                _ => Page::Layouts,
            },
            filter: if std::env::var("TROFEO_STUDIO_PAGE").is_ok() { Filter::All } else { filter },
            path,
            dir,
            doc,
            original: text,
            rev: 1,
            saved_rev: 1,
            changed_at: None,
            save_error: None,
            restart_needed: false,
            jobs,
            done,
            preview: None,
            preview_rev: 0,
            preview_music: true,
            preview_asked: None,
            thumbs: HashMap::new(),
            thumbs_key: String::new(),
            thumbs_asked_at: None,
            status: Status::default(),
            status_rx: None,
            status_at: None,
            city,
            ignore,
            adapter,
            sgdb_key,
            custom,
            logo: image::load_from_memory(include_bytes!("../../assets/studio-icon.png")).ok().map(|i| {
                let i = i.to_rgba8();
                let img = egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
                cc.egui_ctx.load_texture("logo", img, egui::TextureOptions::LINEAR)
            }),
            live: false,
            live_tex: None,
            live_stamp: None,
            live_at: None,
            update_rx: None,
            release: None,
            update_state: UpdateState::Idle,
            update_hidden: false,
            note: None,
        };
        if app.bool_of("update_check", true) && std::env::var_os("TROFEO_STUDIO_PAGE").is_none() {
            app.check_updates(&cc.egui_ctx);
        }
        app
    }

    fn check_updates(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(UpdateMsg::Checked(check_latest()));
            c.request_repaint();
        });
        self.update_rx = Some(rx);
        self.update_state = UpdateState::Checking;
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        let Some(r) = self.release.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let msg = match download(&r, &tx, &c) {
                Ok(p) => UpdateMsg::Ready(p),
                Err(e) => UpdateMsg::Failed(e),
            };
            let _ = tx.send(msg);
            c.request_repaint();
        });
        self.update_rx = Some(rx);
        self.update_state = UpdateState::Downloading(0.0);
    }

    /// Restarts the display program: through a request file when it runs under
    /// `--keep-alive` (no administrator prompt), else through its autostart task.
    fn restart_display(&mut self) {
        let s = &self.status;
        if s.alive && s.get("supervised") == Some("1") && std::fs::write(self.dir.join("trofeo-restart.request"), "restart").is_ok() {
            self.note = Some((self.t("Restarting the display…", "Đang khởi động lại màn hình…").into(), Instant::now()));
        } else if s.alive {
            run_elevated(RESTART_SCRIPT);
        } else {
            run_elevated(&START_SCRIPT.replace("__DIR__", &ps_quote(&self.dir)));
        }
        self.restart_needed = false;
        self.status_at = None;
    }

    /// What support needs to know, as text for the clipboard.
    fn diagnostics(&self) -> String {
        let s = &self.status;
        let mut out = format!(
            "Aperture {}\nsettings: {}\ndisplay program running: {}\nPawnIO: {}\nexternal sensors: {}\n",
            env!("CARGO_PKG_VERSION"),
            self.path.display(),
            s.alive,
            match s.pawnio {
                Some(true) => "installed",
                Some(false) => "not installed",
                None => "-",
            },
            match s.ext.as_ref().map(|(src, _)| *src) {
                Some(Source::Hwinfo) => "HWiNFO",
                Some(Source::Lhm) => "LibreHardwareMonitor",
                None => "none",
            }
        );
        let mut keys: Vec<_> = s.map.iter().collect();
        keys.sort();
        for (k, v) in keys {
            out.push_str(&format!("{k}={v}\n"));
        }
        for key in ["pro_layout", "pro_theme", "orientation", "flip", "pro_fan_sensor", "fps_ignore"] {
            if let Some(v) = self.get(key) {
                out.push_str(&format!("conf.{key}={v}\n"));
            }
        }
        out
    }

    fn t<'a>(&self, en: &'a str, vi: &'a str) -> &'a str {
        match self.lang {
            Lang::En => en,
            Lang::Vi => vi,
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.doc.get(key)
    }

    fn get_or(&self, key: &str, def: &str) -> String {
        self.get(key).filter(|v| !v.trim().is_empty()).unwrap_or_else(|| def.to_string())
    }

    fn set(&mut self, key: &str, v: &str) {
        if self.doc.get(key).as_deref() != Some(v) {
            self.doc.set(key, v);
            self.touch();
        }
    }

    fn unset(&mut self, key: &str) {
        if self.doc.is_set(key) {
            self.doc.unset(key);
            self.touch();
        }
    }

    fn touch(&mut self) {
        self.rev += 1;
        self.changed_at = Some(Instant::now());
    }

    fn bool_of(&self, key: &str, def: bool) -> bool {
        match self.get(key).map(|v| v.trim().to_ascii_lowercase()) {
            Some(v) if !v.is_empty() => matches!(v.as_str(), "true" | "yes" | "on" | "1"),
            _ => def,
        }
    }

    fn set_bool(&mut self, key: &str, on: bool) {
        self.set(key, if on { "true" } else { "false" });
    }

    /// Picks a layout (and leaves the canvas editor).
    fn use_layout(&mut self, key: &str) {
        self.set("pro_layout", key);
        self.set("pro_custom_enabled", "false");
    }

    /// Writes the file a moment after the last change (the display picks it up by itself).
    fn autosave(&mut self) {
        if self.rev == self.saved_rev {
            return;
        }
        if self.changed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            return;
        }
        // the dashboard needs `layout = pro`
        if self.doc.get("layout").as_deref() != Some("pro") {
            self.doc.set("layout", "pro");
        }
        match std::fs::write(&self.path, self.doc.to_text()) {
            Ok(()) => {
                self.saved_rev = self.rev;
                self.save_error = None;
            }
            Err(e) => self.save_error = Some(e.to_string()),
        }
    }

    fn autosave_now(&mut self) {
        self.changed_at = None;
        self.autosave();
    }

    fn set_live(&mut self, on: bool) {
        self.live = on;
        if !on {
            self.live_tex = None;
            self.live_stamp = None;
            let _ = std::fs::remove_file(self.dir.join("trofeo-live.request"));
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        // finished pictures
        if let Some(rx) = &self.done {
            while let Ok(d) = rx.try_recv() {
                match d {
                    Done::Preview(rev, Some(img)) => {
                        if rev >= self.preview_rev {
                            self.preview_rev = rev;
                            self.preview = Some(ctx.load_texture("preview", img, egui::TextureOptions::LINEAR));
                        }
                    }
                    Done::Thumb(_, key, Some(img)) => {
                        self.thumbs.insert(key.clone(), ctx.load_texture(format!("thumb-{key}"), img, egui::TextureOptions::LINEAR));
                    }
                    _ => {}
                }
            }
        }
        // a new preview after changes
        let want = (self.rev, self.preview_music);
        if self.preview_asked != Some(want) && self.changed_at.map_or(true, |t| t.elapsed() > Duration::from_millis(200)) {
            if let Some(tx) = &self.jobs {
                let _ = tx.send(Job::Preview { text: self.doc.to_text(), music: self.preview_music, rev: self.rev });
            }
            self.preview_asked = Some(want);
        }
        // thumbnails follow the colors and formats: redraw when those change
        let key = format!(
            "{}|{}|{}|{}|{}|{}",
            self.get_or("pro_theme", "mineral"),
            self.get_or("pro_style", "cards"),
            self.get_or("pro_accent", ""),
            self.get_or("pro_clock", "24h"),
            self.get_or("weather_unit", "c"),
            self.get_or("pro_cover_color", "true")
        );
        if key != self.thumbs_key {
            if self.thumbs_asked_at.is_none() {
                self.thumbs_asked_at = Some(Instant::now());
            }
            if self.thumbs_asked_at.is_some_and(|t| t.elapsed() > Duration::from_millis(700)) {
                if let Some(tx) = &self.jobs {
                    let first: Vec<&'static str> = match self.filter {
                        Filter::All => Vec::new(),
                        Filter::Only(g) => LAYOUTS.iter().filter(|l| l.group == g).map(|l| l.key).collect(),
                    };
                    let _ = tx.send(Job::Thumbs { text: self.doc.to_text(), rev: self.rev, first });
                }
                self.thumbs_key = key;
                self.thumbs_asked_at = None;
            }
        }
        // updates
        if let Some(rx) = &self.update_rx {
            let mut close = false;
            while let Ok(m) = rx.try_recv() {
                match m {
                    UpdateMsg::Checked(Ok(Some(r))) => {
                        self.release = Some(r);
                        self.update_state = UpdateState::Idle;
                    }
                    UpdateMsg::Checked(Ok(None)) => self.update_state = UpdateState::UpToDate,
                    UpdateMsg::Checked(Err(e)) => self.update_state = UpdateState::Failed(e),
                    UpdateMsg::Progress(f) => self.update_state = UpdateState::Downloading(f),
                    UpdateMsg::Failed(e) => self.update_state = UpdateState::Failed(e),
                    UpdateMsg::Ready(zip) => match apply_update(&zip, &self.dir) {
                        Ok(()) => close = true,
                        Err(e) => self.update_state = UpdateState::Failed(e),
                    },
                }
            }
            if close {
                self.autosave_now();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        // the live view: ask the display program for a picture every second
        if self.live && self.live_at.map_or(true, |t| t.elapsed() > Duration::from_millis(1000)) {
            self.live_at = Some(Instant::now());
            // new content every time: writing the same (empty) bytes does not move the file's
            // modified time on Windows, and that time is what the display program looks at
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let _ = std::fs::write(self.dir.join("trofeo-live.request"), stamp.to_string());
            let png = self.dir.join("trofeo-live.png");
            let stamp = std::fs::metadata(&png).and_then(|m| m.modified()).ok();
            if stamp.is_some() && stamp != self.live_stamp {
                if let Ok(i) = image::open(&png) {
                    let i = i.to_rgba8();
                    let img = egui::ColorImage::from_rgba_unmultiplied([i.width() as usize, i.height() as usize], i.as_raw());
                    self.live_tex = Some(ctx.load_texture("live", img, egui::TextureOptions::LINEAR));
                    self.live_stamp = stamp;
                }
            }
        }
        // status in the background: every 1.5 s on the Sensors page, every 4 s elsewhere
        if let Some(rx) = &self.status_rx {
            if let Ok(s) = rx.try_recv() {
                self.status = s;
                self.status_rx = None;
            }
        }
        let every = if self.page == Page::Sensors { Duration::from_millis(1500) } else { Duration::from_secs(4) };
        if self.status_rx.is_none() && self.status_at.map_or(true, |t| t.elapsed() > every) {
            let (tx, rx) = mpsc::channel();
            let dir = self.dir.clone();
            let c = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(read_status(&dir));
                c.request_repaint();
            });
            self.status_rx = Some(rx);
            self.status_at = Some(Instant::now());
        }
        ctx.request_repaint_after(Duration::from_millis(if self.page == Page::Sensors || self.live { 500 } else { 1000 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_compare_by_number() {
        assert_eq!(parse_version("v4.3.0"), Some((4, 3, 0)));
        assert_eq!(parse_version("4.2"), Some((4, 2, 0)));
        assert_eq!(parse_version("v4.10.1-beta.2"), Some((4, 10, 1)));
        assert!(parse_version("v4.10.0") > parse_version("4.9.9"));
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(super::base64(b"Man"), "TWFu");
        assert_eq!(super::base64(b"Ma"), "TWE=");
        assert_eq!(super::base64(b"M"), "TQ==");
        assert_eq!(super::base64(b""), "");
    }
}

#[cfg(windows)]
fn system_is_vietnamese() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    unsafe { GetUserDefaultUILanguage() & 0x3FF == 0x2A }
}

#[cfg(not(windows))]
fn system_is_vietnamese() -> bool {
    false
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn rgb(c: (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

// ------------------------------------------------------------------------ widgets

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, LINE))
        .rounding(R)
        .inner_margin(egui::Margin::symmetric(22.0, 14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn section_title(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).font(font(16.5, Weight::Semibold)).color(TEXT));
    if !sub.is_empty() {
        ui.add_space(-7.0);
        ui.label(RichText::new(sub).font(font(13.0, Weight::Regular)).color(MUTED));
    }
    ui.add_space(4.0);
}

/// A row: label + help on the left, the control on the right.
fn row(ui: &mut egui::Ui, label: &str, help: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(48.0);
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 360.0).max(220.0));
            ui.add_space(if help.is_empty() { 14.0 } else { 6.0 });
            ui.label(RichText::new(label).font(font(14.5, Weight::Medium)).color(TEXT));
            if !help.is_empty() {
                ui.add_space(-7.0);
                ui.label(RichText::new(help).font(font(12.5, Weight::Regular)).color(MUTED));
            }
        });
        ui.with_layout(ELayout::right_to_left(Align::Center), control);
    });
}

fn divider(ui: &mut egui::Ui) {
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().top() + 1.0;
    ui.painter().hline(r.left()..=r.right(), y, Stroke::new(1.0_f32, LINE));
    ui.add_space(4.0);
}

/// iOS-style switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(46.0, 27.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = lerp_color(Color32::from_rgb(52, 56, 68), ACCENT, t);
    ui.painter().rect_filled(rect, Rounding::same(13.5), bg);
    let x = egui::lerp(rect.left() + 13.5..=rect.right() - 13.5, t);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 10.5, Color32::WHITE);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Segmented control; returns the newly picked value.
fn segmented(ui: &mut egui::Ui, current: &str, options: &[(&str, &str)]) -> Option<String> {
    let mut picked = None;
    let h = 34.0;
    let pad = 14.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| ui.painter().layout_no_wrap(label.to_string(), font(13.5, Weight::Medium), TEXT).size().x + 2.0 * pad)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, h), Sense::hover());
    ui.painter().rect(rect, Rounding::same(10.0), FIELD, Stroke::new(1.0_f32, LINE));
    let mut x = rect.left() + 3.0;
    for ((value, label), w) in options.iter().zip(widths) {
        let r = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(w, h - 6.0));
        let resp = ui.interact(r, ui.id().with(("seg", value, label)), Sense::click());
        let on = *value == current;
        if on {
            ui.painter().rect_filled(r, Rounding::same(8.0), Color32::from_rgb(52, 56, 70));
        } else if resp.hovered() {
            ui.painter().rect_filled(r, Rounding::same(8.0), Color32::from_rgb(30, 32, 40));
        }
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, *label, font(13.5, Weight::Medium), if on { TEXT } else { MUTED });
        if resp.clicked() && !on {
            picked = Some(value.to_string());
        }
        x += w;
    }
    picked
}

/// A filter pill; returns true when clicked.
fn chip(ui: &mut egui::Ui, label: &str, count: usize, on: bool) -> bool {
    let text = if count == 0 { label.to_string() } else { format!("{label}  {count}") };
    let g = ui.painter().layout_no_wrap(text.clone(), font(13.5, Weight::Medium), TEXT);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 30.0, 34.0), Sense::click());
    let (fill, stroke, color) = if on {
        (ACCENT_SOFT, ACCENT, TEXT)
    } else if resp.hovered() {
        (CARD_HI, LINE, TEXT)
    } else {
        (CARD, LINE, MUTED)
    };
    ui.painter().rect(rect, Rounding::same(17.0), fill, Stroke::new(1.0_f32, stroke));
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, font(13.5, Weight::Medium), color);
    resp.clicked()
}

/// "Name  value" in a rounded field, never wrapped inside.
fn pill(ui: &mut egui::Ui, name: &str, value: &str, on: bool) {
    let gn = ui.painter().layout_no_wrap(name.to_string(), font(13.0, Weight::Regular), if on { MUTED } else { DIM });
    let gv = ui.painter().layout_no_wrap(value.to_string(), font(13.0, Weight::Semibold), if on { TEXT } else { DIM });
    let w = gn.size().x + gv.size().x + 34.0;
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
    ui.painter().rect_filled(r, Rounding::same(9.0), FIELD);
    let y = r.center().y - gn.size().y / 2.0;
    let vx = r.left() + 12.0 + gn.size().x + 10.0;
    ui.painter().galley(Pos2::new(r.left() + 12.0, y), gn, MUTED);
    ui.painter().galley(Pos2::new(vx, y), gv, TEXT);
}

fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    let g = ui.painter().layout_no_wrap(text.to_string(), font(13.5, Weight::Medium), TEXT);
    let (r, _) = ui.allocate_exact_size(Vec2::new(g.size().x + 18.0, 22.0), Sense::hover());
    ui.painter().circle_filled(Pos2::new(r.left() + 5.0, r.center().y), 4.5, color);
    ui.painter().galley(Pos2::new(r.left() + 18.0, r.center().y - g.size().y / 2.0), g, TEXT);
}

fn bullet(ui: &mut egui::Ui, text: &str) {
    ui.add_space(-4.0);
    ui.label(RichText::new(format!("•   {text}")).font(font(13.0, Weight::Regular)).color(MUTED));
}

fn badge(p: &egui::Painter, right_top: Pos2, text: &str, fill: Color32, color: Color32) {
    let g = p.layout_no_wrap(text.to_string(), font(11.0, Weight::Semibold), color);
    let r = Rect::from_min_size(Pos2::new(right_top.x - g.size().x - 16.0, right_top.y), Vec2::new(g.size().x + 16.0, 22.0));
    p.rect_filled(r, Rounding::same(11.0), fill);
    p.galley(Pos2::new(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, color);
}

/// Small line icons for the side bar.
fn icon(p: &egui::Painter, page: Page, c: Pos2, color: Color32) {
    let s = Stroke::new(1.7_f32, color);
    match page {
        Page::Layouts => {
            for (dx, dy) in [(-4.5, -4.5), (4.5, -4.5), (-4.5, 4.5), (4.5, 4.5)] {
                p.rect_stroke(Rect::from_center_size(c + Vec2::new(dx, dy), Vec2::splat(6.5)), Rounding::same(1.8), s);
            }
        }
        Page::Colors => {
            p.circle_stroke(c + Vec2::new(-3.5, -2.5), 5.0, s);
            p.circle_stroke(c + Vec2::new(3.5, -2.5), 5.0, s);
            p.circle_stroke(c + Vec2::new(0.0, 3.5), 5.0, s);
        }
        Page::Canvas => {
            for (i, kx) in [(-5.0f32, 3.0f32), (0.0, -3.0), (5.0, 1.0)] {
                p.line_segment([c + Vec2::new(-8.0, i), c + Vec2::new(8.0, i)], s);
                p.circle_filled(c + Vec2::new(kx, i), 2.6, color);
            }
        }
        Page::Display => {
            p.rect_stroke(Rect::from_center_size(c + Vec2::new(0.0, -2.0), Vec2::new(17.0, 11.0)), Rounding::same(2.0), s);
            p.line_segment([c + Vec2::new(-4.0, 7.0), c + Vec2::new(4.0, 7.0)], s);
            p.line_segment([c + Vec2::new(0.0, 3.5), c + Vec2::new(0.0, 7.0)], s);
        }
        Page::Features => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(17.0, 10.0)), Rounding::same(5.0), s);
            p.circle_filled(c + Vec2::new(3.5, 0.0), 3.0, color);
        }
        Page::Sensors => {
            let pts = [(-8.5, 1.0), (-4.0, 1.0), (-2.0, -5.0), (1.5, 6.0), (4.0, -1.0), (8.5, -1.0)];
            for w in pts.windows(2) {
                p.line_segment([c + Vec2::new(w[0].0, w[0].1), c + Vec2::new(w[1].0, w[1].1)], s);
            }
        }
    }
}

// ------------------------------------------------------------------------ frame

impl App {
    fn page_title(&self, page: Page) -> (&'static str, &'static str) {
        match (page, self.lang) {
            (Page::Layouts, Lang::En) => ("Layouts", "Pick what the display looks like. Every design is here, nothing hidden."),
            (Page::Layouts, Lang::Vi) => ("Bố cục", "Chọn giao diện cho màn hình. Tất cả mẫu đều ở đây, không ẩn mẫu nào."),
            (Page::Colors, Lang::En) => ("Colors", "Soft palettes, vivid palettes or your own color."),
            (Page::Colors, Lang::Vi) => ("Màu sắc", "Bộ màu dịu, bộ màu rực rỡ hoặc màu của riêng bạn."),
            (Page::Canvas, Lang::En) => ("Customize", "Move and resize panels, or build your own screen from single readings."),
            (Page::Canvas, Lang::Vi) => ("Tùy biến", "Di chuyển, đổi kích thước ô, hoặc tự dựng màn hình từ từng thông số."),
            (Page::Display, Lang::En) => ("Display", "Orientation, brightness, clock and weather."),
            (Page::Display, Lang::Vi) => ("Hiển thị", "Chiều màn hình, độ sáng, đồng hồ và thời tiết."),
            (Page::Features, Lang::En) => ("Features", "Music, game mode, night mode and alerts."),
            (Page::Features, Lang::Vi) => ("Tính năng", "Nhạc, chế độ chơi game, ban đêm và cảnh báo."),
            (Page::Sensors, Lang::En) => ("Sensors", "Live readings, where each one comes from, and how to fix the missing ones."),
            (Page::Sensors, Lang::Vi) => ("Cảm biến", "Số liệu trực tiếp, mỗi thông số lấy từ đâu và cách sửa khi thiếu."),
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new((logo.id(), Vec2::new(36.0, 36.0))).rounding(Rounding::same(9.0)));
            }
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.label(RichText::new("Aperture").font(font(18.0, Weight::Semibold)).color(TEXT));
                ui.add_space(-8.0);
                ui.label(RichText::new("by HuyBruh").font(font(12.0, Weight::Regular)).color(DIM));
            });
        });
        ui.add_space(18.0);
        let pages = [Page::Layouts, Page::Colors, Page::Canvas, Page::Display, Page::Features, Page::Sensors];
        for p in pages {
            let name = self.page_title(p).0;
            let on = self.page == p;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
            if on {
                ui.painter().rect_filled(rect, Rounding::same(10.0), CARD_HI);
                ui.painter().rect_filled(Rect::from_min_size(rect.min + Vec2::new(0.0, 11.0), Vec2::new(3.0, 20.0)), Rounding::same(1.5), ACCENT);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, Rounding::same(10.0), Color32::from_rgb(24, 26, 31));
            }
            let col = if on { TEXT } else { MUTED };
            icon(ui.painter(), p, Pos2::new(rect.left() + 26.0, rect.center().y), if on { ACCENT } else { col });
            ui.painter().text(Pos2::new(rect.left() + 48.0, rect.center().y), egui::Align2::LEFT_CENTER, name, font(14.5, Weight::Medium), col);
            if resp.clicked() {
                self.page = p;
            }
            ui.add_space(-6.0);
        }
        // bottom: connection, language
        ui.with_layout(ELayout::bottom_up(Align::Min), |ui| {
            ui.add_space(6.0);
            let cur = if self.lang == Lang::En { "en" } else { "vi" };
            if let Some(v) = segmented(ui, cur, &[("en", "English"), ("vi", "Tiếng Việt")]) {
                self.lang = if v == "vi" { Lang::Vi } else { Lang::En };
                self.set("studio_language", &v);
            }
            ui.add_space(4.0);
            if ui.add_sized([ui.available_width(), 34.0], egui::Button::new(self.t("Restart display", "Khởi động lại màn hình"))).clicked() {
                self.restart_display();
            }
            ui.add_space(2.0);
            let (color, text) = self.connection();
            let app_v = env!("CARGO_PKG_VERSION");
            let disp_v = self.status.get("version").filter(|_| self.status.alive).map(str::to_string);
            egui::Frame::none().fill(CARD).rounding(10.0).inner_margin(egui::Margin::symmetric(12.0, 10.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                // (the side bar is laid out bottom up: the last line comes first)
                let (vtext, vcol) = match &disp_v {
                    Some(d) if d != app_v => (format!("{} {d} · {} {app_v}", self.t("Display", "Màn hình"), self.t("app", "app")), WARN),
                    _ => (format!("Aperture {app_v}"), DIM),
                };
                ui.label(RichText::new(vtext).font(font(12.0, Weight::Regular)).color(vcol));
                ui.add_space(-4.0);
                status_dot(ui, color, &text);
            });
        });
    }

    fn connection(&self) -> (Color32, String) {
        let s = &self.status;
        if self.status_at.is_none() || (self.status.map.is_empty() && self.status_rx.is_some()) {
            return (DIM, self.t("Checking…", "Đang kiểm tra…").into());
        }
        if !s.alive {
            return (BAD, self.t("Display program not running", "Chương trình màn hình chưa chạy").into());
        }
        match s.get("lcd") {
            Some("connected") => (GOOD, self.t("Display connected", "Đã kết nối màn hình").into()),
            Some("not_found") => (BAD, self.t("Display not found", "Không tìm thấy màn hình").into()),
            _ => (WARN, self.t("Starting…", "Đang khởi động…").into()),
        }
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        let (title, sub) = self.page_title(self.page);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(title).font(font(24.0, Weight::Semibold)).color(TEXT));
                ui.add_space(-8.0);
                ui.label(RichText::new(sub).font(font(13.5, Weight::Regular)).color(MUTED));
            });
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                let saved = self.rev == self.saved_rev;
                let (c, msg) = if let Some(e) = &self.save_error {
                    (BAD, format!("{} {e}", self.t("Could not save:", "Không lưu được:")))
                } else if saved {
                    (DIM, self.t("Saved · applies in a second", "Đã lưu · áp dụng sau 1 giây").to_string())
                } else {
                    (WARN, self.t("Saving…", "Đang lưu…").to_string())
                };
                if self.original != self.doc.to_text() && ui.button(self.t("Undo all changes", "Hoàn tác tất cả")).clicked() {
                    self.doc = ConfDoc::parse(&self.original);
                    self.city = self.doc.get("weather_city").unwrap_or_default();
                    self.ignore = self.doc.get("fps_ignore").unwrap_or_default();
                    self.adapter = self.doc.get("pro_amd_adapter").unwrap_or_default();
                    self.touch();
                }
                ui.label(RichText::new(msg).font(font(12.5, Weight::Regular)).color(c));
            });
        });
    }

    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.release.clone() else { return };
        if self.update_hidden {
            return;
        }
        let state = self.update_state.clone();
        egui::Frame::none().fill(ACCENT_SOFT).stroke(Stroke::new(1.0_f32, ACCENT)).rounding(R).inner_margin(egui::Margin::symmetric(16.0, 12.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let title = format!("{} {}", self.t("New version available:", "Đã có phiên bản mới:"), r.tag);
                    ui.label(RichText::new(title).font(font(15.0, Weight::Semibold)).color(TEXT));
                    let sub = match &state {
                        UpdateState::Downloading(f) => format!("{} {:.0}%", self.t("Downloading…", "Đang tải…"), f * 100.0),
                        UpdateState::Failed(e) => format!("{} {e}", self.t("Update failed:", "Cập nhật lỗi:")),
                        _ => r.notes.first().cloned().unwrap_or_else(|| self.t("Your settings are kept. The display restarts by itself.", "Giữ nguyên cài đặt của bạn. Màn hình tự khởi động lại.").to_string()),
                    };
                    ui.add_space(-6.0);
                    ui.label(RichText::new(sub).font(font(13.0, Weight::Regular)).color(if matches!(state, UpdateState::Failed(_)) { BAD } else { MUTED }));
                });
                ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                    match state {
                        UpdateState::Downloading(f) => {
                            ui.add(egui::ProgressBar::new(f).desired_width(180.0));
                        }
                        _ => {
                            let go = egui::Button::new(RichText::new(self.t("Update now", "Cập nhật ngay")).color(Color32::WHITE)).fill(ACCENT);
                            if ui.add(go).clicked() {
                                self.start_update(ui.ctx());
                            }
                            if ui.button(self.t("What's new", "Có gì mới")).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(&r.page));
                            }
                            if ui.button(self.t("Later", "Để sau")).clicked() {
                                self.update_hidden = true;
                            }
                        }
                    }
                });
            });
        });
        ui.add_space(10.0);
    }

    fn preview_card(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let h = (w * 462.0 / 1920.0).round();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
        ui.painter().rect_filled(rect, Rounding::same(R), Color32::BLACK);
        let shown = if self.live { self.live_tex.as_ref().filter(|_| self.status.alive) } else { self.preview.as_ref() };
        if self.live && shown.is_none() {
            let msg = if !self.status.alive {
                self.t("The display program is not running", "Chương trình màn hình chưa chạy")
            } else {
                self.t("Waiting for the display…", "Đang chờ màn hình…")
            };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, font(14.0, Weight::Regular), MUTED);
        } else if let Some(tex) = shown {
            let ts = tex.size_vec2();
            let s = (rect.width() / ts.x).min(rect.height() / ts.y);
            let r2 = Rect::from_center_size(rect.center(), ts * s);
            egui::Image::new((tex.id(), r2.size())).rounding(Rounding::same(R)).paint_at(ui, r2);
        } else {
            let msg = if self.jobs.is_none() {
                self.t("aperture-display.exe was not found next to Aperture", "Không tìm thấy aperture-display.exe cạnh Aperture")
            } else {
                self.t("Drawing the preview…", "Đang vẽ bản xem trước…")
            };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, font(14.0, Weight::Regular), MUTED);
        }
        ui.painter().rect_stroke(rect, Rounding::same(R), Stroke::new(1.0_f32, LINE));
        ui.add_space(-2.0);
        ui.horizontal(|ui| {
            let caption = if self.live {
                self.t("Live: exactly what the panel shows now, once a second", "Trực tiếp: đúng hình màn hình đang hiện, mỗi giây một lần")
            } else {
                self.t("Preview with sample data", "Xem trước với dữ liệu mẫu")
            };
            ui.label(RichText::new(caption).font(font(12.5, Weight::Regular)).color(DIM));
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                let cur = if self.live { "l" } else if self.preview_music { "m" } else { "i" };
                let lm = self.t("Music playing", "Đang phát nhạc").to_string();
                let li = self.t("No music", "Không phát nhạc").to_string();
                let ll = self.t("Live display", "Màn hình thật").to_string();
                if let Some(v) = segmented(ui, cur, &[("i", &li), ("m", &lm), ("l", &ll)]) {
                    self.set_live(v == "l");
                    if v != "l" {
                        self.preview_music = v == "m";
                    }
                }
            });
        });
    }
}

// ------------------------------------------------------------------------ pages

impl App {
    fn group_name(&self, g: Group) -> &'static str {
        match (g, self.lang) {
            (Group::New, Lang::En) => "New",
            (Group::New, Lang::Vi) => "Mới",
            (Group::Aperture, _) => "Aperture",
            (Group::Signature, Lang::En) => "Signature",
            (Group::Signature, Lang::Vi) => "Đặc trưng",
            (Group::Classic, Lang::En) => "Classic",
            (Group::Classic, Lang::Vi) => "Cổ điển",
            (Group::Portrait, Lang::En) => "Portrait",
            (Group::Portrait, Lang::Vi) => "Màn dọc",
        }
    }

    fn page_layouts(&mut self, ui: &mut egui::Ui) {
        let current = self.get_or("pro_layout", "board");
        let custom = self.bool_of("pro_custom_enabled", false);
        if custom {
            ui.add_space(4.0);
            egui::Frame::none().fill(ACCENT_SOFT).stroke(Stroke::new(1.0_f32, ACCENT)).rounding(R).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(self.t("Your free canvas is on the display. Pick a layout below to switch back.", "Màn hình đang dùng canvas tự dựng của bạn. Chọn một bố cục bên dưới để quay lại.")).color(TEXT));
                });
            });
        }
        ui.add_space(8.0);
        self.quick_looks(ui);
        ui.add_space(4.0);
        // filter chips
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let all = self.t("All", "Tất cả").to_string();
            if chip(ui, &all, LAYOUTS.len(), self.filter == Filter::All) {
                self.filter = Filter::All;
            }
            for g in [Group::New, Group::Aperture, Group::Signature, Group::Classic, Group::Portrait] {
                let n = LAYOUTS.iter().filter(|l| l.group == g).count();
                let name = self.group_name(g);
                if chip(ui, name, n, self.filter == Filter::Only(g)) {
                    self.filter = Filter::Only(g);
                }
            }
        });
        ui.add_space(6.0);
        let groups: Vec<Group> = match self.filter {
            Filter::All => vec![Group::New, Group::Aperture, Group::Signature, Group::Classic, Group::Portrait],
            Filter::Only(g) => vec![g],
        };
        for g in groups {
            let items: Vec<&LayoutInfo> = LAYOUTS.iter().filter(|l| l.group == g).collect();
            if self.filter == Filter::All {
                section_title(ui, self.group_name(g), "");
            }
            let cols = if g == Group::Portrait { 5 } else if ui.available_width() > 1100.0 { 3 } else { 2 };
            if let Some(k) = self.grid(ui, &items, &current, cols) {
                self.use_layout(&k);
            }
        }
        ui.add_space(6.0);
        card(ui, |ui| {
            let rotating = current.contains(',') || current == "all";
            let mut on = rotating;
            let lbl = self.t("Rotate layouts", "Tự đổi bố cục").to_string();
            let help = self.t("Cycles through the New, Aperture and favourite layouts", "Lần lượt đổi qua các bố cục Mới, Aperture và yêu thích").to_string();
            let mut changed = false;
            row(ui, &lbl, &help, |ui| {
                changed = toggle(ui, &mut on).changed();
            });
            if changed {
                if on {
                    self.use_layout("all");
                } else {
                    self.use_layout("board");
                }
            }
            if on {
                divider(ui);
                let mut m: u32 = self.get("pro_theme_interval").and_then(|v| v.trim().parse().ok()).unwrap_or(30).clamp(1, 240);
                let lbl = self.t("Change every", "Đổi sau mỗi").to_string();
                let mut ch = false;
                row(ui, &lbl, "", |ui| {
                    ch = ui.add(egui::Slider::new(&mut m, 1..=240).suffix(" min")).changed();
                });
                if ch {
                    self.set("pro_theme_interval", &m.to_string());
                }
            }
        });
    }

    /// One-click combinations of a layout and a palette.
    fn quick_looks(&mut self, ui: &mut egui::Ui) {
        let cur_l = self.get_or("pro_layout", "board");
        let cur_t = self.get_or("pro_theme", "mineral");
        let n = LOOKS.len();
        let w = ui.available_width();
        let cw = ((w - 10.0 * (n as f32 - 1.0)) / n as f32).floor();
        let mut picked = None;
        ui.label(RichText::new(self.t("Quick looks", "Phong cách nhanh")).font(font(13.0, Weight::Semibold)).color(MUTED));
        ui.add_space(-2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for (i, (en, vi, layout, theme)) in LOOKS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, 58.0), Sense::click());
                let on = cur_l == *layout && cur_t == *theme && !self.bool_of("pro_custom_enabled", false);
                let th = THEMES.iter().find(|t| t.key == *theme);
                let bg = th.map_or(CARD, |t| lerp_color(rgb(t.bg), CARD, 0.5));
                let p = ui.painter();
                p.rect(rect, Rounding::same(12.0), if resp.hovered() { CARD_HI } else { bg }, Stroke::new(if on { 1.5_f32 } else { 1.0_f32 }, if on { ACCENT } else { LINE }));
                if let Some(t) = th {
                    for (j, c) in t.colors.iter().take(2).enumerate() {
                        p.circle_filled(Pos2::new(rect.left() + 20.0 + j as f32 * 11.0, rect.center().y), 7.0, rgb(*c));
                    }
                }
                let name = if self.lang == Lang::En { *en } else { *vi };
                let lname = LAYOUTS.iter().find(|l| l.key == *layout).map_or(*layout, |l| l.name);
                let tname = th.map_or(*theme, |t| t.name);
                p.text(Pos2::new(rect.left() + 46.0, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, name, font(14.0, Weight::Semibold), TEXT);
                p.text(Pos2::new(rect.left() + 46.0, rect.center().y + 10.0), egui::Align2::LEFT_CENTER, format!("{lname} · {tname}"), font(12.0, Weight::Regular), MUTED);
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        });
        if let Some(i) = picked {
            let (_, _, layout, theme) = LOOKS[i];
            self.use_layout(layout);
            self.set("pro_theme", theme);
        }
    }

    /// Thumbnail cards; returns the clicked layout.
    fn grid(&self, ui: &mut egui::Ui, items: &[&LayoutInfo], current: &str, cols: usize) -> Option<String> {
        let mut picked = None;
        let w = ui.available_width();
        let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        for chunk in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for l in chunk {
                    let img_h = if l.portrait() { (cw - 16.0) * 1920.0 / 462.0 * 0.55 } else { ((cw - 16.0) * 462.0 / 1920.0).round() };
                    let size = Vec2::new(cw, img_h + 78.0);
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                    let on = l.key == current;
                    let p = ui.painter();
                    p.rect(rect, Rounding::same(R), if resp.hovered() { CARD_HI } else { CARD }, Stroke::new(if on { 2.0_f32 } else { 1.0 }, if on { ACCENT } else { LINE }));
                    let ir = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(cw - 16.0, img_h));
                    p.rect_filled(ir, Rounding::same(9.0), Color32::BLACK);
                    if let Some(tex) = self.thumbs.get(l.key) {
                        let ts = tex.size_vec2();
                        let s = (ir.width() / ts.x).min(ir.height() / ts.y);
                        let r2 = Rect::from_center_size(ir.center(), ts * s);
                        egui::Image::new((tex.id(), r2.size())).rounding(Rounding::same(8.0)).paint_at(ui, r2);
                    } else {
                        p.text(ir.center(), egui::Align2::CENTER_CENTER, "…", font(18.0, Weight::Regular), DIM);
                    }
                    let ty = ir.bottom() + 14.0;
                    p.text(Pos2::new(rect.left() + 16.0, ty), egui::Align2::LEFT_TOP, l.name, font(15.0, Weight::Semibold), TEXT);
                    let d = if self.lang == Lang::En { l.en } else { l.vi };
                    let galley = p.layout(d.to_string(), font(12.5, Weight::Regular), MUTED, cw - 60.0);
                    p.galley(Pos2::new(rect.left() + 16.0, ty + 22.0), galley, MUTED);
                    if on {
                        let c = Pos2::new(rect.right() - 24.0, ty + 10.0);
                        p.circle_filled(c, 11.0, ACCENT);
                        p.line_segment([c + Vec2::new(-5.0, 0.0), c + Vec2::new(-1.2, 3.8)], Stroke::new(2.2_f32, Color32::WHITE));
                        p.line_segment([c + Vec2::new(-1.2, 3.8), c + Vec2::new(5.2, -3.8)], Stroke::new(2.2_f32, Color32::WHITE));
                    } else if l.group == Group::New {
                        badge(p, Pos2::new(rect.right() - 14.0, ty - 2.0), if self.lang == Lang::En { "NEW" } else { "MỚI" }, ACCENT_SOFT, ACCENT);
                    }
                    if resp.clicked() {
                        picked = Some(l.key.to_string());
                    }
                }
            });
            ui.add_space(GAP - 10.0);
        }
        picked
    }

    fn page_colors(&mut self, ui: &mut egui::Ui) {
        let theme = self.get_or("pro_theme", "mineral").to_ascii_lowercase();
        let is_custom = theme.starts_with('#');
        for (title, sub, soft) in [
            (self.t("Soft palettes", "Bộ màu dịu"), self.t("Quiet surfaces and one accent. Made for the Aperture and New layouts.", "Nền dịu, một màu nhấn. Hợp với bố cục Aperture và Mới."), true),
            (self.t("Vivid palettes", "Bộ màu rực rỡ"), self.t("A color per section on black. Made for the Signature and Classic layouts.", "Mỗi mục một màu trên nền đen. Hợp với bố cục Đặc trưng và Cổ điển."), false),
        ] {
            section_title(ui, title, sub);
            let items: Vec<&ThemeInfo> = THEMES.iter().filter(|t| t.soft == soft).collect();
            if let Some(k) = self.palette_grid(ui, &items, &theme) {
                self.set("pro_theme", &k);
            }
        }
        section_title(ui, self.t("Your color", "Màu của bạn"), self.t("One color, and a whole matching palette is built from it.", "Chọn một màu, cả bộ màu phù hợp được tạo từ màu đó."));
        card(ui, |ui| {
            let mut c = self.custom;
            let lbl = self.t("Use my own color", "Dùng màu của tôi").to_string();
            let hex = format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
            let mut on = is_custom;
            let mut toggled = false;
            row(ui, &lbl, &hex, |ui| {
                toggled = toggle(ui, &mut on).changed();
                ui.add_space(8.0);
                egui::color_picker::color_edit_button_srgb(ui, &mut c);
            });
            if c != self.custom {
                self.custom = c;
                on = true;
                toggled = true;
            }
            if toggled {
                if on {
                    let hex = format!("#{:02x}{:02x}{:02x}", self.custom[0], self.custom[1], self.custom[2]);
                    self.set("pro_theme", &hex);
                } else {
                    self.set("pro_theme", "mineral");
                }
            }
            divider(ui);
            let mut rot = theme == "all";
            let lbl = self.t("Rotate the soft palettes", "Tự đổi qua các bộ màu dịu").to_string();
            let mut ch = false;
            row(ui, &lbl, "", |ui| ch = toggle(ui, &mut rot).changed());
            if ch {
                self.set("pro_theme", if rot { "all" } else { "mineral" });
            }
        });
        section_title(ui, self.t("Details", "Chi tiết"), "");
        card(ui, |ui| {
            let mut on = self.bool_of("pro_cover_color", true);
            let lbl = self.t("Colors follow the album cover", "Màu theo ảnh bìa bài hát").to_string();
            let help = self.t("While music plays, accents take the colors of the cover art", "Khi phát nhạc, màu nhấn lấy theo màu ảnh bìa").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                self.set_bool("pro_cover_color", on);
            }
            divider(ui);
            let style = self.get_or("pro_style", "cards");
            let (cards, glass, flat) = (self.t("Cards", "Ô bo góc"), self.t("Glass", "Kính mờ"), self.t("Flat", "Phẳng"));
            let lbl = self.t("Panel style", "Kiểu ô").to_string();
            let help = self.t("Used by Classic, Cockpit and Focus", "Áp dụng cho Classic, Cockpit và Focus").to_string();
            let mut pick = None;
            row(ui, &lbl, &help, |ui| pick = segmented(ui, &style, &[("flat", flat), ("glass", glass), ("cards", cards)]));
            if let Some(v) = pick {
                self.set("pro_style", &v);
            }
        });
    }

    fn palette_grid(&self, ui: &mut egui::Ui, items: &[&ThemeInfo], current: &str) -> Option<String> {
        let mut picked = None;
        let cols = if ui.available_width() > 1000.0 { 6 } else { 3 };
        let w = ui.available_width();
        let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        for chunk in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for t in chunk {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, 112.0), Sense::click());
                    let on = t.key == current;
                    let p = ui.painter();
                    p.rect(rect, Rounding::same(R), if resp.hovered() { CARD_HI } else { CARD }, Stroke::new(if on { 2.0_f32 } else { 1.0 }, if on { ACCENT } else { LINE }));
                    // a tiny display in the palette's colors
                    let pr = Rect::from_min_size(rect.min + Vec2::new(10.0, 10.0), Vec2::new(cw - 20.0, 56.0));
                    p.rect_filled(pr, Rounding::same(8.0), rgb(t.bg));
                    let bw = (pr.width() - 32.0) / 3.0;
                    for (i, c) in t.colors.iter().enumerate() {
                        let col = if t.soft && i > 0 { lerp_color(rgb(t.bg), rgb(t.colors[0]), 0.18 + 0.12 * i as f32) } else { rgb(*c) };
                        let x = pr.left() + 8.0 + i as f32 * (bw + 8.0);
                        p.rect_filled(Rect::from_min_size(Pos2::new(x, pr.top() + 10.0), Vec2::new(bw, 8.0)), Rounding::same(4.0), col);
                        p.rect_filled(Rect::from_min_size(Pos2::new(x, pr.top() + 26.0), Vec2::new(bw * 0.6, 18.0)), Rounding::same(4.0), if t.soft { rgb(t.colors[0]) } else { rgb(*c) }.gamma_multiply(if i == 0 { 1.0 } else { 0.55 }));
                    }
                    p.text(Pos2::new(rect.left() + 14.0, rect.bottom() - 32.0), egui::Align2::LEFT_TOP, t.name, font(14.0, Weight::Semibold), TEXT);
                    if on {
                        let c = Pos2::new(rect.right() - 20.0, rect.bottom() - 23.0);
                        p.circle_filled(c, 9.0, ACCENT);
                        p.line_segment([c + Vec2::new(-4.0, 0.0), c + Vec2::new(-1.0, 3.0)], Stroke::new(2.0_f32, Color32::WHITE));
                        p.line_segment([c + Vec2::new(-1.0, 3.0), c + Vec2::new(4.2, -3.0)], Stroke::new(2.0_f32, Color32::WHITE));
                    }
                    if resp.clicked() {
                        picked = Some(t.key.to_string());
                    }
                }
            });
            ui.add_space(GAP - 10.0);
        }
        picked
    }

    fn page_display(&mut self, ui: &mut egui::Ui) {
        section_title(ui, self.t("Screen", "Màn hình"), "");
        card(ui, |ui| {
            let mut flip = self.bool_of("flip", false);
            let lbl = self.t("Picture upside down? Turn it 180°", "Hình bị ngược? Xoay 180°").to_string();
            let help = self.t("Use this when the display is mounted the other way round", "Dùng khi màn hình được gắn ngược chiều").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut flip).changed());
            if ch {
                self.set_bool("flip", flip);
            }
            divider(ui);
            let mut b: u32 = self.get("brightness").and_then(|v| v.trim().parse().ok()).unwrap_or(100).min(100);
            let lbl = self.t("Brightness", "Độ sáng").to_string();
            let mut ch = false;
            row(ui, &lbl, "", |ui| ch = ui.add(egui::Slider::new(&mut b, 5..=100).suffix(" %")).changed());
            if ch {
                self.set("brightness", &b.to_string());
            }
            divider(ui);
            let rot = self.get_or("pro_rotate", "cw");
            let lbl = self.t("Portrait turn direction", "Chiều xoay khi dựng đứng").to_string();
            let help = self.t("Only for portrait layouts. Upside down? Pick the other one.", "Chỉ dùng cho bố cục dọc. Bị ngược thì chọn bên còn lại.").to_string();
            let (a, c) = (self.t("Clockwise", "Xoay phải"), self.t("Counter-clockwise", "Xoay trái"));
            let mut pick = None;
            row(ui, &lbl, &help, |ui| pick = segmented(ui, &rot, &[("ccw", c), ("cw", a)]));
            if let Some(v) = pick {
                self.set("pro_rotate", &v);
            }
        });
        section_title(ui, self.t("Clock & weather", "Đồng hồ & thời tiết"), "");
        card(ui, |ui| {
            let clock = if self.get_or("pro_clock", "24h").starts_with("12") { "12h" } else { "24h" };
            let lbl = self.t("Clock format", "Kiểu đồng hồ").to_string();
            let mut pick = None;
            row(ui, &lbl, "", |ui| pick = segmented(ui, clock, &[("12h", "12h  (2:30 PM)"), ("24h", "24h  (14:30)")]));
            if let Some(v) = pick {
                self.set("pro_clock", &v);
            }
            divider(ui);
            let unit = self.get_or("weather_unit", "c").to_ascii_lowercase();
            let lbl = self.t("Temperature unit (weather)", "Đơn vị nhiệt độ (thời tiết)").to_string();
            let mut pick = None;
            row(ui, &lbl, "", |ui| pick = segmented(ui, &unit, &[("f", "°F"), ("c", "°C")]));
            if let Some(v) = pick {
                self.set("weather_unit", &v);
            }
            divider(ui);
            let lang = self.get_or("language", "en");
            let lbl = self.t("Language on the display", "Ngôn ngữ trên màn hình").to_string();
            let help = self.t("Weekdays, months and weather words", "Thứ, tháng và mô tả thời tiết").to_string();
            let mut pick = None;
            row(ui, &lbl, &help, |ui| pick = segmented(ui, &lang, &[("it", "Italiano"), ("vi", "Tiếng Việt"), ("en", "English")]));
            if let Some(v) = pick {
                self.set("language", &v);
            }
            divider(ui);
            let lbl = self.t("City for the weather", "Thành phố cho thời tiết").to_string();
            let help = self.t("“City, Country” or “21.03, 105.85”. Empty = found from your internet connection.", "“Thành phố, Quốc gia” hoặc “21.03, 105.85”. Để trống = tự đoán theo mạng.").to_string();
            let apply = self.t("Apply", "Áp dụng").to_string();
            let mut do_apply = false;
            row(ui, &lbl, &help, |ui| {
                if ui.button(&apply).clicked() {
                    do_apply = true;
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.city).desired_width(240.0).hint_text("Hanoi, Vietnam"));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_apply = true;
                }
            });
            if do_apply {
                let c = self.city.trim().to_string();
                if c.is_empty() {
                    self.unset("weather_city");
                } else {
                    self.set("weather_city", &c);
                }
                self.restart_needed = true;
            }
        });
        if self.restart_needed {
            ui.add_space(10.0);
            self.restart_banner(ui, self.t("The new city is used after the display program restarts.", "Thành phố mới được dùng sau khi khởi động lại chương trình màn hình."));
        }
    }

    fn restart_banner(&mut self, ui: &mut egui::Ui, text: &str) {
        let text = text.to_string();
        egui::Frame::none().fill(Color32::from_rgb(40, 33, 14)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(96, 74, 20))).rounding(R).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(TEXT));
                ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                    if ui.button(self.t("Restart display", "Khởi động lại màn hình")).clicked() {
                        self.restart_display();
                    }
                });
            });
        });
    }

    fn layout_combo(&self, ui: &mut egui::Ui, id: &str, current: &str) -> Option<String> {
        let mut picked = None;
        let name = LAYOUTS.iter().find(|l| l.key == current).map(|l| l.name).unwrap_or(current);
        egui::ComboBox::from_id_salt(id).selected_text(name).width(220.0).height(420.0).show_ui(ui, |ui| {
            for l in LAYOUTS.iter().filter(|l| !l.portrait()) {
                if ui.selectable_label(l.key == current, l.name).clicked() {
                    picked = Some(l.key.to_string());
                }
            }
        });
        picked
    }

    fn theme_combo(&self, ui: &mut egui::Ui, id: &str, current: &str) -> Option<String> {
        let mut picked = None;
        let same = self.t("Same as normal", "Giống bình thường").to_string();
        let name = THEMES.iter().find(|t| t.key == current).map(|t| t.name.to_string()).unwrap_or(if current.is_empty() { same.clone() } else { current.to_string() });
        egui::ComboBox::from_id_salt(id).selected_text(name).width(220.0).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_empty(), &same).clicked() {
                picked = Some(String::new());
            }
            for t in THEMES {
                if ui.selectable_label(t.key == current, t.name).clicked() {
                    picked = Some(t.key.to_string());
                }
            }
        });
        picked
    }

    fn page_features(&mut self, ui: &mut egui::Ui) {
        section_title(ui, self.t("Music", "Nhạc"), "");
        card(ui, |ui| {
            let mut on = self.bool_of("pro_song_popup", true);
            let lbl = self.t("New song pop-up", "Thông báo khi đổi bài").to_string();
            let help = self.t("Shows the cover, title and artist for 3 seconds", "Hiện ảnh bìa, tên bài và ca sĩ trong 3 giây").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                self.set_bool("pro_song_popup", on);
            }
        });
        section_title(ui, self.t("Game FPS", "FPS khi chơi game"), self.t("Measured from the game's own frames, for any GPU (DirectX 9-12, Vulkan, OpenGL).", "Đo từ chính khung hình của game, mọi card đồ họa (DirectX 9-12, Vulkan, OpenGL)."));
        card(ui, |ui| {
            let mut on = self.bool_of("fps_monitor", true);
            let lbl = self.t("Measure game FPS", "Đo FPS trong game").to_string();
            let help = self.t("Applies after the display program restarts", "Có hiệu lực sau khi khởi động lại chương trình màn hình").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                self.set_bool("fps_monitor", on);
                self.restart_needed = true;
            }
            divider(ui);
            let lbl = self.t("Never show FPS for", "Không đo FPS của").to_string();
            let help = self.t("Browsers, launchers, Discord and the desktop are already skipped. Add more: name.exe, other.exe", "Trình duyệt, launcher, Discord và desktop đã được bỏ qua. Thêm: ten.exe, khac.exe").to_string();
            let apply = self.t("Apply", "Áp dụng").to_string();
            let mut do_apply = false;
            row(ui, &lbl, &help, |ui| {
                if ui.button(&apply).clicked() {
                    do_apply = true;
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.ignore).desired_width(240.0).hint_text("obs64.exe"));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    do_apply = true;
                }
            });
            if do_apply {
                let v = self.ignore.trim().to_string();
                if v.is_empty() {
                    self.unset("fps_ignore");
                } else {
                    self.set("fps_ignore", &v);
                }
                self.restart_needed = true;
            }
        });
        if self.restart_needed {
            ui.add_space(10.0);
            self.restart_banner(ui, self.t("Some changes apply after the display program restarts.", "Một số thay đổi có hiệu lực sau khi khởi động lại chương trình màn hình."));
        }
        section_title(ui, self.t("Game mode", "Chế độ chơi game"), self.t("Switches layout while a game shows FPS, and back when you quit.", "Tự đổi bố cục khi đang chơi game, thoát game thì tự trở lại."));
        card(ui, |ui| {
            let cur = self.get("pro_game_layout").unwrap_or_default();
            let mut on = !cur.trim().is_empty();
            let lbl = self.t("Turn on game mode", "Bật chế độ chơi game").to_string();
            let mut ch = false;
            row(ui, &lbl, "", |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                if on {
                    self.set("pro_game_layout", "focus");
                } else {
                    self.unset("pro_game_layout");
                    self.unset("pro_game_theme");
                }
            }
            if on {
                divider(ui);
                let lbl = self.t("Layout while gaming", "Bố cục khi chơi game").to_string();
                let mut pick = None;
                row(ui, &lbl, "", |ui| pick = self.layout_combo(ui, "game_layout", &cur));
                if let Some(v) = pick {
                    self.set("pro_game_layout", &v);
                }
                divider(ui);
                let gt = self.get("pro_game_theme").unwrap_or_default();
                let lbl = self.t("Colors while gaming", "Màu khi chơi game").to_string();
                let mut pick = None;
                row(ui, &lbl, "", |ui| pick = self.theme_combo(ui, "game_theme", &gt));
                match pick.as_deref() {
                    Some("") => self.unset("pro_game_theme"),
                    Some(v) => self.set("pro_game_theme", v),
                    None => {}
                }
            }
        });
        section_title(
            ui,
            self.t("Game artwork & Showcase", "Ảnh game & Showcase"),
            self.t(
                "Shows the running game's title, logo or cover in the music slot, and lets you customize the Showcase poster background.",
                "Hiển thị tên, logo hoặc ảnh bìa game trong ô nhạc và chọn poster nền cho Showcase.",
            ),
        );
        card(ui, |ui| {
            let mut game_media = self.bool_of("pro_game_media", true);
            let lbl = self.t("Show running game in music slot", "Hiển thị game trong ô nhạc").to_string();
            let help = self.t("Replaces music info with the active game's logo/cover and title.", "Thay ô nhạc bằng logo/bìa và tên game đang chạy.").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut game_media).changed());
            if ch {
                self.set("pro_game_media", if game_media { "true" } else { "false" });
            }

            if game_media {
                divider(ui);
                let art_mode = self.get_or("pro_game_art", "logo");
                let lbl = self.t("Music slot artwork", "Kiểu ảnh ô nhạc").to_string();
                let is_cover = art_mode.eq_ignore_ascii_case("cover") || art_mode.eq_ignore_ascii_case("poster");
                row(ui, &lbl, "", |ui| {
                    if ui.selectable_label(is_cover, self.t("Cover / Poster", "Ảnh bìa")).clicked() {
                        self.set("pro_game_art", "cover");
                    }
                    if ui.selectable_label(!is_cover, self.t("Transparent logo", "Logo nền trong")).clicked() {
                        self.set("pro_game_art", "logo");
                    }
                });
            }

            divider(ui);
            let mut game_art_api = self.bool_of("game_art", true);
            let lbl = self.t("Look up game artwork automatically", "Tự động tìm tên và ảnh game").to_string();
            let help = self.t("Uses local Steam/Epic manifests and Steam Store / CDN (no key needed).", "Đọc Steam/Epic cục bộ và Steam Store / CDN (không cần key).").to_string();
            let mut ch_api = false;
            row(ui, &lbl, &help, |ui| ch_api = toggle(ui, &mut game_art_api).changed());
            if ch_api {
                self.set("game_art", if game_art_api { "true" } else { "false" });
            }

            divider(ui);
            let lbl = self.t("SteamGridDB API key (optional)", "API key SteamGridDB (tùy chọn)").to_string();
            let help = self.t("Improves logos and posters for non-Steam or indie games.", "Bổ sung logo/poster cho game ngoài Steam hoặc game ít phổ biến.").to_string();
            let mut apply_sgdb = false;
            row(ui, &lbl, &help, |ui| {
                let stored = self.get("steamgriddb_key").unwrap_or_default();
                if ui.button(self.t("Apply", "Áp dụng")).clicked() {
                    apply_sgdb = true;
                }
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.sgdb_key)
                        .desired_width(200.0)
                        .password(true)
                        .hint_text("Bearer API key"),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    apply_sgdb = true;
                }
                if !stored.is_empty() && self.sgdb_key.is_empty() {
                    self.unset("steamgriddb_key");
                }
            });
            if apply_sgdb {
                let k = self.sgdb_key.trim().to_string();
                if k.is_empty() {
                    self.unset("steamgriddb_key");
                } else {
                    self.set("steamgriddb_key", &k);
                }
            }

            divider(ui);
            let poster = self.get_or("pro_game_poster", "");
            let has_poster = !poster.trim().is_empty();
            let lbl = self.t("Custom poster image for Showcase", "Ảnh poster riêng cho Showcase").to_string();
            let help = if has_poster {
                poster.clone()
            } else {
                self.t("Optional: overrides the auto-fetched game poster in the Showcase layout.", "Tùy chọn: thay ảnh poster tự động của Showcase bằng ảnh bạn chọn.").to_string()
            };
            let mut choose = false;
            let mut clear = false;
            row(ui, &lbl, &help, |ui| {
                if has_poster {
                    clear = ui.button(self.t("Remove", "Xóa ảnh")).clicked();
                }
                choose = ui.button(self.t("Choose image…", "Chọn ảnh…")).clicked();
            });
            if clear {
                self.unset("pro_game_poster");
            }
            if choose {
                if let Some(path) = custom_editor::pick_file("Poster image", &["png", "jpg", "jpeg", "bmp"]) {
                    let result = (|| -> std::io::Result<String> {
                        let dir = self.dir.join("assets").join("posters");
                        std::fs::create_dir_all(&dir)?;
                        let suffix = path.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                        let name = format!("poster-{stamp}.{suffix}");
                        std::fs::copy(&path, dir.join(&name))?;
                        Ok(format!("assets/posters/{name}"))
                    })();
                    match result {
                        Ok(p) => self.set("pro_game_poster", &p),
                        Err(e) => self.save_error = Some(e.to_string()),
                    }
                }
            }

            divider(ui);
            let mut dim = self.get("pro_game_bg_dim").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(45).min(100);
            let lbl = self.t("Darken Showcase poster", "Làm tối poster Showcase").to_string();
            let mut ch_dim = false;
            row(ui, &lbl, "", |ui| ch_dim = ui.add(egui::Slider::new(&mut dim, 0..=100).suffix(" %")).changed());
            if ch_dim {
                self.set("pro_game_bg_dim", &dim.to_string());
            }
        });
        section_title(ui, self.t("Night mode", "Chế độ ban đêm"), self.t("Dims the display (and can change colors) during the hours you choose.", "Giảm độ sáng (và có thể đổi màu) trong khung giờ bạn chọn."));
        card(ui, |ui| {
            let night = self.get("pro_night").unwrap_or_default();
            let mut on = !night.trim().is_empty();
            let lbl = self.t("Turn on night mode", "Bật chế độ ban đêm").to_string();
            let mut ch = false;
            row(ui, &lbl, "", |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                if on {
                    self.set("pro_night", "22:00-07:00");
                } else {
                    self.unset("pro_night");
                    self.unset("pro_night_theme");
                }
            }
            if on {
                let (mut a, mut b) = parse_range(&night).unwrap_or((22 * 60, 7 * 60));
                divider(ui);
                let lbl = self.t("From – to", "Từ – đến").to_string();
                let mut changed = false;
                row(ui, &lbl, "", |ui| {
                    changed |= time_edit(ui, "night_b", &mut b);
                    ui.label(RichText::new("–").color(MUTED));
                    changed |= time_edit(ui, "night_a", &mut a);
                });
                if changed {
                    self.set("pro_night", &format!("{:02}:{:02}-{:02}:{:02}", a / 60, a % 60, b / 60, b % 60));
                }
                divider(ui);
                let mut dim: u32 = self.get("pro_night_dim").and_then(|v| v.trim().parse().ok()).unwrap_or(40).clamp(5, 100);
                let lbl = self.t("Brightness at night", "Độ sáng ban đêm").to_string();
                let mut ch = false;
                row(ui, &lbl, "", |ui| ch = ui.add(egui::Slider::new(&mut dim, 5..=100).suffix(" %")).changed());
                if ch {
                    self.set("pro_night_dim", &dim.to_string());
                }
                divider(ui);
                let nt = self.get("pro_night_theme").unwrap_or_default();
                let lbl = self.t("Colors at night", "Màu ban đêm").to_string();
                let mut pick = None;
                row(ui, &lbl, "", |ui| pick = self.theme_combo(ui, "night_theme", &nt));
                match pick.as_deref() {
                    Some("") => self.unset("pro_night_theme"),
                    Some(v) => self.set("pro_night_theme", v),
                    None => {}
                }
            }
        });
        section_title(ui, self.t("Heat alert", "Cảnh báo nóng"), self.t("A red frame pulses on the display when a temperature goes too high.", "Viền đỏ nhấp nháy trên màn hình khi nhiệt độ quá cao."));
        card(ui, |ui| {
            let cpu: u32 = self.get("pro_alert_cpu").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            let gpu: u32 = self.get("pro_alert_gpu").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
            let mut on = cpu > 0 || gpu > 0;
            let lbl = self.t("Turn on heat alert", "Bật cảnh báo nóng").to_string();
            let mut ch = false;
            row(ui, &lbl, "", |ui| ch = toggle(ui, &mut on).changed());
            if ch {
                self.set("pro_alert_cpu", if on { "90" } else { "0" });
                self.set("pro_alert_gpu", if on { "85" } else { "0" });
            }
            if on {
                for (key, en, vi, val) in [("pro_alert_cpu", "CPU above", "CPU vượt quá", cpu), ("pro_alert_gpu", "GPU above", "GPU vượt quá", gpu)] {
                    divider(ui);
                    let mut v = if val == 0 { 85 } else { val };
                    let lbl = self.t(en, vi).to_string();
                    let mut ch = false;
                    row(ui, &lbl, "", |ui| ch = ui.add(egui::Slider::new(&mut v, 60..=105).suffix(" °C")).changed());
                    if ch {
                        self.set(key, &v.to_string());
                    }
                }
            }
        });
    }

    /// One live reading tile of the Sensors page.
    fn sensor_tile(&self, ui: &mut egui::Ui, w: f32, title: &str, value: Option<String>, unit: &str, source: (Color32, String)) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 118.0), Sense::hover());
        let p = ui.painter();
        p.rect(rect, Rounding::same(R), CARD, Stroke::new(1.0_f32, LINE));
        p.text(rect.min + Vec2::new(16.0, 14.0), egui::Align2::LEFT_TOP, title.to_uppercase(), font(11.5, Weight::Semibold), DIM);
        let (txt, col) = match &value {
            Some(v) => (v.clone(), TEXT),
            None => ("--".to_string(), DIM),
        };
        let g = p.layout_no_wrap(txt, font(34.0, Weight::Medium), col);
        let vw = g.size().x;
        p.galley(rect.min + Vec2::new(16.0, 34.0), g, col);
        if value.is_some() && !unit.is_empty() {
            p.text(rect.min + Vec2::new(16.0 + vw + 6.0, 70.0), egui::Align2::LEFT_BOTTOM, unit, font(14.0, Weight::Regular), MUTED);
        }
        let (c, s) = source;
        p.circle_filled(Pos2::new(rect.left() + 20.0, rect.bottom() - 21.0), 4.0, c);
        let s = if s.chars().count() > 34 { format!("{}…", s.chars().take(33).collect::<String>()) } else { s };
        p.text(Pos2::new(rect.left() + 31.0, rect.bottom() - 21.0), egui::Align2::LEFT_CENTER, s, font(12.5, Weight::Regular), MUTED);
    }

    fn source_text(&self, v: Option<&str>) -> (Color32, String) {
        let s = &self.status;
        let t = |en: &str, vi: &str| self.t(en, vi).to_string();
        if !s.alive {
            return (DIM, t("Display program not running", "Chương trình màn hình chưa chạy"));
        }
        match v {
            Some("builtin") => (GOOD, t("Built in", "Có sẵn")),
            Some("board") => (GOOD, format!("{} {}", t("Motherboard", "Bo mạch chủ"), s.get("board_chip").unwrap_or(""))),
            Some("hwinfo") => (GOOD, t("From HWiNFO", "Lấy từ HWiNFO")),
            Some("lhm") => (GOOD, t("From LibreHardwareMonitor", "Lấy từ LibreHardwareMonitor")),
            _ => (BAD, t("Not available", "Không có dữ liệu")),
        }
    }

    fn page_sensors(&mut self, ui: &mut egui::Ui) {
        let s = self.status.clone();
        // ---- live tiles
        let cols = if ui.available_width() > 1000.0 { 3 } else { 2 };
        let w = ui.available_width();
        let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        let live = |k: &str| if s.alive { s.get(k).map(str::to_string) } else { None };
        let fps_src = match s.get("fps_capture") {
            Some("needs_admin") => (BAD, self.t("Needs administrator rights", "Cần quyền quản trị").to_string()),
            Some("failed") => (BAD, self.t("Capture failed", "Không đo được").to_string()),
            Some("disabled") => (DIM, self.t("Turned off", "Đang tắt").to_string()),
            _ if !s.alive => (DIM, self.t("Display program not running", "Chương trình màn hình chưa chạy").to_string()),
            _ => match (s.get("fps_process"), s.get("fps_source")) {
                (Some(p), Some(api)) => (GOOD, format!("{p} · {api}")),
                (None, Some("GPU driver")) => (WARN, self.t("GPU driver (fullscreen only)", "Driver GPU (chỉ toàn màn hình)").to_string()),
                _ => (DIM, self.t("Waiting for a game", "Đang chờ game").to_string()),
            },
        };
        let tiles: Vec<(String, Option<String>, &str, (Color32, String))> = vec![
            (self.t("CPU temperature", "Nhiệt độ CPU").into(), live("cpu_temp_c"), "°C", self.source_text(s.get("cpu_temp"))),
            (self.t("CPU fan", "Quạt CPU").into(), live("cpu_fan_rpm"), "RPM", self.source_text(s.get("cpu_fan"))),
            (self.t("Game FPS", "FPS game").into(), live("fps"), "FPS", fps_src),
            (self.t("GPU temperature", "Nhiệt độ GPU").into(), live("gpu_temp_c"), "°C", if live("gpu_temp_c").is_some() { (GOOD, self.t("Built in", "Có sẵn").into()) } else { self.source_text(None) }),
            (self.t("GPU hotspot", "Hotspot GPU").into(), live("gpu_hotspot_c"), "°C", self.source_text(s.get("gpu_hotspot"))),
            (self.t("GPU fan", "Quạt GPU").into(), live("gpu_fan_rpm"), "RPM", if live("gpu_fan_rpm").is_some() { (GOOD, self.t("Built in", "Có sẵn").into()) } else { self.source_text(None) }),
        ];
        ui.add_space(6.0);
        for chunk in tiles.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (title, value, unit, src) in chunk {
                    self.sensor_tile(ui, cw, title, value.clone(), unit, src.clone());
                }
            });
            ui.add_space(GAP - 10.0);
        }

        // ---- Update interval & performance
        section_title(
            ui,
            self.t("Update interval & performance", "Tần suất cập nhật & hiệu năng"),
            self.t(
                "Increase these intervals to lower CPU usage and protect 1% low FPS in competitive games.",
                "Tăng các khoảng thời gian này để giảm tải CPU và giữ ổn định 1% low FPS khi chơi game.",
            ),
        );
        card(ui, |ui| {
            let mut ms: u32 = self
                .get("sensor_refresh_ms")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(500)
                .clamp(200, 10_000);
            let lbl = self.t("Sensor update interval", "Chu kỳ đọc cảm biến").to_string();
            let help = self.t(
                "How often CPU, RAM, GPU, disk and network readings are sampled (default: 500 ms).",
                "Tần suất lấy mẫu CPU, RAM, GPU, ổ đĩa và mạng (mặc định: 500 ms).",
            ).to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| {
                ch = ui.add(egui::Slider::new(&mut ms, 250..=5000).step_by(50.0).suffix(" ms")).changed();
            });
            if ch {
                self.set("sensor_refresh_ms", &ms.to_string());
            }

            divider(ui);
            let mut afps: u32 = self
                .get("active_fps")
                .and_then(|v| v.trim().parse::<f32>().ok())
                .map(|v| v.round() as u32)
                .unwrap_or(15)
                .clamp(1, 30);
            let lbl = self.t("Display refresh rate with audio", "Tần số màn hình khi có âm thanh").to_string();
            let help = self.t(
                "Rises to this rate whenever game/system sound plays (default: 15 FPS). Use 1–2 FPS for Showcase or layouts without spectrum bars.",
                "Tăng lên mức này khi có âm thanh (mặc định: 15 FPS). Dùng 1–2 FPS cho Showcase hoặc bố cục không có cột nhạc.",
            ).to_string();
            let mut ch_fps = false;
            row(ui, &lbl, &help, |ui| {
                ch_fps = ui.add(egui::Slider::new(&mut afps, 1..=30).suffix(" FPS")).changed();
            });
            if ch_fps {
                self.set("active_fps", &afps.to_string());
            }

            divider(ui);
            let mut etw_on = self.bool_of("fps_monitor", true);
            let lbl = self.t("Measure game FPS (ETW)", "Đo FPS game (ETW)").to_string();
            let help = self.t(
                "Disable if high-FPS games feel heavier; game artwork still works via GPU > 50%. Requires display restart.",
                "Tắt nếu game FPS cao bị nặng; ảnh game vẫn hoạt động qua GPU > 50%. Cần khởi động lại dịch vụ.",
            ).to_string();
            let mut ch_etw = false;
            row(ui, &lbl, &help, |ui| ch_etw = toggle(ui, &mut etw_on).changed());
            if ch_etw {
                self.set_bool("fps_monitor", etw_on);
                self.restart_needed = true;
            }
        });

        // ---- CPU fan
        section_title(ui, self.t("CPU fan", "Quạt CPU"), self.t("Read straight from the motherboard; HWiNFO or LibreHardwareMonitor are only needed on boards we cannot read.", "Đọc thẳng từ bo mạch chủ; chỉ cần HWiNFO hoặc LibreHardwareMonitor với bo mạch không đọc được."));
        card(ui, |ui| {
            let board = s.board_fans();
            let ext: Vec<(String, f64)> = s.ext.as_ref().map(|(_, snap)| snap.fans.iter().map(|f| (f.selector(), f.value)).collect()).unwrap_or_default();
            let cur = self.get("pro_fan_sensor").unwrap_or_default();
            let auto = self.t("Automatic", "Tự động").to_string();
            let lbl = self.t("Which fan is the CPU fan?", "Quạt nào là quạt CPU?").to_string();
            let help = self.t("Automatic picks the CPU_FAN header. With an AIO, choose the pump or the radiator fans.", "Tự động chọn cổng CPU_FAN. Dùng tản nước AIO thì chọn bơm hoặc quạt két.").to_string();
            let mut pick: Option<String> = None;
            row(ui, &lbl, &help, |ui| {
                let shown = if cur.trim().is_empty() { auto.clone() } else { cur.clone() };
                egui::ComboBox::from_id_salt("fan_pick").selected_text(shown).width(300.0).height(420.0).show_ui(ui, |ui| {
                    if ui.selectable_label(cur.trim().is_empty(), &auto).clicked() {
                        pick = Some(String::new());
                    }
                    for (sel, rpm) in &board {
                        if ui.selectable_label(*sel == cur, format!("{sel}  ·  {rpm} RPM")).clicked() {
                            pick = Some(sel.clone());
                        }
                    }
                    for (sel, rpm) in &ext {
                        if ui.selectable_label(*sel == cur, format!("{sel}  ·  {rpm:.0} RPM")).clicked() {
                            pick = Some(sel.clone());
                        }
                    }
                });
            });
            match pick.as_deref() {
                Some("") => self.unset("pro_fan_sensor"),
                Some(v) => self.set("pro_fan_sensor", v),
                None => {}
            }
            if !board.is_empty() {
                divider(ui);
                let lbl = format!("{} {}", self.t("Fan headers on", "Các cổng quạt trên"), s.get("board_chip").unwrap_or(""));
                ui.label(RichText::new(lbl).font(font(13.5, Weight::Medium)).color(TEXT));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    for (sel, rpm) in &board {
                        let name = sel.rsplit(" / ").next().unwrap_or(sel);
                        pill(ui, name, &format!("{rpm} RPM"), *rpm > 0);
                    }
                });
            } else if s.alive {
                divider(ui);
                if s.pawnio == Some(false) {
                    ui.label(RichText::new(self.t("Install the PawnIO driver below: it is what lets Aperture read the motherboard.", "Hãy cài driver PawnIO bên dưới: driver này giúp Aperture đọc được bo mạch chủ.")).color(MUTED));
                } else {
                    ui.label(RichText::new(self.t("This motherboard's chip could not be read. Run LibreHardwareMonitor (Options → Remote Web Server → Run) and the fans appear here.", "Không đọc được chip của bo mạch này. Hãy chạy LibreHardwareMonitor (Options → Remote Web Server → Run), các quạt sẽ hiện ở đây.")).color(MUTED));
                    if ui.link(self.t("Download LibreHardwareMonitor", "Tải LibreHardwareMonitor")).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases/latest"));
                    }
                }
            }
        });

        // ---- drivers and adapters
        section_title(ui, self.t("Drivers", "Driver"), "");
        card(ui, |ui| {
            let lbl = self.t("PawnIO driver", "Driver PawnIO").to_string();
            let help = self.t("Signed driver used to read CPU temperature, power and the motherboard fans", "Driver có chữ ký, dùng để đọc nhiệt độ, công suất CPU và quạt bo mạch").to_string();
            let installed = s.pawnio;
            let mut install = false;
            row(ui, &lbl, &help, |ui| match installed {
                Some(true) => status_dot(ui, GOOD, if self.lang == Lang::En { "Installed" } else { "Đã cài" }),
                Some(false) => {
                    install = ui.button(self.t("Install PawnIO", "Cài PawnIO")).clicked();
                    status_dot(ui, BAD, if self.lang == Lang::En { "Not installed" } else { "Chưa cài" });
                }
                None => status_dot(ui, DIM, "--"),
            });
            if install {
                run_elevated(PAWNIO_SCRIPT);
            }
            divider(ui);
            let lbl = self.t("External sensors", "Cảm biến ngoài").to_string();
            let help = self.t("HWiNFO or LibreHardwareMonitor, used when they run (optional)", "HWiNFO hoặc LibreHardwareMonitor, dùng khi đang chạy (không bắt buộc)").to_string();
            let ext = match s.ext.as_ref().map(|(src, _)| *src) {
                Some(Source::Hwinfo) => (GOOD, "HWiNFO".to_string()),
                Some(Source::Lhm) => (GOOD, "LibreHardwareMonitor".to_string()),
                None => (DIM, self.t("None running", "Không có").to_string()),
            };
            row(ui, &lbl, &help, |ui| status_dot(ui, ext.0, &ext.1));
            divider(ui);
            let lbl = self.t("AMD graphics card", "Card đồ họa AMD").to_string();
            let help = self.t("With two AMD GPUs (e.g. an iGPU), type part of the right name, like 9070 XT. Empty = automatic.", "Có hai GPU AMD (vd. iGPU) thì gõ một phần tên đúng, như 9070 XT. Để trống = tự động.").to_string();
            let apply = self.t("Apply", "Áp dụng").to_string();
            let mut do_apply = false;
            row(ui, &lbl, &help, |ui| {
                if ui.button(&apply).clicked() {
                    do_apply = true;
                }
                ui.add(egui::TextEdit::singleline(&mut self.adapter).desired_width(200.0).hint_text("9070 XT"));
            });
            if do_apply {
                let v = self.adapter.trim().to_string();
                if v.is_empty() {
                    self.unset("pro_amd_adapter");
                } else {
                    self.set("pro_amd_adapter", &v);
                }
                self.restart_needed = true;
            }
        });
        if self.restart_needed {
            ui.add_space(10.0);
            self.restart_banner(ui, self.t("Some changes apply after the display program restarts.", "Một số thay đổi có hiệu lực sau khi khởi động lại chương trình màn hình."));
        }

        // ---- display help
        section_title(ui, self.t("Nothing on the screen?", "Màn hình không hiện gì?"), "");
        card(ui, |ui| {
            ui.add_space(4.0);
            bullet(ui, self.t("Close Thermalright TRCC and remove it from Windows startup: two programs cannot drive the display.", "Tắt phần mềm TRCC của Thermalright và gỡ khỏi khởi động cùng Windows: hai chương trình không thể dùng màn hình cùng lúc."));
            bullet(ui, self.t("Check the USB cable (a motherboard USB header or a rear port).", "Kiểm tra dây USB (cắm vào header USB trên mainboard hoặc cổng sau)."));
            bullet(ui, self.t("Only the Trofeo Vision 9.16″ (1920 × 462) is supported; the 6.86″ model is not.", "Chỉ hỗ trợ Trofeo Vision 9.16″ (1920 × 462); bản 6.86″ không được hỗ trợ."));
            bullet(ui, self.t("Picture upside down? Display → “Turn it 180°”.", "Hình bị ngược? Vào Hiển thị → “Xoay 180°”."));
            ui.add_space(4.0);
            divider(ui);
            let lbl = self.t("Settings file", "Tệp cài đặt").to_string();
            let p = self.path.display().to_string();
            let version = s.get("version").map(|v| format!("  ·  {} {v}", self.t("display program", "chương trình màn hình"))).unwrap_or_default();
            row(ui, &lbl, &format!("{p}{version}"), |ui| {
                if ui.button(self.t("Open folder", "Mở thư mục")).clicked() {
                    #[cfg(windows)]
                    {
                        let _ = std::process::Command::new("explorer").arg(&self.dir).spawn();
                    }
                }
            });
            divider(ui);
            let lbl = self.t("Diagnostics", "Chẩn đoán").to_string();
            let help = self.t("Copies the state of the display program and the sensors, to paste in a bug report", "Sao chép trạng thái chương trình và cảm biến, để dán vào báo lỗi").to_string();
            let mut copy = false;
            row(ui, &lbl, &help, |ui| copy = ui.button(self.t("Copy", "Sao chép")).clicked());
            if copy {
                ui.ctx().copy_text(self.diagnostics());
                self.note = Some((self.t("Diagnostics copied", "Đã sao chép thông tin chẩn đoán").into(), Instant::now()));
            }
        });

        // ---- updates
        section_title(ui, self.t("Updates", "Cập nhật"), "");
        card(ui, |ui| {
            let lbl = format!("Aperture {}", env!("CARGO_PKG_VERSION"));
            let help = match (&self.update_state, &self.release) {
                (UpdateState::Checking, _) => self.t("Checking GitHub…", "Đang kiểm tra GitHub…").to_string(),
                (_, Some(r)) => format!("{} {}", self.t("Available:", "Có bản mới:"), r.tag),
                (UpdateState::UpToDate, _) => self.t("You have the latest version", "Bạn đang dùng bản mới nhất").to_string(),
                (UpdateState::Failed(e), _) => format!("{} {e}", self.t("Could not check:", "Không kiểm tra được:")),
                _ => self.t("Updates come from github.com/HuyBruh/Aperture", "Bản cập nhật lấy từ github.com/HuyBruh/Aperture").to_string(),
            };
            let mut check = false;
            let mut install = false;
            let has = self.release.is_some();
            row(ui, &lbl, &help, |ui| {
                if has {
                    install = ui.button(self.t("Update now", "Cập nhật ngay")).clicked();
                } else {
                    check = ui.button(self.t("Check now", "Kiểm tra ngay")).clicked();
                }
            });
            if check {
                self.check_updates(ui.ctx());
            }
            if install {
                self.update_hidden = false;
                self.start_update(ui.ctx());
            }
            divider(ui);
            let mut auto = self.bool_of("update_check", true);
            let lbl = self.t("Check when Aperture opens", "Kiểm tra khi mở Aperture").to_string();
            let help = self.t("Only asks GitHub for the latest version; nothing is installed without your click", "Chỉ hỏi GitHub phiên bản mới nhất; không cài gì nếu bạn chưa bấm").to_string();
            let mut ch = false;
            row(ui, &lbl, &help, |ui| ch = toggle(ui, &mut auto).changed());
            if ch {
                self.set_bool("update_check", auto);
            }
        });
    }
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once('-')?;
    let one = |x: &str| -> Option<u32> {
        let (h, m) = x.trim().split_once(':')?;
        Some(h.parse::<u32>().ok()?.min(23) * 60 + m.parse::<u32>().ok()?.min(59))
    };
    Some((one(a)?, one(b)?))
}

/// hh:mm editor (drag or type), in minutes after midnight.
fn time_edit(ui: &mut egui::Ui, id: &str, minutes: &mut u32) -> bool {
    let mut h = *minutes / 60;
    let mut m = *minutes % 60;
    let mut changed = false;
    ui.push_id(id, |ui| {
        changed |= ui.add(egui::DragValue::new(&mut m).range(0..=59).custom_formatter(|v, _| format!("{:02}", v as u32))).changed();
        ui.label(":");
        changed |= ui.add(egui::DragValue::new(&mut h).range(0..=23).custom_formatter(|v, _| format!("{:02}", v as u32))).changed();
    });
    if changed {
        *minutes = h * 60 + m;
    }
    changed
}

impl Drop for App {
    fn drop(&mut self) {
        if self.live {
            let _ = std::fs::remove_file(self.dir.join("trofeo-live.request"));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump(ctx);
        egui::SidePanel::left("nav")
            .exact_width(236.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::none().fill(SIDEBAR).inner_margin(egui::Margin::symmetric(16.0, 18.0)))
            .show(ctx, |ui| self.sidebar(ui));
        egui::CentralPanel::default().frame(egui::Frame::none().fill(BG).inner_margin(egui::Margin { left: 30.0, right: 30.0, top: 22.0, bottom: 10.0 })).show(ctx, |ui| {
            self.top(ui);
            ui.add_space(12.0);
            self.update_banner(ui);
            if let Some((msg, at)) = self.note.clone() {
                if at.elapsed() < Duration::from_secs(5) {
                    egui::Frame::none().fill(CARD_HI).rounding(10.0).inner_margin(egui::Margin::symmetric(14.0, 9.0)).show(ui, |ui| {
                        status_dot(ui, GOOD, &msg);
                    });
                    ui.add_space(8.0);
                } else {
                    self.note = None;
                }
            }
            if matches!(self.page, Page::Layouts | Page::Colors | Page::Display | Page::Features) {
                self.preview_card(ui);
                ui.add_space(4.0);
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 6.0);
                match self.page {
                    Page::Layouts => self.page_layouts(ui),
                    Page::Canvas => self.page_canvas(ui),
                    Page::Colors => self.page_colors(ui),
                    Page::Display => self.page_display(ui),
                    Page::Features => self.page_features(ui),
                    Page::Sensors => self.page_sensors(ui),
                }
                ui.add_space(24.0);
            });
        });
        self.autosave();
    }
}

fn main() -> eframe::Result {
    let icon = image::load_from_memory(include_bytes!("../../assets/studio-icon.png")).ok().map(|i| {
        let i = i.to_rgba8();
        egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1320.0, 880.0]).with_min_inner_size([1040.0, 660.0]).with_title("Aperture");
    if let Some(i) = icon {
        viewport = viewport.with_icon(i);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Aperture", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
