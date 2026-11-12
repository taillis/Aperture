//! Game identification and artwork lookup (hero banner, poster, transparent logo).
//!
//! Works analogously to [`crate::media`] for music:
//! 1. Receives `(exe_name, exe_path)` from the main loop (ETW / foreground / test hook).
//! 2. Resolves the game locally first (manual override, Steam `.acf` manifest,
//!    Epic `.item` manifest, cleaned `.exe` name) and checks local Steam `librarycache`.
//! 3. Falls back to Steam Store Search + Steam CDN (zero API key required), and
//!    optionally SteamGridDB API v2 when `steamgriddb_key` is configured.
//! 4. Caches decoded artwork on disk under `cache/games/` and publishes `Arc<GameInfo>`
//!    via `SharedGame` to `ProScreen`.

use crate::weather::{
    extract_array, json_bool, json_number, json_string, split_json_objects, urlencode,
};
use image::imageops::FilterType;
use image::{ColorType, ImageEncoder};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HTTP_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const NEGATIVE_CACHE_TTL_SECS: u64 = 24 * 3600;
const USER_AGENT: &str = "Aperture/0.1 (+https://github.com/Aperture)";

/// Runtime configuration controlling how game artwork is resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameArtConfig {
    /// Enable remote & local game identification / artwork lookup.
    pub enabled: bool,
    /// Preferred source: `"auto"`, `"steam"`, `"steamgriddb"`, or `"local"`.
    pub source: String,
    /// Optional SteamGridDB API v2 bearer token.
    pub steamgriddb_key: String,
    /// Manual `(exe_lower, target)` overrides where target is a Steam AppID, title, or image path.
    pub overrides: Vec<(String, String)>,
}

impl Default for GameArtConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            source: "auto".to_string(),
            steamgriddb_key: String::new(),
            overrides: Vec::new(),
        }
    }
}

/// Transparent RGBA game logo cropped tightly to its visible pixels.
#[derive(Clone, Debug)]
pub struct GameLogo {
    pub width: u32,
    pub height: u32,
    /// Raw non-premultiplied RGBA8888 pixels.
    pub rgba: Vec<u8>,
}

/// RGB888 game poster or wide hero background.
#[derive(Clone, Debug)]
pub struct GamePoster {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGB888 pixels.
    pub rgb: Vec<u8>,
}

/// Resolved metadata and artwork for the currently running game.
#[derive(Clone, Debug)]
pub struct GameInfo {
    /// Stable cache/render key (e.g. `"steam:1245620"` or `"exe:eldenring"`).
    pub key: String,
    /// Executable base name (without `.exe`).
    pub exe_name: String,
    /// Human-readable game title.
    pub title: String,
    /// Resolution source label (`"Steam"`, `"Epic Games"`, `"SteamGridDB"`, `"Local"`, `"Detected"`).
    pub source: String,
    /// Resolved Steam AppID when known.
    pub app_id: Option<u32>,
    /// Wide landscape banner suitable for full-screen dashboard backgrounds.
    pub hero: Option<GamePoster>,
    /// Portrait or rectangular game cover art.
    pub poster: Option<GamePoster>,
    /// Transparent game wordmark/logo.
    pub logo: Option<GameLogo>,
}

/// Shared handle read by the render loop.
pub type SharedGame = Arc<Mutex<Option<Arc<GameInfo>>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct GameQuery {
    exe_name: String,
    exe_path: String,
    config: GameArtConfig,
}

#[derive(Default)]
struct WatcherState {
    desired: Option<GameQuery>,
    seq: u64,
}

/// Coordinates background game identification without blocking the 20 FPS render loop.
pub struct GameWatcher {
    state: Mutex<WatcherState>,
    cv: Condvar,
    shared: SharedGame,
}

impl GameWatcher {
    /// Updates the currently active game executable and wakes the resolver thread if it changed.
    pub fn update(&self, exe_name: Option<&str>, exe_path: &str, config: &GameArtConfig) {
        let next = exe_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|name| GameQuery {
                exe_name: name.to_string(),
                exe_path: exe_path.trim().to_string(),
                config: config.clone(),
            });

        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.desired == next {
            return;
        }
        guard.desired = next;
        guard.seq = guard.seq.wrapping_add(1);
        if guard.desired.is_none() {
            if let Ok(mut out) = self.shared.lock() {
                *out = None;
            }
        }
        self.cv.notify_one();
    }
}

/// Spawns the background `"game-watcher"` thread and returns its controller + shared result.
pub fn spawn_game_watcher() -> anyhow::Result<(Arc<GameWatcher>, SharedGame)> {
    let shared: SharedGame = Arc::new(Mutex::new(None));
    let watcher = Arc::new(GameWatcher {
        state: Mutex::new(WatcherState::default()),
        cv: Condvar::new(),
        shared: Arc::clone(&shared),
    });

    let bg = Arc::clone(&watcher);
    std::thread::Builder::new()
        .name("game-watcher".into())
        .spawn(move || {
            let mut mem_cache: HashMap<String, Arc<GameInfo>> = HashMap::new();
            let mut handled_seq = 0u64;
            loop {
                let (query, seq) = {
                    let mut guard = bg.state.lock().unwrap_or_else(|e| e.into_inner());
                    while guard.seq == handled_seq {
                        guard = bg.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
                    }
                    handled_seq = guard.seq;
                    (guard.desired.clone(), guard.seq)
                };

                let Some(q) = query else {
                    continue;
                };

                let ov_sig = override_for(&q.exe_name, &q.config).unwrap_or("");
                let mem_key = format!(
                    "{}|{}|{}|{}|{}|{}",
                    q.exe_name.to_ascii_lowercase(),
                    q.exe_path,
                    q.config.enabled,
                    q.config.source,
                    !q.config.steamgriddb_key.is_empty(),
                    ov_sig
                );

                if let Some(hit) = mem_cache.get(&mem_key).cloned() {
                    if is_desired_seq(&bg, seq) {
                        set_shared(&bg.shared, Some(hit));
                    }
                    continue;
                }

                // Publish quick local identification immediately so HUD updates without waiting for network.
                let quick = resolve_local_only(&q.exe_name, &q.exe_path, &q.config);
                if is_desired_seq(&bg, seq) {
                    set_shared(&bg.shared, Some(Arc::new(quick.clone())));
                }

                let resolved = if q.config.enabled && q.config.source != "local" {
                    resolve_full(&q.exe_name, &q.exe_path, &q.config, Some(quick))
                } else {
                    quick
                };

                let arc = Arc::new(resolved);
                if mem_cache.len() > 64 {
                    mem_cache.clear();
                }
                mem_cache.insert(mem_key, Arc::clone(&arc));

                if is_desired_seq(&bg, seq) {
                    set_shared(&bg.shared, Some(arc));
                }
            }
        })?;

    Ok((watcher, shared))
}

