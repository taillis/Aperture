//! Detects the .exe name of the currently active (foreground) window on Windows.
//!
//! Used by `main.rs` to swap the "NOW PLAYING" display for the name of the
//! game/program currently running when GPU usage is high (an indication
//! that a game is being played, not that music is being listened to).
//!
//! How it works: `GetForegroundWindow` (active window) -> `GetWindowThreadProcessId`
//! (PID owning that window) -> `OpenProcess` + `QueryFullProcessImageNameW`
//! (exe path from that PID). Three standard WinAPI calls, no admin
//! privileges needed for ordinary processes/games (uses
//! `PROCESS_QUERY_LIMITED_INFORMATION`, the minimal access right that's
//! enough for this).

/// Metadata for the currently focused process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundProcess {
    /// OS process identifier.
    pub pid: u32,
    /// Clean executable name without directory or `.exe` suffix.
    pub name: String,
    /// Full path to the executable when available, empty otherwise.
    pub path: String,
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    /// PID of the process that owns the foreground window.
    pub fn foreground_pid() -> Option<u32> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            (pid != 0).then_some(pid)
        }
    }

    /// Full executable path of process `pid`.
    /// `None` if the process can't be opened (a protected system process).
    pub fn exe_path_of(pid: u32) -> Option<String> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let result = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = CloseHandle(process);
            result.ok()?;
            let path = String::from_utf16_lossy(&buf[..size as usize]);
            (!path.trim().is_empty()).then_some(path)
        }
    }

    fn exe_name_from_path(path: &str) -> Option<String> {
        let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        let name = file_name
            .strip_suffix(".exe")
            .or_else(|| file_name.strip_suffix(".EXE"))
            .unwrap_or(file_name);
        (!name.trim().is_empty()).then(|| name.to_string())
    }

    /// Exe file name (without path, without the ".exe" extension) of process `pid`.
    /// `None` if the process can't be opened (a protected system process).
    pub fn exe_name_of(pid: u32) -> Option<String> {
        exe_name_from_path(&exe_path_of(pid)?)
    }

    /// Full foreground process info: PID, clean exe name, and full exe path.
    pub fn foreground_process_info() -> Option<super::ForegroundProcess> {
        let pid = foreground_pid()?;
        let path = exe_path_of(pid)?;
        let name = exe_name_from_path(&path)?;
        Some(super::ForegroundProcess { pid, name, path })
    }

    /// (pid, exe name) of the process that owns the foreground window.
    pub fn foreground_process() -> Option<(u32, String)> {
        let info = foreground_process_info()?;
        Some((info.pid, info.name))
    }

    /// Exe file name (without path, without the ".exe" extension) of the
    /// process that owns the current foreground window. `None` if there's
    /// no foreground window (e.g. locked screen), or the process can't be
    /// opened (system process with protection higher than our access
    /// level).
    pub fn foreground_exe_name() -> Option<String> {
        foreground_process().map(|p| p.1)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    /// Exe file name (without path, without extension) of the process that
    /// owns the currently active window — via X11 (`_NET_ACTIVE_WINDOW` +
    /// `_NET_WM_PID`, the standard EWMH protocol supported by every modern
    /// window manager: GNOME/Mutter, KDE/KWin, XFCE, i3, etc).
    ///
    /// LIMITATION: only works in an X11 session (native OR via XWayland for
    /// apps that aren't native Wayland yet). In a PURE Wayland session
    /// (native Wayland windows without XWayland), `_NET_ACTIVE_WINDOW`
    /// doesn't exist because the base Wayland protocol deliberately doesn't
    /// expose "which window is active" to other applications (a
    /// security/sandboxing restriction of Wayland's own architecture, not
    /// something that can be worked around here) — some compositors
    /// (Sway/wlroots) have their own additional protocol
    /// (`wlr-foreign-toplevel-management`) for this, but it's not standard
    /// across compositors. If this happens, this function returns `None` —
    /// game mode still works (GPU>50% based), it's just that NOW PLAYING
    /// won't switch to the game's name.
    pub fn foreground_process_info() -> Option<super::ForegroundProcess> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen_num)?.root;

        let net_active_window = intern_atom(&conn, b"_NET_ACTIVE_WINDOW")?;
        let net_wm_pid = intern_atom(&conn, b"_NET_WM_PID")?;

        let active_reply = conn
            .get_property(false, root, net_active_window, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let window = active_reply.value32()?.next()?;
        if window == 0 {
            return None;
        }

        let pid_reply = conn
            .get_property(false, window, net_wm_pid, AtomEnum::CARDINAL, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let pid = pid_reply.value32()?.next()?;
        if pid == 0 {
            return None;
        }

        let (name, path) = exe_info_from_pid(pid)?;
        Some(super::ForegroundProcess { pid, name, path })
    }

    pub fn foreground_exe_name() -> Option<String> {
        foreground_process_info().map(|p| p.name)
    }

    fn intern_atom(conn: &impl Connection, name: &[u8]) -> Option<u32> {
        Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
    }

    /// Exe name and full path from `/proc/{pid}/exe` — more accurate than
    /// `/proc/{pid}/comm`, which the kernel truncates to 15 characters.
    fn exe_info_from_pid(pid: u32) -> Option<(String, String)> {
        if let Ok(target) = fs::read_link(format!("/proc/{pid}/exe")) {
            let path = target.to_string_lossy().into_owned();
            if let Some(raw_name) = target.file_name().and_then(|n| n.to_str()) {
                let name = raw_name
                    .strip_suffix(".exe")
                    .or_else(|| raw_name.strip_suffix(".EXE"))
                    .unwrap_or(raw_name);
                if !name.trim().is_empty() {
                    return Some((name.to_string(), path));
                }
            }
        }
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        let name = comm.trim();
        (!name.is_empty()).then(|| (name.to_string(), String::new()))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn foreground_exe_name() -> Option<String> {
        None
    }

    pub fn foreground_process_info() -> Option<super::ForegroundProcess> {
        None
    }
}

pub use imp::{foreground_exe_name, foreground_process_info};
#[cfg(windows)]
#[allow(unused_imports)]
pub use imp::foreground_process;
