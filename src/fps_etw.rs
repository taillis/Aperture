//! FPS of the game being played, for ANY GPU, measured the way PresentMon does: by listening
//! to Windows' own `Present` events (ETW) and timing the frames of the game's process.
//!
//! Three event sources are used, so nearly every game is covered:
//! - `Microsoft-Windows-DXGI`  `Present_Start`  -> DirectX 10 / 11 / 12
//! - `Microsoft-Windows-D3D9`  `Present_Start`  -> DirectX 9
//! - `Microsoft-Windows-DxgKrnl` `Present_Info` -> everything else that reaches the screen
//!   through the graphics kernel (Vulkan, OpenGL); used only when the process has no
//!   DirectX events of its own.
//!
//! How the number is computed (and why it is steady):
//! - the frame rate comes from the events' own time stamps (QPC), over the last second of
//!   frames **anchored on the newest frame**, not on the wall clock. ETW delivers events in
//!   batches, so counting "events that arrived in the last second" under-reads whenever a
//!   batch is still on its way; time stamps do not have that problem;
//! - `(frames - 1) / time between the first and the last frame` instead of a bare count, so
//!   the value is not off by one and does not flicker between two integers;
//! - only the event IDs we need are enabled (ID filter), so nothing is lost at high FPS.
//!
//! Which process is "the game": the foreground window's process if it is presenting and it
//! is not a browser / launcher / chat / desktop program (see [`NOT_GAMES`]); when you alt-tab
//! away, the last game keeps being shown for as long as it keeps rendering.
//!
//! Notes: needs administrator rights (real-time ETW); Windows only.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NotStarted,
    Running,
    /// ETW couldn't be started (usually: missing administrator access).
    NeedsAdmin,
    Failed,
}

/// Where a frame rate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Dxgi,
    D3d9,
    /// Graphics-kernel presents (Vulkan / OpenGL).
    Kernel,
}

impl Api {
    pub fn name(self) -> &'static str {
        match self {
            Api::Dxgi => "DirectX 10-12",
            Api::D3d9 => "DirectX 9",
            Api::Kernel => "Vulkan / OpenGL",
        }
    }
}

/// One measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub fps: f32,
    pub api: Api,
}

/// Programs that draw with the GPU all the time but are not games: showing "FPS 144" for a
/// browser or the desktop is noise. Lower-case exe names without `.exe`. Extend it with
/// `fps_ignore = name, name` in the config; `fps_ignore = none` shows every program.
pub const NOT_GAMES: &[&str] = &[
    // desktop / shell
    "explorer", "dwm", "searchhost", "searchapp", "shellexperiencehost", "startmenuexperiencehost",
    "textinputhost", "applicationframehost", "systemsettings", "widgets", "widgetboard", "lockapp",
    "taskmgr", "snippingtool", "screenclippinghost", "phoneexperiencehost", "gamebar", "gamebarftserver",
    // browsers and web views
    "chrome", "msedge", "msedgewebview2", "firefox", "brave", "opera", "opera_gx", "vivaldi", "arc",
    "zen", "librewolf", "waterfox", "thorium", "chromium", "coccoc", "browser",
    // launchers and overlays
    "steam", "steamwebhelper", "epicgameslauncher", "epicwebhelper", "galaxyclient", "battle.net",
    "riotclientux", "riotclientservices", "leagueclientux", "leagueclient", "ubisoftconnect", "upc",
    "eadesktop", "origin", "rockstar games launcher", "launcher", "socialclubhelper", "xboxpcapp",
    "nvidia overlay", "nvidia app", "nvidia share", "radeonsoftware", "amdrssrcext", "msiafterburner",
    "rtss", "overwolf", "medal", "playnite.desktopapp", "hydralauncher",
    // chat / media / work
    "discord", "discordptb", "discordcanary", "vesktop", "telegram", "whatsapp", "zalo", "wechat",
    "weixin", "slack", "teams", "ms-teams", "zoom", "skype", "spotify", "vlc", "mpc-hc64", "mpc-hc",
    "mpc-be64", "mpv", "potplayermini64", "potplayer64", "foobar2000", "applemusic", "itunes",
    "obs64", "obs32", "streamlabs obs", "code", "cursor", "devenv", "notepad", "windowsterminal",
    "wt", "notion", "obsidian", "figma", "claude", "chatgpt", "codex", "lmstudio", "lm studio",
    "acrobat", "winword", "excel", "powerpnt", "outlook", "onenote", "photos", "mspaint",
    // wallpapers / monitors / ourselves
    "wallpaper32", "wallpaper64", "lively", "rainmeter", "hwinfo64", "hwinfo32", "librehardwaremonitor",
    "trcc", "aperture", "aperture-display", "aperture-config", "trofeo_lcd", "huybruh_dash",
];