fn is_desired_seq(bg: &GameWatcher, seq: u64) -> bool {
    bg.state
        .lock()
        .map(|g| g.seq == seq && g.desired.is_some())
        .unwrap_or(false)
}

fn set_shared(shared: &SharedGame, val: Option<Arc<GameInfo>>) {
    if let Ok(mut guard) = shared.lock() {
        *guard = val;
    }
}

/// Parses `exe=target` overrides separated by `;` (or `,` when values aren't filesystem paths).
pub fn parse_game_overrides(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let sep = if raw.contains(';') { ';' } else { ',' };
    for entry in raw.split(sep) {
        let entry = entry.trim();
        let Some((exe, target)) = entry.split_once('=') else {
            continue;
        };
        let exe = strip_exe_suffix(exe.trim()).to_ascii_lowercase();
        let target = target.trim();
        if !exe.is_empty() && !target.is_empty() {
            out.push((exe, target.to_string()));
        }
    }
    out
}

fn strip_exe_suffix(s: &str) -> &str {
    s.strip_suffix(".exe")
        .or_else(|| s.strip_suffix(".EXE"))
        .unwrap_or(s)
}

/// Cleans a game's `.exe` name into a search-friendly / readable game title.
pub fn clean_exe_title(exe: &str) -> String {
    let mut stem = strip_exe_suffix(exe.trim()).to_string();
    const SUFFIXES: &[&str] = &[
        "-win64-shipping",
        "-wingdk-shipping",
        "-win32-shipping",
        "_win64_shipping",
        "-shipping",
        "_shipping",
        "shipping",
        "_dx12",
        "-dx12",
        "_dx11",
        "-dx11",
        "_vulkan",
        "-vulkan",
        "_x64",
        "-x64",
        "x64",
        "_64",
        "-64",
    ];
    loop {
        let lower = stem.to_ascii_lowercase();
        let mut stripped = false;
        for suf in SUFFIXES {
            if lower.ends_with(suf) && stem.len() > suf.len() + 1 {
                stem.truncate(stem.len() - suf.len());
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }

    // Split camelCase / letter-to-digit boundaries before normalizing separators.
    let chars: Vec<char> = stem.chars().collect();
    let mut spaced = String::with_capacity(stem.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if matches!(c, '_' | '-' | '.') {
            spaced.push(' ');
            continue;
        }
        if i > 0 {
            let prev = chars[i - 1];
            let camel = prev.is_ascii_lowercase() && c.is_ascii_uppercase();
            let digit_split = prev.is_ascii_alphabetic() && c.is_ascii_digit();
            if (camel || digit_split) && !spaced.ends_with(' ') {
                spaced.push(' ');
            }
        }
        spaced.push(c);
    }

    let words: Vec<&str> = spaced.split_whitespace().collect();
    if words.is_empty() {
        return strip_exe_suffix(exe.trim()).to_string();
    }

    let joined = words.join(" ");
    let has_upper = joined.chars().any(|c| c.is_ascii_uppercase());
    if has_upper {
        joined
    } else {
        words
            .iter()
            .map(|w| {
                let mut ch = w.chars();
                match ch.next() {
                    Some(first) => {
                        let mut s = first.to_uppercase().to_string();
                        s.push_str(ch.as_str());
                        s
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Parses a single `"key" "value"` field inside a Valve `.acf` manifest.
pub fn parse_acf_field(text: &str, wanted_key: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = Vec::with_capacity(2);
        let mut rest = line.trim();
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else {
                break;
            };
            parts.push(&after[..end]);
            rest = &after[end + 1..];
            if parts.len() == 2 {
                break;
            }
        }
        if parts.len() == 2 && parts[0].eq_ignore_ascii_case(wanted_key) {
            let val = parts[1].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Detects Steam `(appid, title, steam_root)` from an executable path inside `steamapps/common/<dir>/...`.
pub fn find_steam_game(exe_path: &str) -> Option<(u32, String, Option<PathBuf>)> {
    let norm = exe_path.replace('\\', "/");
    let parts: Vec<&str> = norm.split('/').filter(|p| !p.is_empty()).collect();
    for i in 0..parts.len().saturating_sub(2) {
        if parts[i].eq_ignore_ascii_case("steamapps") && parts[i + 1].eq_ignore_ascii_case("common") {
            let install_dir = parts[i + 2];
            // Reconstruct `<prefix>/steamapps` using original separator style when native, or `/`.
            let prefix = if exe_path.starts_with('/') {
                format!("/{}", parts[..=i].join("/"))
            } else {
                parts[..=i].join("/")
            };
            let steamapps = PathBuf::from(&prefix);
            let Ok(entries) = std::fs::read_dir(&steamapps) else {
                continue;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !fname.starts_with("appmanifest_") || !fname.ends_with(".acf") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&p) else {
                    continue;
                };
                let Some(dir) = parse_acf_field(&text, "installdir") else {
                    continue;
                };
                if dir.eq_ignore_ascii_case(install_dir) {
                    let app_id = parse_acf_field(&text, "appid")?.parse::<u32>().ok()?;
                    let name = parse_acf_field(&text, "name")?;
                    let steam_root = steamapps.parent().map(Path::to_path_buf);
                    return Some((app_id, name, steam_root));
                }
            }
        }
    }
    None
}

/// Finds an Epic Games title matching `exe_path` inside the launcher manifests directory.
pub fn find_epic_game_in_dir(exe_path: &str, manifests_dir: &Path) -> Option<String> {
    let norm_exe = normalize_path_cmp(exe_path);
    let entries = std::fs::read_dir(manifests_dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Some(loc) = json_string(&text, "\"InstallLocation\"") else {
            continue;
        };
        let norm_loc = normalize_path_cmp(&loc);
        let prefix = format!("{}/", norm_loc.trim_end_matches('/'));
        if norm_exe.starts_with(&prefix) || norm_exe == norm_loc {
            if let Some(title) = json_string(&text, "\"DisplayName\"") {
                return Some(title);
            }
        }
    }
    None
}

fn find_epic_game(exe_path: &str) -> Option<String> {
    if exe_path.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        let dir = Path::new(r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests");
        if dir.is_dir() {
            return find_epic_game_in_dir(exe_path, dir);
        }
    }
    None
}

fn normalize_path_cmp(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

#[derive(Default)]
struct LocalSteamArt {
    hero: Option<PathBuf>,
    poster: Option<PathBuf>,
    logo: Option<PathBuf>,
}

fn find_local_steam_art(steam_root: Option<&Path>, app_id: u32) -> LocalSteamArt {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(r) = steam_root {
        roots.push(r.to_path_buf());
    }
    #[cfg(windows)]
    {
        let default_win = PathBuf::from(r"C:\Program Files (x86)\Steam");
        if default_win.is_dir() && !roots.iter().any(|r| r == &default_win) {
            roots.push(default_win);
        }
    }

    let mut out = LocalSteamArt::default();
    for root in roots {
        let libcache = root.join("appcache").join("librarycache");
        if !libcache.is_dir() {
            continue;
        }
        // New Steam layout: `appcache/librarycache/<appid>/...`
        let per_app = libcache.join(app_id.to_string());
        if per_app.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&per_app) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if out.hero.is_none() && (name == "library_hero.jpg" || name.starts_with("library_hero")) {
                        out.hero = Some(p);
                    } else if out.logo.is_none() && (name == "logo.png" || name.starts_with("logo")) {
                        out.logo = Some(p);
                    } else if out.poster.is_none()
                        && (name == "library_600x900.jpg"
                            || name.starts_with("library_capsule")
                            || name == "header.jpg")
                    {
                        out.poster = Some(p);
                    }
                }
            }
        }
        // Classic Steam layout: `<appid>_library_hero.jpg`, `<appid>_logo.png`, etc.
        let pick = |names: &[String]| names.iter().map(|f| libcache.join(f)).find(|p| p.is_file());
        if out.hero.is_none() {
            out.hero = pick(&[format!("{app_id}_library_hero.jpg")]);
        }
        if out.logo.is_none() {
            out.logo = pick(&[format!("{app_id}_logo.png")]);
        }
        if out.poster.is_none() {
            out.poster = pick(&[
                format!("{app_id}_library_600x900.jpg"),
                format!("{app_id}_header.jpg"),
            ]);
        }
    }
    out
}

fn cache_root() -> PathBuf {
    if let Ok(override_dir) = std::env::var("APERTURE_CACHE_DIR") {
        let p = PathBuf::from(override_dir);
        if !p.as_os_str().is_empty() {
            return p.join("games");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("cache").join("games");
            if candidate.exists() || dir.join("trofeo.conf").exists() {
                return candidate;
            }
        }
    }
    if let Ok(home) = std::env::var("LOCALAPPDATA") {
        if !home.is_empty() {
            return PathBuf::from(home).join("Aperture").join("cache").join("games");
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("aperture").join("games");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".cache").join("aperture").join("games");
        }
    }
    PathBuf::from("cache").join("games")
}

fn sanitize_cache_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_disk_cache(key: &str, exe_name: &str) -> Option<(GameInfo, bool)> {
    let dir = cache_root().join(sanitize_cache_key(key));
    let meta = std::fs::read_to_string(dir.join("meta.txt")).ok()?;
    let mut title = String::new();
    let mut source = String::new();
    let mut app_id = None;
    let mut ts = 0u64;
    for line in meta.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "title" => title = v.trim().to_string(),
            "source" => source = v.trim().to_string(),
            "appid" => app_id = v.trim().parse::<u32>().ok(),
            "ts" => ts = v.trim().parse::<u64>().unwrap_or(0),
            _ => {}
        }
    }
    if title.is_empty() {
        return None;
    }

    let hero = std::fs::read(dir.join("hero.jpg"))
        .ok()
        .and_then(|b| decode_poster_bytes(&b, 1920, 600));
    let poster = std::fs::read(dir.join("poster.jpg"))
        .ok()
        .and_then(|b| decode_poster_bytes(&b, 600, 900));
    let logo = std::fs::read(dir.join("logo.png"))
        .ok()
        .and_then(|b| decode_logo_bytes(&b, 760, 320));

    let has_art = hero.is_some() || poster.is_some() || logo.is_some();
    let fresh_miss = !has_art && now_unix_secs().saturating_sub(ts) < NEGATIVE_CACHE_TTL_SECS;
    if !has_art && !fresh_miss {
        return None;
    }

    let info = GameInfo {
        key: key.to_string(),
        exe_name: exe_name.to_string(),
        title,
        source: if source.is_empty() { "Cached".to_string() } else { source },
        app_id,
        hero,
        poster,
        logo,
    };
    Some((info, has_art || fresh_miss))
}

fn save_disk_cache(info: &GameInfo) {
    let dir = cache_root().join(sanitize_cache_key(&info.key));
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let meta = format!(
        "title={}\nsource={}\nappid={}\nts={}\n",
        info.title.replace(['\n', '\r'], " "),
        info.source.replace(['\n', '\r'], " "),
        info.app_id.map(|id| id.to_string()).unwrap_or_default(),
        now_unix_secs()
    );
    let _ = std::fs::write(dir.join("meta.txt"), meta);

    if let Some(hero) = &info.hero {
        let mut buf = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        if enc.write_image(&hero.rgb, hero.width, hero.height, ColorType::Rgb8.into()).is_ok() {
            let _ = std::fs::write(dir.join("hero.jpg"), buf);
        }
    }
    if let Some(poster) = &info.poster {
        let mut buf = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        if enc.write_image(&poster.rgb, poster.width, poster.height, ColorType::Rgb8.into()).is_ok() {
            let _ = std::fs::write(dir.join("poster.jpg"), buf);
        }
    }
    if let Some(logo) = &info.logo {
        let mut buf = Vec::new();
        let enc = image::codecs::png::PngEncoder::new(&mut buf);
        if enc.write_image(&logo.rgba, logo.width, logo.height, ColorType::Rgba8.into()).is_ok() {
            let _ = std::fs::write(dir.join("logo.png"), buf);
        }
    }
}

fn override_for<'a>(exe_name: &str, cfg: &'a GameArtConfig) -> Option<&'a str> {
    let clean = strip_exe_suffix(exe_name).to_ascii_lowercase();
    cfg.overrides
        .iter()
        .find(|(k, _)| *k == clean)
        .map(|(_, v)| v.as_str())
}

