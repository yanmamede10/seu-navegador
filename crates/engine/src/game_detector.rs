//! Win32 Game Mode detector (SPEC §9):
//! - Exclusive Direct3D fullscreen via `SHQueryUserNotificationState` (`QUNS_RUNNING_D3D_FULL_SCREEN`)
//! - Borderless fullscreen via foreground window vs monitor rect, explicitly EXCLUDING
//!   all processes belonging to this Seu Navegador instance (so YouTube fullscreen never triggers)
//! - Foreground executable name matching against the user's configurable `.exe` list
//! - Minimized browser detection (`IsIconic` on all tracked native windows).

#![cfg(target_os = "windows")]

use crate::NativeWindowHandle;
use seu_core::game_mode::GameProbeSample;
use std::collections::HashSet;
use windows_sys::Win32::Foundation::{CloseHandle, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};

const IGNORED_SHELL_EXES: &[&str] = &[
    "explorer.exe",
    "shellexperiencehost.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "lockapp.exe",
    "applicationframehost.exe",
    "dwm.exe",
];

fn collect_own_process_tree_pids() -> HashSet<u32> {
    let root_pid = unsafe { GetCurrentProcessId() };
    let mut owned = HashSet::new();
    owned.insert(root_pid);

    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap.is_null() || snap == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return owned;
    }

    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut ok = unsafe { Process32FirstW(snap, &mut entry) };
    while ok != 0 {
        pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
        ok = unsafe { Process32NextW(snap, &mut entry) };
    }
    unsafe { CloseHandle(snap) };

    // Expand descendants (CEF GPU, network, renderer child processes).
    loop {
        let mut added = false;
        for &(pid, ppid) in &pairs {
            if owned.contains(&ppid) && owned.insert(pid) {
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    owned
}

fn query_process_basename(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(handle) };
    if ok == 0 || len == 0 {
        return None;
    }
    let full = String::from_utf16_lossy(&buf[..len as usize]);
    full.rsplit(['\\', '/'])
        .next()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// Samples current Windows fullscreen / foreground executable / minimized state.
pub fn sample_windows_game_state(
    now_ms: u64,
    browser_handles: &[NativeWindowHandle],
) -> GameProbeSample {
    let own_pids = collect_own_process_tree_pids();

    let mut quns: QUERY_USER_NOTIFICATION_STATE = 0;
    let d3d_fullscreen = unsafe { SHQueryUserNotificationState(&mut quns) == 0 }
        && quns == QUNS_RUNNING_D3D_FULL_SCREEN;

    let fg = unsafe { GetForegroundWindow() };
    let mut fg_pid: u32 = 0;
    if !fg.is_null() {
        unsafe { GetWindowThreadProcessId(fg, &mut fg_pid) };
    }

    let foreground_is_own_browser = fg_pid != 0 && own_pids.contains(&fg_pid);
    let foreground_exe = if fg_pid != 0 {
        query_process_basename(fg_pid)
    } else {
        None
    };

    let mut borderless_fullscreen = false;
    if !fg.is_null() && !foreground_is_own_browser {
        let visible = unsafe { IsWindowVisible(fg) != 0 };
        let iconic = unsafe { IsIconic(fg) != 0 };
        let is_shell_exe = foreground_exe
            .as_deref()
            .map(|e| IGNORED_SHELL_EXES.contains(&e))
            .unwrap_or(false);

        if visible && !iconic && !is_shell_exe {
            let mut wr: RECT = unsafe { std::mem::zeroed() };
            let got_rect = unsafe { GetWindowRect(fg, &mut wr) != 0 };
            let mon = unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) };
            let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let got_mon = !mon.is_null() && unsafe { GetMonitorInfoW(mon, &mut mi) != 0 };

            if got_rect && got_mon {
                let mr = mi.rcMonitor;
                let w = wr.right - wr.left;
                let h = wr.bottom - wr.top;
                let mw = mr.right - mr.left;
                let mh = mr.bottom - mr.top;
                if mw > 0
                    && mh > 0
                    && w >= mw - 2
                    && h >= mh - 2
                    && wr.left <= mr.left + 2
                    && wr.top <= mr.top + 2
                {
                    borderless_fullscreen = true;
                }
            }
        }
    }

    let all_browser_windows_minimized = !browser_handles.is_empty()
        && browser_handles
            .iter()
            .all(|h| h.0 != 0 && unsafe { IsIconic(h.0 as _) != 0 });

    GameProbeSample {
        now_ms,
        d3d_fullscreen,
        borderless_fullscreen,
        foreground_is_own_browser,
        foreground_exe,
        all_browser_windows_minimized,
    }
}