/// Is `exe` (any case, with or without `.exe`) one of the non-game programs?
/// `extra` = the user's own additions (`fps_ignore`); a single `none` switches the list off.
pub fn is_ignored(exe: &str, extra: &[String]) -> bool {
    let name = exe.trim().to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);
    if name.is_empty() {
        return true;
    }
    if extra.iter().any(|e| e == "none") {
        return false;
    }
    NOT_GAMES.contains(&name) || extra.iter().any(|e| e == name)
}

/// Parses `fps_ignore = a, b.exe` into lower-case names without `.exe`.
pub fn parse_ignore(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .map(|s| s.strip_suffix(".exe").map(str::to_string).unwrap_or(s))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Remembers which process is the game.
#[derive(Default)]
pub struct Target {
    pid: Option<u32>,
    name: String,
    path: String,
}

impl Target {
    /// `foreground` = (pid, exe name) of the focused window; `read` = the frame rate of a pid.
    /// Returns the measurement and the exe name it belongs to.
    pub fn sample(
        &mut self,
        foreground: Option<(u32, String)>,
        ignore: &[String],
        read: impl FnMut(u32) -> Option<Sample>,
    ) -> Option<(Sample, &str)> {
        let fg = foreground.map(|(pid, name)| (pid, name, String::new()));
        self.sample_with_path(fg, ignore, read)
            .map(|(s, name, _path)| (s, name))
    }

    /// Like `sample`, but also preserves the executable's full path for launcher/manifest lookup.
    pub fn sample_with_path(
        &mut self,
        foreground: Option<(u32, String, String)>,
        ignore: &[String],
        mut read: impl FnMut(u32) -> Option<Sample>,
    ) -> Option<(Sample, &str, &str)> {
        if let Some((pid, name, path)) = foreground {
            if !is_ignored(&name, ignore) {
                if let Some(s) = read(pid) {
                    self.pid = Some(pid);
                    self.name = name;
                    self.path = path;
                    return Some((s, &self.name, &self.path));
                }
            }
        }
        // focus is elsewhere (alt-tab, second monitor): the last game, while it keeps rendering
        match self.pid.and_then(|pid| read(pid)) {
            Some(s) => Some((s, &self.name, &self.path)),
            None => {
                self.pid = None;
                self.path.clear();
                None
            }
        }
    }
}

/// Frames per second from time stamps (`ticks`, oldest first; `freq` ticks per second):
/// the frames of the last second before the newest one. `None` when the newest frame is
/// older than `stale` seconds at `now`, or when there are too few frames to tell.
pub fn rate(ticks: &std::collections::VecDeque<i64>, freq: i64, now: i64, stale: f32) -> Option<f32> {
    let last = *ticks.back()?;
    if freq <= 0 || (now - last) as f32 > stale * freq as f32 {
        return None;
    }
    let from = last - freq;
    let n = ticks.iter().rev().take_while(|t| **t > from).count();
    if n < 3 {
        return None;
    }
    let first = ticks[ticks.len() - n];
    let span = (last - first) as f32 / freq as f32;
    if span < 0.15 {
        return None;
    }
    Some((n - 1) as f32 / span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn frames(fps: i64, seconds: i64, freq: i64) -> VecDeque<i64> {
        (0..=fps * seconds).map(|i| i * freq / fps).collect()
    }

    #[test]
    fn rate_is_exact_for_steady_frames_and_ignores_delivery_lag() {
        let freq = 10_000_000;
        for fps in [24, 30, 60, 144, 240, 500] {
            let q = frames(fps, 3, freq);
            let last = *q.back().unwrap();
            // read right away, and 0.9 s later while the next ETW batch is still on its way
            for lag in [0, freq * 9 / 10] {
                let r = rate(&q, freq, last + lag, 2.5).unwrap();
                assert!((r - fps as f32).abs() < 0.6, "{fps} fps read as {r}");
            }
        }
    }

    #[test]
    fn rate_goes_away_when_the_game_stops() {
        let freq = 10_000_000;
        let q = frames(60, 2, freq);
        let last = *q.back().unwrap();
        assert!(rate(&q, freq, last + freq * 2, 2.5).is_some());
        assert!(rate(&q, freq, last + freq * 3, 2.5).is_none());
        assert!(rate(&VecDeque::new(), freq, 0, 2.5).is_none());
        assert!(rate(&VecDeque::from([1, 2]), freq, 2, 2.5).is_none());
    }

    #[test]
    fn browsers_and_the_desktop_are_not_games() {
        let none: Vec<String> = Vec::new();
        for exe in ["chrome.exe", "MSEDGE.EXE", "explorer", "Discord", "dwm", "aperture"] {
            assert!(is_ignored(exe, &none), "{exe}");
        }
        for exe in ["cs2.exe", "eldenring", "Cyberpunk2077.exe", "bf6", "GTA5"] {
            assert!(!is_ignored(exe, &none), "{exe}");
        }
        assert!(is_ignored("mygame.exe", &parse_ignore("MyGame.exe, other")));
        assert!(!is_ignored("chrome", &parse_ignore("none")));
    }

    #[test]
    fn target_follows_the_game_and_never_keeps_stale_frames() {
        let none: Vec<String> = Vec::new();
        let s = |fps: f32| Sample { fps, api: Api::Dxgi };
        let mut t = Target::default();
        // game in front
        assert_eq!(t.sample(Some((7, "game".into())), &none, |p| (p == 7).then_some(s(95.0))).map(|r| r.0.fps), Some(95.0));
        // alt-tab to the browser: the browser's own frames are ignored, the game stays
        let r = t.sample(Some((9, "chrome".into())), &none, |p| Some(s(if p == 7 { 90.0 } else { 60.0 })));
        assert_eq!(r.map(|r| (r.0.fps, r.1.to_string())), Some((90.0, "game".to_string())));
        // the game closed: nothing, and it does not come back by itself
        assert!(t.sample(Some((9, "chrome".into())), &none, |_| None).is_none());
        assert!(t.sample(None, &none, |p| (p == 7).then_some(s(100.0))).is_none());
        // a new game
        assert_eq!(t.sample(Some((10, "other".into())), &none, |p| (p == 10).then_some(s(120.0))).map(|r| r.0.fps), Some(120.0));
    }
}

#[cfg(windows)]
mod imp {
    use super::{rate, Api, Sample, Status};
    use std::collections::{HashMap, VecDeque};
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Mutex, OnceLock};
    use windows::core::{GUID, PCWSTR, PWSTR};
    use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
    use windows::Win32::System::Diagnostics::Etw::*;
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    const SESSION_NAME: &str = "ApertureFpsSession";
    /// Sessions of older versions, stopped at start so they do not linger.
    const OLD_SESSIONS: &[&str] = &["TrofeoLcdFpsSession"];

    const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
    const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
    const DXGKRNL: GUID = GUID::from_u128(0x802EC45A_1E99_4B83_9920_87C98277BA9D);
    const DXGI_PRESENT_START: u16 = 42;
    const DXGI_PRESENT_MPO_START: u16 = 55;
    const D3D9_PRESENT_START: u16 = 1;
    const DXGKRNL_PRESENT_INFO: u16 = 184;

    /// Seconds of frames kept per source; a little more than the 1 s the rate needs.
    const KEEP_S: i64 = 2;
    /// A source with no frame for this long is not presenting any more (the ETW flush
    /// timer is 0.2 s, 1 s on systems that refuse it; this leaves room for both).
    const STALE_S: f32 = 2.5;

    #[derive(Default)]
    struct Proc {
        /// One queue per swap chain: a game with a second (overlay / video) swap chain
        /// must not have its frames added together.
        dxgi: HashMap<u64, VecDeque<i64>>,
        d3d9: VecDeque<i64>,
        kernel: VecDeque<i64>,
        last: i64,
    }

    static PROCS: OnceLock<Mutex<HashMap<u32, Proc>>> = OnceLock::new();
    static STATUS: AtomicU8 = AtomicU8::new(0);
    static FREQUENCY: OnceLock<i64> = OnceLock::new();

    fn procs() -> &'static Mutex<HashMap<u32, Proc>> {
        PROCS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub fn status() -> Status {
        match STATUS.load(Ordering::Relaxed) {
            1 => Status::Running,
            2 => Status::NeedsAdmin,
            3 => Status::Failed,
            _ => Status::NotStarted,
        }
    }

    fn push(q: &mut VecDeque<i64>, t: i64, freq: i64) {
        q.push_back(t);
        while q.front().is_some_and(|f| t - *f > KEEP_S * freq) || q.len() > 8192 {
            q.pop_front();
        }
    }

    unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
        if rec.is_null() {
            return;
        }
        let h = &(*rec).EventHeader;
        let id = h.EventDescriptor.Id;
        let Some(&freq) = FREQUENCY.get() else { return };
        let t = h.TimeStamp;
        let pid = h.ProcessId;
        if pid == 0 {
            return;
        }
        // which source, and (DXGI) which swap chain
        let mut chain = 0u64;
        let api = if h.ProviderId == DXGI {
            if id != DXGI_PRESENT_START && id != DXGI_PRESENT_MPO_START {
                return;
            }
            // payload: swap chain pointer, flags (u32), sync interval
            let pointer_bytes = if h.Flags & 0x20 != 0 { 4 } else { 8 }; // EVENT_HEADER_FLAG_32_BIT_HEADER
            let data = (*rec).UserData.cast::<u8>();
            if !data.is_null() && usize::from((*rec).UserDataLength) >= pointer_bytes + 4 {
                let payload = std::slice::from_raw_parts(data, pointer_bytes + 4);
                let mut address = [0u8; 8];
                address[..pointer_bytes].copy_from_slice(&payload[..pointer_bytes]);
                chain = u64::from_le_bytes(address);
                let flags = u32::from_le_bytes([payload[pointer_bytes], payload[pointer_bytes + 1], payload[pointer_bytes + 2], payload[pointer_bytes + 3]]);
                if flags & 1 != 0 {
                    return; // DXGI_PRESENT_TEST: asks "could I present?", shows nothing
                }
            }
            Api::Dxgi
        } else if h.ProviderId == D3D9 {
            if id != D3D9_PRESENT_START {
                return;
            }
            Api::D3d9
        } else if h.ProviderId == DXGKRNL {
            if id != DXGKRNL_PRESENT_INFO {
                return;
            }
            Api::Kernel
        } else {
            return;
        };
        if let Ok(mut map) = procs().lock() {
            if map.len() > 256 {
                map.retain(|_, p| t - p.last <= 5 * freq);
            }
            let p = map.entry(pid).or_default();
            p.last = t;
            match api {
                Api::Dxgi => {
                    if p.dxgi.len() > 16 {
                        p.dxgi.retain(|_, q| q.back().is_some_and(|l| t - *l <= KEEP_S * freq));
                    }
                    push(p.dxgi.entry(chain).or_default(), t, freq)
                }
                Api::D3d9 => push(&mut p.d3d9, t, freq),
                Api::Kernel => push(&mut p.kernel, t, freq),
            }
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// EVENT_TRACE_PROPERTIES buffer + session name, 8-byte aligned.
    fn make_props(name: &str, ms_flush: bool) -> Vec<u64> {
        let name_bytes = (name.len() + 1) * 2;
        let total = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + name_bytes + 64;
        let mut buf = vec![0u64; total.div_ceil(8)];
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        unsafe {
            (*p).Wnode.BufferSize = (buf.len() * 8) as u32;
            (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            (*p).Wnode.ClientContext = 1; // time stamps = QPC
            (*p).BufferSize = 64; // KB per buffer
            (*p).MinimumBuffers = 16;
            (*p).MaximumBuffers = 64;
            // deliver events 5 times a second instead of once (0x10 = EVENT_TRACE_USE_MS_FLUSH_TIMER)
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE | if ms_flush { 0x10 } else { 0 };
            (*p).FlushTimer = if ms_flush { 200 } else { 1 };
            (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        }
        buf
    }

    fn stop_session(name: &str) {
        let w = wide(name);
        let mut props = make_props(name, false);
        unsafe {
            let _ = ControlTraceW(
                CONTROLTRACE_HANDLE::default(),
                PCWSTR(w.as_ptr()),
                props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
                EVENT_TRACE_CONTROL_STOP,
            );
        }
    }

    /// Turns `provider` on for the event IDs in `ids` only (falls back to "all events of
    /// these keywords" where ID filters are not supported).
    unsafe fn enable(handle: CONTROLTRACE_HANDLE, provider: &GUID, keywords: u64, ids: &[u16]) -> u32 {
        // EVENT_FILTER_EVENT_ID { FilterIn: u8, Reserved: u8, Count: u16, Events: [u16; Count] }
        let mut filter: Vec<u16> = Vec::with_capacity(2 + ids.len());
        filter.push(1); // FilterIn = TRUE (little endian: byte 0 = 1, byte 1 = reserved)
        filter.push(ids.len() as u16);
        filter.extend_from_slice(ids);
        let mut desc = EVENT_FILTER_DESCRIPTOR {
            Ptr: filter.as_ptr() as u64,
            Size: (filter.len() * 2) as u32,
            Type: 0x8000_0200, // EVENT_FILTER_TYPE_EVENT_ID
        };
        let params = ENABLE_TRACE_PARAMETERS {
            Version: 2, // ENABLE_TRACE_PARAMETERS_VERSION_2
            EnableProperty: 0,
            ControlFlags: 0,
            SourceId: GUID::zeroed(),
            EnableFilterDesc: &mut desc,
            FilterDescCount: 1,
        };
        let err = EnableTraceEx2(handle, provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, 4, keywords, 0, 0, Some(&params));
        if err.0 == 0 {
            return 0;
        }
        EnableTraceEx2(handle, provider, EVENT_CONTROL_CODE_ENABLE_PROVIDER.0, 4, keywords, 0, 0, None).0
    }

    pub fn start() -> Result<(), String> {
        let mut frequency = 0;
        unsafe { QueryPerformanceFrequency(&mut frequency) }.map_err(|error| format!("QPC frequency: {error}"))?;
        if frequency <= 0 {
            return Err("Invalid QPC frequency".into());
        }
        let _ = FREQUENCY.set(frequency);
        for old in OLD_SESSIONS {
            stop_session(old);
        }

        let name = wide(SESSION_NAME);
        let mut handle = CONTROLTRACE_HANDLE::default();
        unsafe {
            let mut err = windows::Win32::Foundation::WIN32_ERROR(0);
            // first with the fast flush timer; plain 1 s timer if the system refuses it
            for ms_flush in [true, false] {
                let mut props = make_props(SESSION_NAME, ms_flush);
                err = StartTraceW(&mut handle, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
                if err == ERROR_ALREADY_EXISTS {
                    // left over from a previous run (crash): stop it and retry
                    stop_session(SESSION_NAME);
                    let mut props = make_props(SESSION_NAME, ms_flush);
                    err = StartTraceW(&mut handle, PCWSTR(name.as_ptr()), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
                }
                if err.0 == 0 || err.0 == 5 {
                    break;
                }
            }
            if err.0 != 0 {
                STATUS.store(if err.0 == 5 { 2 } else { 3 }, Ordering::Relaxed);
                return Err(format!("StartTrace error {} (5 = administrator required)", err.0));
            }

            // DirectX 10-12 is the essential one; the other two only widen the coverage.
            let e = enable(handle, &DXGI, 0x8000_0000_0000_0002, &[DXGI_PRESENT_START, DXGI_PRESENT_MPO_START]);
            if e != 0 {
                STATUS.store(3, Ordering::Relaxed);
                stop_session(SESSION_NAME);
                return Err(format!("EnableTraceEx2 (DXGI) error {e}"));
            }
            let e = enable(handle, &D3D9, 0x8000_0000_0000_0002, &[D3D9_PRESENT_START]);
            if e != 0 {
                eprintln!("FPS: DirectX 9 events not available (error {e})");
            }
            // keyword Present | Base, Performance channel
            let e = enable(handle, &DXGKRNL, 0x4000_0000_0800_0001, &[DXGKRNL_PRESENT_INFO]);
            if e != 0 {
                eprintln!("FPS: graphics-kernel events not available (error {e}); Vulkan / OpenGL games show no FPS");
            }
        }

        let logger = wide(SESSION_NAME);
        std::thread::spawn(move || unsafe {
            let mut logger = logger;
            let mut file: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            file.LoggerName = PWSTR(logger.as_mut_ptr());
            file.Anonymous1.ProcessTraceMode =
                PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
            file.Anonymous2.EventRecordCallback = Some(on_event);
            let th = OpenTraceW(&mut file);
            if th.Value == u64::MAX {
                STATUS.store(3, Ordering::Relaxed);
                eprintln!("FPS: OpenTrace failed");
                return;
            }
            let _ = ProcessTrace(&[th], None, None); // blocks while the session is alive
            STATUS.store(3, Ordering::Relaxed);
        });
        STATUS.store(1, Ordering::Relaxed);
        Ok(())
    }

    /// Stops the ETW session (called when the program exits).
    pub fn stop() {
        if STATUS.load(Ordering::Relaxed) == 1 {
            stop_session(SESSION_NAME);
        }
    }

    /// The frame rate of process `pid`; `None` when it is not presenting.
    pub fn fps_for_pid(pid: u32) -> Option<Sample> {
        let freq = *FREQUENCY.get()?;
        let mut now = 0;
        unsafe { QueryPerformanceCounter(&mut now) }.ok()?;
        let map = procs().lock().ok()?;
        let p = map.get(&pid)?;
        // the busiest swap chain = the game's own picture
        let dxgi = p.dxgi.values().filter_map(|q| rate(q, freq, now, STALE_S)).fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.max(b))));
        let (fps, api) = if let Some(f) = dxgi {
            (f, Api::Dxgi)
        } else if let Some(f) = rate(&p.d3d9, freq, now, STALE_S) {
            (f, Api::D3d9)
        } else {
            (rate(&p.kernel, freq, now, STALE_S)?, Api::Kernel)
        };
        (fps >= 1.0).then_some(Sample { fps, api })
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Sample, Status};
    pub fn status() -> Status {
        Status::NotStarted
    }
    pub fn start() -> Result<(), String> {
        Err("FPS via ETW is only available on Windows".into())
    }
    pub fn stop() {}
    pub fn fps_for_pid(_pid: u32) -> Option<Sample> {
        None
    }
}

pub use imp::{fps_for_pid, start, status, stop};