fn resolve_local_only(exe_name: &str, exe_path: &str, cfg: &GameArtConfig) -> GameInfo {
    let exe_clean = strip_exe_suffix(exe_name).to_string();
    let fallback_title = clean_exe_title(&exe_clean);
    let mut app_id: Option<u32> = None;
    let mut title = fallback_title;
    let mut source = "Detected".to_string();
    let mut steam_root: Option<PathBuf> = None;
    let mut custom_file: Option<PathBuf> = None;

    if let Some(ov) = override_for(&exe_clean, cfg) {
        if let Ok(id) = ov.parse::<u32>() {
            app_id = Some(id);
            source = "Steam".to_string();
        } else if Path::new(ov).is_file() {
            custom_file = Some(PathBuf::from(ov));
            source = "Local".to_string();
        } else {
            title = ov.to_string();
            source = "Manual".to_string();
        }
    }

    if app_id.is_none() && custom_file.is_none() {
        if let Some((sid, stitle, sroot)) = find_steam_game(exe_path) {
            app_id = Some(sid);
            if source == "Detected" {
                title = stitle;
                source = "Steam".to_string();
            }
            steam_root = sroot;
        } else if let Some(etitle) = find_epic_game(exe_path) {
            if source == "Detected" {
                title = etitle;
                source = "Epic Games".to_string();
            }
        }
    }

    let key = match app_id {
        Some(id) => format!("steam:{id}"),
        None => format!("exe:{}", exe_clean.to_ascii_lowercase()),
    };

    if let Some((mut cached, _)) = load_disk_cache(&key, &exe_clean) {
        if custom_file.is_none() {
            if cached.title.is_empty() {
                cached.title = title;
            }
            return cached;
        }
    }

    let mut hero = None;
    let mut poster = None;
    let mut logo = None;

    if let Some(p) = custom_file {
        if let Ok(bytes) = std::fs::read(&p) {
            hero = decode_poster_bytes(&bytes, 1920, 600);
            poster = decode_poster_bytes(&bytes, 600, 900);
        }
    } else if let Some(id) = app_id {
        let local = find_local_steam_art(steam_root.as_deref(), id);
        if let Some(p) = local.hero.and_then(|p| std::fs::read(p).ok()) {
            hero = decode_poster_bytes(&p, 1920, 600);
        }
        if let Some(p) = local.poster.and_then(|p| std::fs::read(p).ok()) {
            poster = decode_poster_bytes(&p, 600, 900);
        }
        if let Some(p) = local.logo.and_then(|p| std::fs::read(p).ok()) {
            logo = decode_logo_bytes(&p, 760, 320);
        }
    }

    GameInfo {
        key,
        exe_name: exe_clean,
        title,
        source,
        app_id,
        hero,
        poster,
        logo,
    }
}

fn resolve_full(
    exe_name: &str,
    exe_path: &str,
    cfg: &GameArtConfig,
    seed: Option<GameInfo>,
) -> GameInfo {
    let mut info = seed.unwrap_or_else(|| resolve_local_only(exe_name, exe_path, cfg));

    if let Some((_, complete)) = load_disk_cache(&info.key, &info.exe_name) {
        if complete && (info.hero.is_some() || info.logo.is_some() || info.poster.is_some()) {
            return info;
        }
    }

    // If the title is too short/generic without an override or manifest app_id, skip noisy API searches.
    let has_override = override_for(&info.exe_name, cfg).is_some();
    if info.app_id.is_none() && !has_override && !is_searchable_title(&info.title) {
        return info;
    }

    let pref = cfg.source.to_ascii_lowercase();
    let allow_steam = pref != "steamgriddb" || cfg.steamgriddb_key.trim().is_empty();
    let allow_sgdb = !cfg.steamgriddb_key.trim().is_empty() && pref != "steam";

    // Prefer SteamGridDB first when `game_art_source = steamgriddb`.
    if pref == "steamgriddb" && allow_sgdb {
        fill_from_steamgriddb(&mut info, cfg.steamgriddb_key.trim());
    }

    // Resolve Steam AppID + Steam CDN assets.
    if info.app_id.is_none() && allow_steam {
        if let Some((sid, sname)) = search_steam_store(&info.title) {
            info.app_id = Some(sid);
            info.key = format!("steam:{sid}");
            if info.source == "Detected" {
                info.title = sname;
                info.source = "Steam".to_string();
            }
            // Check disk cache under newly resolved `steam:<id>` key before fetching CDN.
            if let Some((cached, true)) = load_disk_cache(&info.key, &info.exe_name) {
                if cached.hero.is_some() || cached.logo.is_some() || cached.poster.is_some() {
                    return cached;
                }
            }
        }
    }

    if let Some(sid) = info.app_id {
        if allow_steam {
            fill_from_steam_cdn(&mut info, sid);
        }
    }

    // Use SteamGridDB to fill any still-missing artwork when key is provided.
    if allow_sgdb && pref != "steamgriddb" && (info.hero.is_none() || info.logo.is_none() || info.poster.is_none()) {
        fill_from_steamgriddb(&mut info, cfg.steamgriddb_key.trim());
    }

    save_disk_cache(&info);
    info
}

fn is_searchable_title(title: &str) -> bool {
    let lower = title.trim().to_ascii_lowercase();
    if lower.len() < 3 {
        return false;
    }
    !matches!(
        lower.as_str(),
        "game" | "launcher" | "bootstrapper" | "client" | "app" | "main" | "start" | "play"
    )
}

fn fill_from_steam_cdn(info: &mut GameInfo, app_id: u32) {
    if info.hero.is_none() {
        let urls = [
            format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{app_id}/library_hero.jpg"),
            format!("https://cdn.akamai.steamstatic.com/steam/apps/{app_id}/library_hero.jpg"),
        ];
        if let Some(bytes) = fetch_first_bytes(&urls) {
            info.hero = decode_poster_bytes(&bytes, 1920, 600);
        }
    }
    if info.logo.is_none() {
        let urls = [
            format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{app_id}/logo.png"),
            format!("https://cdn.akamai.steamstatic.com/steam/apps/{app_id}/logo.png"),
        ];
        if let Some(bytes) = fetch_first_bytes(&urls) {
            info.logo = decode_logo_bytes(&bytes, 760, 320);
        }
    }
    if info.poster.is_none() {
        let urls = [
            format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{app_id}/library_600x900.jpg"),
            format!("https://cdn.akamai.steamstatic.com/steam/apps/{app_id}/library_600x900.jpg"),
            format!("https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/{app_id}/header.jpg"),
            format!("https://cdn.akamai.steamstatic.com/steam/apps/{app_id}/header.jpg"),
        ];
        if let Some(bytes) = fetch_first_bytes(&urls) {
            info.poster = decode_poster_bytes(&bytes, 600, 900);
        }
    }
}

/// Queries Steam's public Store Search endpoint (zero API key required).
pub fn search_steam_store(query: &str) -> Option<(u32, String)> {
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US",
        urlencode(query)
    );
    let body = http_get_text(&url, None)?;
    parse_steam_storesearch(&body, query)
}

/// Parses Steam Store Search JSON into the best matching `(app_id, name)`.
pub fn parse_steam_storesearch(body: &str, query: &str) -> Option<(u32, String)> {
    let array = extract_array(body, "\"items\"")?;
    let q_tokens = normalized_tokens(query);
    let mut best: Option<(i32, u32, String)> = None;

    for obj in split_json_objects(array) {
        let Some(id) = json_number(obj, "\"id\"").map(|n| n as u32) else {
            continue;
        };
        let Some(name) = json_string(obj, "\"name\"") else {
            continue;
        };
        let kind = json_string(obj, "\"type\"");
        if kind.as_deref().is_some_and(|t| t != "app") {
            continue;
        }
        let n_tokens = normalized_tokens(&name);
        let overlap = q_tokens
            .iter()
            .filter(|qt| n_tokens.iter().any(|nt| nt == *qt))
            .count() as i32;
        let exact_bonus = if n_tokens == q_tokens { 10 } else { 0 };
        let score = exact_bonus + overlap * 2 - (n_tokens.len() as i32 - overlap).max(0);
        let replace = match &best {
            None => true,
            Some((best_score, _, _)) => score > *best_score,
        };
        if replace {
            best = Some((score, id, name));
        }
    }

    best.map(|(_, id, name)| (id, name))
}

fn normalized_tokens(s: &str) -> Vec<String> {
    s.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn fill_from_steamgriddb(info: &mut GameInfo, api_key: &str) {
    let sgdb_game = if let Some(app_id) = info.app_id {
        let url = format!("https://www.steamgriddb.com/api/v2/games/steam/{app_id}");
        http_get_text(&url, Some(api_key)).and_then(|b| parse_sgdb_game(&b))
    } else {
        None
    }
    .or_else(|| {
        let url = format!(
            "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
            urlencode(&info.title)
        );
        http_get_text(&url, Some(api_key)).and_then(|b| parse_sgdb_search_first(&b))
    });

    let Some((sgdb_id, sgdb_name)) = sgdb_game else {
        return;
    };
    if matches!(info.source.as_str(), "Detected" | "Manual") {
        info.title = sgdb_name;
        info.source = "SteamGridDB".to_string();
    }

    if info.hero.is_none() {
        let url = format!("https://www.steamgriddb.com/api/v2/heroes/game/{sgdb_id}?types=static");
        if let Some(asset_url) = http_get_text(&url, Some(api_key)).and_then(|b| parse_sgdb_first_url(&b)) {
            if let Some(bytes) = fetch_bytes(&asset_url) {
                info.hero = decode_poster_bytes(&bytes, 1920, 600);
            }
        }
    }
    if info.logo.is_none() {
        let url = format!("https://www.steamgriddb.com/api/v2/logos/game/{sgdb_id}?types=static");
        if let Some(asset_url) = http_get_text(&url, Some(api_key)).and_then(|b| parse_sgdb_first_url(&b)) {
            if let Some(bytes) = fetch_bytes(&asset_url) {
                info.logo = decode_logo_bytes(&bytes, 760, 320);
            }
        }
    }
    if info.poster.is_none() {
        let url = format!("https://www.steamgriddb.com/api/v2/grids/game/{sgdb_id}?types=static");
        if let Some(asset_url) = http_get_text(&url, Some(api_key)).and_then(|b| parse_sgdb_first_url(&b)) {
            if let Some(bytes) = fetch_bytes(&asset_url) {
                info.poster = decode_poster_bytes(&bytes, 600, 900);
            }
        }
    }
}

/// Parses SteamGridDB `/games/steam/{id}` response into `(sgdb_id, name)`.
pub fn parse_sgdb_game(body: &str) -> Option<(u32, String)> {
    if json_bool(body, "\"success\"") == Some(false) {
        return None;
    }
    let id = json_number(body, "\"id\"").map(|n| n as u32)?;
    let name = json_string(body, "\"name\"")?;
    Some((id, name))
}

/// Parses SteamGridDB `/search/autocomplete/...` response into `(sgdb_id, name)`.
pub fn parse_sgdb_search_first(body: &str) -> Option<(u32, String)> {
    if json_bool(body, "\"success\"") == Some(false) {
        return None;
    }
    let arr = extract_array(body, "\"data\"")?;
    let first = split_json_objects(arr).into_iter().next()?;
    let id = json_number(first, "\"id\"").map(|n| n as u32)?;
    let name = json_string(first, "\"name\"")?;
    Some((id, name))
}

/// Extracts the first asset `"url"` from SteamGridDB `/heroes|logos|grids/game/{id}` JSON.
pub fn parse_sgdb_first_url(body: &str) -> Option<String> {
    if json_bool(body, "\"success\"") == Some(false) {
        return None;
    }
    let arr = extract_array(body, "\"data\"")?;
    for obj in split_json_objects(arr) {
        if let Some(url) = json_string(obj, "\"url\"") {
            return Some(url);
        }
    }
    None
}

fn http_get_text(url: &str, bearer: Option<&str>) -> Option<String> {
    let mut req = ureq::get(url)
        .timeout(HTTP_TIMEOUT)
        .set("User-Agent", USER_AGENT);
    if let Some(tok) = bearer {
        req = req.set("Authorization", &format!("Bearer {tok}"));
    }
    let resp = req.call().ok()?;
    let mut buf = String::new();
    resp.into_reader()
        .take(512 * 1024)
        .read_to_string(&mut buf)
        .ok()?;
    Some(buf)
}

fn fetch_first_bytes(urls: &[String]) -> Option<Vec<u8>> {
    for url in urls {
        if let Some(bytes) = fetch_bytes(url) {
            return Some(bytes);
        }
    }
    None
}

fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let resp = ureq::get(url)
        .timeout(HTTP_TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .call()
        .ok()?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_IMAGE_BYTES as u64)
        .read_to_end(&mut buf)
        .ok()?;
    (!buf.is_empty()).then_some(buf)
}

/// Decodes image bytes into an RGB888 poster capped to `(max_w, max_h)`.
pub fn decode_poster_bytes(bytes: &[u8], max_w: u32, max_h: u32) -> Option<GamePoster> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let scaled = if w > max_w || h > max_h {
        let sx = max_w as f32 / w as f32;
        let sy = max_h as f32 / h as f32;
        let s = sx.min(sy);
        let nw = (w as f32 * s).round().max(1.0) as u32;
        let nh = (h as f32 * s).round().max(1.0) as u32;
        image::DynamicImage::ImageRgb8(image::imageops::resize(
            &img.to_rgb8(),
            nw,
            nh,
            FilterType::Triangle,
        ))
    } else {
        img
    };
    let rgb = scaled.to_rgb8();
    Some(GamePoster {
        width: rgb.width(),
        height: rgb.height(),
        rgb: rgb.into_raw(),
    })
}

/// Decodes a transparent PNG logo, auto-crops empty margins, and caps to `(max_w, max_h)`.
pub fn decode_logo_bytes(bytes: &[u8], max_w: u32, max_h: u32) -> Option<GameLogo> {
    let img = image::load_from_memory(bytes).ok()?;
    let rgba_img = img.to_rgba8();
    let (w, h) = rgba_img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }

    let raw = rgba_img.as_raw();
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for y in 0..h {
        for x in 0..w {
            let a = raw[((y * w + x) * 4 + 3) as usize];
            if a > 8 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return None;
    }

    let cw = max_x - min_x + 1;
    let ch = max_y - min_y + 1;
    let cropped = image::imageops::crop_imm(&rgba_img, min_x, min_y, cw, ch).to_image();
    let final_img = if cw > max_w || ch > max_h {
        let sx = max_w as f32 / cw as f32;
        let sy = max_h as f32 / ch as f32;
        let s = sx.min(sy);
        let nw = (cw as f32 * s).round().max(1.0) as u32;
        let nh = (ch as f32 * s).round().max(1.0) as u32;
        image::imageops::resize(&cropped, nw, nh, FilterType::Triangle)
    } else {
        cropped
    };

    Some(GameLogo {
        width: final_img.width(),
        height: final_img.height(),
        rgba: final_img.into_raw(),
    })
}

/// Renders a game logo into a square RGB cover tile matted onto `bg` (used when a layout only displays square covers).
pub fn logo_as_square_cover(logo: &GameLogo, bg: [u8; 3], size: u32) -> Vec<u8> {
    let s = size.max(16);
    // Keep a dark backing pad when theme background is light so bright/white game logos stay legible.
    let luma = (bg[0] as u32 * 299 + bg[1] as u32 * 587 + bg[2] as u32 * 114) / 1000;
    let pad_bg = if luma > 130 { [18, 22, 30] } else { bg };

    let mut out = vec![0u8; (s * s * 3) as usize];
    for p in out.chunks_exact_mut(3) {
        p.copy_from_slice(&pad_bg);
    }

    let inner = (s as f32 * 0.84).max(8.0);
    let scale = (inner / logo.width.max(1) as f32).min(inner / logo.height.max(1) as f32);
    let dw = (logo.width as f32 * scale).round().max(1.0) as u32;
    let dh = (logo.height as f32 * scale).round().max(1.0) as u32;
    let src = image::RgbaImage::from_raw(logo.width, logo.height, logo.rgba.clone());
    let Some(src) = src else {
        return out;
    };
    let resized = image::imageops::resize(&src, dw, dh, FilterType::Triangle);
    let ox = ((s - dw) / 2) as usize;
    let oy = ((s - dh) / 2) as usize;
    let rraw = resized.as_raw();

    for y in 0..dh as usize {
        for x in 0..dw as usize {
            let si = (y * dw as usize + x) * 4;
            let a = rraw[si + 3] as f32 / 255.0;
            if a <= 0.002 {
                continue;
            }
            let di = ((oy + y) * s as usize + (ox + x)) * 3;
            for c in 0..3 {
                out[di + c] =
                    (out[di + c] as f32 * (1.0 - a) + rraw[si + c] as f32 * a).clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Converts a `GamePoster` into a square RGB tile (center-crop cover fit) of edge `size`.
pub fn poster_as_square_cover(poster: &GamePoster, size: u32) -> Vec<u8> {
    let s = size.max(16);
    let Some(src) = image::RgbImage::from_raw(poster.width, poster.height, poster.rgb.clone()) else {
        return vec![24; (s * s * 3) as usize];
    };
    let side = poster.width.min(poster.height);
    let x0 = (poster.width - side) / 2;
    let y0 = (poster.height - side) / 2;
    let cropped = image::imageops::crop_imm(&src, x0, y0, side, side).to_image();
    image::imageops::resize(&cropped, s, s, FilterType::Triangle).into_raw()
}

/// Generates a rich synthetic sample `GameInfo` for offline `--preview-png` and layout thumbnails.
pub fn sample_game_info() -> GameInfo {
    let hw = 960u32;
    let hh = 231u32;
    let mut hero_rgb = vec![0u8; (hw * hh * 3) as usize];
    for y in 0..hh {
        let fy = y as f32 / hh as f32;
        for x in 0..hw {
            let fx = x as f32 / hw as f32;
            // Atmospheric cyberpunk skyline / sunset-neon sweep with subtle scanlines.
            let horizon = (1.0 - ((fy - 0.58).abs() * 3.2)).clamp(0.0, 1.0);
            let glow_r = ((fx - 0.72) * 2.2).powi(2) + ((fy - 0.45) * 3.0).powi(2);
            let cyan = (-glow_r * 3.5).exp();
            let neon = (-(fx - 0.55).powi(2) * 8.0 - (fy - 0.62).powi(2) * 14.0).exp();
            let grid = if fy > 0.68 && ((x % 32 == 0) || (y % 18 == 0)) {
                14.0 * (fy - 0.68) * 3.0
            } else {
                0.0
            };
            // Silhouette skyline towers on the right half.
            let col = x / 28;
            let tower_h = 0.25 + ((col.wrapping_mul(37) % 11) as f32) * 0.045;
            let in_tower = fx > 0.40 && (1.0 - fy) < tower_h && (x % 28) < 22;
            let mut r = 10.0 + 34.0 * fx + 150.0 * neon + 22.0 * horizon;
            let mut g = 14.0 + 28.0 * (1.0 - fy) + 125.0 * cyan + grid * 0.6;
            let mut b = 26.0 + 62.0 * fx + 160.0 * cyan + 70.0 * neon + grid;
            if in_tower {
                r *= 0.35;
                g *= 0.35;
                b *= 0.42;
                if (x % 28) % 6 == 2 && (y % 10) < 3 && fy > 0.45 {
                    r += 90.0;
                    g += 170.0;
                    b += 210.0;
                }
            }
            let idx = ((y * hw + x) * 3) as usize;
            hero_rgb[idx] = r.clamp(0.0, 255.0) as u8;
            hero_rgb[idx + 1] = g.clamp(0.0, 255.0) as u8;
            hero_rgb[idx + 2] = b.clamp(0.0, 255.0) as u8;
        }
    }

    let poster_rgb = poster_as_square_cover(
        &GamePoster {
            width: hw,
            height: hh,
            rgb: hero_rgb.clone(),
        },
        300,
    );

    // Anti-aliased geometric game crest + wordmark bars for sample preview.
    let lw = 380u32;
    let lh = 132u32;
    let mut logo_rgba = vec![0u8; (lw * lh * 4) as usize];
    for y in 0..lh {
        for x in 0..lw {
            let idx = ((y * lw + x) * 4) as usize;
            let fx = x as f32 / lw as f32;
            let dx = x as f32 - 64.0;
            let dy = y as f32 - 66.0;
            let r = (dx * dx + dy * dy).sqrt();
            // Smooth ring with opening on the right
            let ring_cov = (1.0 - ((r - 42.0).abs() - 6.0)).clamp(0.0, 1.0);
            let cut = if dx > 8.0 && dy.abs() < 14.0 { 0.0 } else { 1.0 };
            let diamond_d = (dx.abs() + dy.abs()) - 22.0;
            let diamond_cov = (1.0 - diamond_d).clamp(0.0, 1.0);
            // Sleek staggered cyber wordmark bars on right side of crest
            let in_top_bar = x >= 134 && x <= 368 && y >= 36 && y <= 56;
            let in_mid_bar = x >= 134 && x <= 318 && y >= 68 && y <= 84;
            let in_bot_bar = x >= 134 && x <= 252 && y >= 96 && y <= 104;
            let bar_cov = if in_top_bar || in_mid_bar || in_bot_bar { 1.0 } else { 0.0 };
            let alpha = (ring_cov * cut).max(diamond_cov).max(bar_cov);
            if alpha > 0.01 {
                logo_rgba[idx] = (248.0 - 155.0 * fx) as u8;
                logo_rgba[idx + 1] = (212.0 + 36.0 * fx) as u8;
                logo_rgba[idx + 2] = (72.0 + 180.0 * fx) as u8;
                logo_rgba[idx + 3] = (alpha * 255.0).round() as u8;
            }
        }
    }

    GameInfo {
        key: "sample:cyberpunk2077".to_string(),
        exe_name: "Cyberpunk2077".to_string(),
        title: "CYBERPUNK 2077".to_string(),
        source: "Steam".to_string(),
        app_id: Some(1091500),
        hero: Some(GamePoster {
            width: hw,
            height: hh,
            rgb: hero_rgb,
        }),
        poster: Some(GamePoster {
            width: 300,
            height: 300,
            rgb: poster_rgb,
        }),
        logo: Some(GameLogo {
            width: lw,
            height: lh,
            rgba: logo_rgba,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("aperture_game_test_{name}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn clean_exe_title_strips_launchers_and_splits_camel() {
        assert_eq!(clean_exe_title("eldenring.exe"), "Eldenring");
        assert_eq!(clean_exe_title("Cyberpunk2077"), "Cyberpunk 2077");
        assert_eq!(clean_exe_title("HollowKnight"), "Hollow Knight");
        assert_eq!(clean_exe_title("Palworld-Win64-Shipping.exe"), "Palworld");
        assert_eq!(clean_exe_title("Witcher3_dx12.exe"), "Witcher 3");
    }

    #[test]
    fn parse_acf_field_extracts_id_and_name() {
        let acf = r#"
"AppState"
{
    "appid"     "1245620"
    "name"      "ELDEN RING"
    "installdir"    "ELDEN RING"
}
"#;
        assert_eq!(parse_acf_field(acf, "appid").as_deref(), Some("1245620"));
        assert_eq!(parse_acf_field(acf, "name").as_deref(), Some("ELDEN RING"));
        assert_eq!(parse_acf_field(acf, "installdir").as_deref(), Some("ELDEN RING"));
    }

    #[test]
    fn find_steam_game_resolves_acf_and_librarycache() {
        let root = temp_dir("steam");
        let steamapps = root.join("steamapps");
        let exe_dir = steamapps.join("common").join("ELDEN RING").join("Game");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_1245620.acf"),
            "\"AppState\"\n{\n  \"appid\" \"1245620\"\n  \"name\" \"ELDEN RING\"\n  \"installdir\" \"ELDEN RING\"\n}\n",
        )
        .unwrap();

        let exe_path = exe_dir.join("eldenring.exe");
        let (appid, name, steam_root) = find_steam_game(exe_path.to_str().unwrap()).unwrap();
        assert_eq!(appid, 1245620);
        assert_eq!(name, "ELDEN RING");
        assert_eq!(steam_root.unwrap(), root);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_epic_game_in_dir_matches_install_location() {
        let dir = temp_dir("epic");
        std::fs::write(
            dir.join("alanwake2.item"),
            r#"{"DisplayName":"Alan Wake 2","InstallLocation":"D:\\Games\\Epic\\AlanWake2"}"#,
        )
        .unwrap();

        let title = find_epic_game_in_dir(r"d:\games\epic\alanwake2\AlanWake2.exe", &dir);
        assert_eq!(title.as_deref(), Some("Alan Wake 2"));
        assert!(find_epic_game_in_dir(r"C:\Games\Other\foo.exe", &dir).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_steam_storesearch_picks_best_app() {
        let json = r#"{
            "total": 2,
            "items": [
                {"type":"app","name":"Elden Ring Shadow of the Erdtree Soundtrack","id":9999},
                {"type":"app","name":"ELDEN RING","id":1245620}
            ]
        }"#;
        let (id, name) = parse_steam_storesearch(json, "Elden Ring").unwrap();
        assert_eq!(id, 1245620);
        assert_eq!(name, "ELDEN RING");
    }

    #[test]
    fn parse_sgdb_responses_extract_ids_and_urls() {
        let game = r#"{"success":true,"data":{"id":5272521,"name":"ELDEN RING","types":["steam"]}}"#;
        assert_eq!(parse_sgdb_game(game), Some((5272521, "ELDEN RING".to_string())));

        let heroes = r#"{"success":true,"data":[{"id":1,"url":"https://cdn2.steamgriddb.com/hero/abc.jpg"}]}"#;
        assert_eq!(
            parse_sgdb_first_url(heroes).as_deref(),
            Some("https://cdn2.steamgriddb.com/hero/abc.jpg")
        );
    }

    #[test]
    fn decode_logo_bytes_crops_transparent_padding() {
        // 20x10 transparent canvas with a 4x2 opaque block in center (x=8..12, y=4..6).
        let mut img = image::RgbaImage::new(20, 10);
        for y in 4..6 {
            for x in 8..12 {
                img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 20, 10, ColorType::Rgba8.into())
            .unwrap();

        let logo = decode_logo_bytes(&png, 200, 100).unwrap();
        assert_eq!((logo.width, logo.height), (4, 2));
        assert_eq!(logo.rgba.len(), 4 * 2 * 4);
    }

    #[test]
    fn parse_game_overrides_parses_pairs() {
        let parsed = parse_game_overrides("Minecraft=8888; RocketLeague.exe = Rocket League ; invalid");
        assert_eq!(
            parsed,
            vec![
                ("minecraft".to_string(), "8888".to_string()),
                ("rocketleague".to_string(), "Rocket League".to_string()),
            ]
        );
    }

    #[test]
    fn disk_cache_roundtrip_persists_hero_poster_and_logo() {
        let dir = temp_dir("cache");
        // Serialize env var access inside this process by setting & clearing explicitly here
        std::env::set_var("APERTURE_CACHE_DIR", &dir);
        let sample = sample_game_info();
        save_disk_cache(&sample);

        let (loaded, complete) = load_disk_cache(&sample.key, &sample.exe_name).unwrap();
        std::env::remove_var("APERTURE_CACHE_DIR");

        assert!(complete);
        assert_eq!(loaded.title, sample.title);
        assert_eq!(loaded.app_id, sample.app_id);
        assert!(loaded.hero.is_some());
        assert!(loaded.poster.is_some());
        assert!(loaded.logo.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
