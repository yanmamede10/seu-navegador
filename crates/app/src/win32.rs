//! Win32 frameless multi-window host with custom resize borders, DWM shadow,
//! Windows Snap support, and dynamic drag initiation from the HTML shell.

use seu_core::{CoreSettings, Locale, SearchEngine, WindowId};
use seu_engine::cef_backend::{CefEngine, NativeWindowHost, SplitterAxis};
use seu_engine::{NativeWindowHandle, RectPx};
use seu_storage::Storage;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, Mutex, Once};
use tracing::{error, warn};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, ScreenToClient, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetWindowLongPtrW,
    GetWindowRect, IsZoomed, LoadCursorW, PostMessageW, RegisterClassExW, SetCursor,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, CS_DBLCLKS, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT,
    HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IDC_ARROW, IDC_SIZENS, IDC_SIZEWE, MINMAXINFO,
    NCCALCSIZE_PARAMS, SC_MOVE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, WM_CLOSE, WM_GETMINMAXINFO, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_SETCURSOR, WM_SIZE, WM_SYSCOMMAND,
    WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};

const WINDOW_CLASS: &str = "SeuNavegadorFramelessWindow";
const RESIZE_BORDER_PX: i32 = 6;
static REGISTER_ONCE: Once = Once::new();

fn wide_null(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub struct Win32WindowHost {
    hwnds: Mutex<HashMap<WindowId, isize>>,
    storage: Arc<Mutex<Option<Storage>>>,
}

impl Win32WindowHost {
    pub fn new(storage: Arc<Mutex<Option<Storage>>>) -> Self {
        Self {
            hwnds: Mutex::new(HashMap::new()),
            storage,
        }
    }

    fn ensure_class_registered() {
        REGISTER_ONCE.call_once(|| unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());
            let class_name = wide_null(WINDOW_CLASS);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
        });
    }

    fn hwnd_for(&self, window_id: WindowId) -> Option<HWND> {
        let raw = self.hwnds.lock().ok()?.get(&window_id).copied()?;
        if raw == 0 {
            None
        } else {
            Some(raw as HWND)
        }
    }
}

impl NativeWindowHost for Win32WindowHost {
    fn create_window(
        &self,
        window_id: WindowId,
        incognito: bool,
    ) -> Option<(NativeWindowHandle, RectPx)> {
        Self::ensure_class_registered();

        let title = if incognito {
            "Seu Navegador (Anônimo)"
        } else {
            "Seu Navegador"
        };
        let class_name = wide_null(WINDOW_CLASS);
        let title_w = wide_null(title);

        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;

        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(std::ptr::null());
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                style,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1280,
                820,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        };

        if hwnd.is_null() {
            error!("CreateWindowExW returned null");
            return None;
        }

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, window_id as isize);
            // Keep subtle native DWM drop shadow on frameless window.
            let margins = MARGINS {
                cxLeftWidth: 0,
                cxRightWidth: 0,
                cyTopHeight: 1,
                cyBottomHeight: 0,
            };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            );
            ShowWindow(hwnd, SW_SHOW);
        }

        if let Ok(mut guard) = self.hwnds.lock() {
            guard.insert(window_id, hwnd as isize);
        }

        self.get_window_rect(window_id)
    }

    fn get_window_rect(&self, window_id: WindowId) -> Option<(NativeWindowHandle, RectPx)> {
        let hwnd = self.hwnd_for(window_id)?;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe {
            if GetClientRect(hwnd, &mut rect) == 0 {
                return None;
            }
        }
        Some((
            NativeWindowHandle(hwnd as isize),
            RectPx {
                x: 0,
                y: 0,
                width: (rect.right - rect.left).max(1),
                height: (rect.bottom - rect.top).max(1),
            },
        ))
    }

    fn minimize_window(&self, window_id: WindowId) {
        if let Some(hwnd) = self.hwnd_for(window_id) {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        }
    }

    fn maximize_window(&self, window_id: WindowId) {
        if let Some(hwnd) = self.hwnd_for(window_id) {
            unsafe {
                if IsZoomed(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                } else {
                    ShowWindow(hwnd, SW_MAXIMIZE);
                }
            }
        }
    }

    fn close_window(&self, window_id: WindowId) {
        if let Some(hwnd) = self.hwnd_for(window_id) {
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
        }
    }

    fn start_window_drag(&self, window_id: WindowId) {
        if let Some(hwnd) = self.hwnd_for(window_id) {
            unsafe {
                let _ = SetForegroundWindow(hwnd);
                ReleaseCapture();
                // 0xF012 = SC_MOVE | HTCAPTION initiates native drag even when click started in a child CEF HWND.
                PostMessageW(hwnd, WM_SYSCOMMAND, (SC_MOVE | 0x0002) as WPARAM, 0);
            }
        }
    }

    fn on_settings_changed(&self, settings: &CoreSettings) {
        if let Ok(guard) = self.storage.lock() {
            if let Some(db) = guard.as_ref() {
                if let Err(err) = db.set_setting("locale", settings.locale.as_code()) {
                    warn!("Failed to persist locale setting: {err}");
                }
                let engine_str = match settings.search_engine {
                    SearchEngine::DuckDuckGo => "duck-duck-go",
                    SearchEngine::Google => "google",
                    SearchEngine::Brave => "brave",
                    SearchEngine::Startpage => "startpage",
                };
                if let Err(err) = db.set_setting("search_engine", engine_str) {
                    warn!("Failed to persist search_engine setting: {err}");
                }
                if let Err(err) = db.set_setting("active_theme_id", &settings.active_theme_id) {
                    warn!("Failed to persist active_theme_id setting: {err}");
                }
                let gm = if settings.game_mode_enabled { "1" } else { "0" };
                if let Err(err) = db.set_setting("game_mode_enabled", gm) {
                    warn!("Failed to persist game_mode_enabled setting: {err}");
                }
                if let Ok(json) = serde_json::to_string(&settings.shortcuts) {
                    if let Err(err) = db.set_setting("shortcuts", &json) {
                        warn!("Failed to persist shortcuts setting: {err}");
                    }
                }
                if let Ok(json) = serde_json::to_string(&settings.quick_shortcuts) {
                    if let Err(err) = db.set_setting("quick_shortcuts", &json) {
                        warn!("Failed to persist quick_shortcuts setting: {err}");
                    }
                }
                if let Ok(json) = serde_json::to_string(&settings.memory_policy) {
                    if let Err(err) = db.set_setting("memory_policy", &json) {
                        warn!("Failed to persist memory_policy setting: {err}");
                    }
                }
                let blk = if settings.blocker_enabled { "1" } else { "0" };
                if let Err(err) = db.set_setting("blocker_enabled", blk) {
                    warn!("Failed to persist blocker_enabled setting: {err}");
                }
                if let Ok(json) = serde_json::to_string(&settings.blocker_allowlist) {
                    if let Err(err) = db.set_setting("blocker_allowlist", &json) {
                        warn!("Failed to persist blocker_allowlist setting: {err}");
                    }
                }
                let rs = if settings.restore_session_on_startup {
                    "1"
                } else {
                    "0"
                };
                if let Err(err) = db.set_setting("restore_session_on_startup", rs) {
                    warn!("Failed to persist restore_session_on_startup setting: {err}");
                }
            }
        }
    }
}

pub fn load_persisted_settings(storage: &Option<Storage>) -> CoreSettings {
    let mut settings = CoreSettings::default();
    let Some(db) = storage.as_ref() else {
        return settings;
    };

    if let Ok(Some(code)) = db.get_setting("locale") {
        if let Some(locale) = Locale::from_code(&code) {
            settings.locale = locale;
        }
    }
    if let Ok(Some(engine)) = db.get_setting("search_engine") {
        settings.search_engine = match engine.as_str() {
            "google" => SearchEngine::Google,
            "brave" => SearchEngine::Brave,
            "startpage" => SearchEngine::Startpage,
            _ => SearchEngine::DuckDuckGo,
        };
    }
    if let Ok(Some(theme_id)) = db.get_setting("active_theme_id") {
        if !theme_id.trim().is_empty() {
            settings.active_theme_id = theme_id;
        }
    }
    if let Ok(Some(gm)) = db.get_setting("game_mode_enabled") {
        settings.game_mode_enabled = gm == "1";
    }
    if let Ok(Some(raw_shortcuts)) = db.get_setting("shortcuts") {
        if let Ok(parsed) =
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(&raw_shortcuts)
        {
            settings.shortcuts = seu_core::shortcuts::merge_with_defaults(&parsed);
        }
    }
    if let Ok(Some(raw_quick)) = db.get_setting("quick_shortcuts") {
        if let Ok(parsed) =
            serde_json::from_str::<Vec<seu_core::shortcuts::QuickShortcut>>(&raw_quick)
        {
            if !parsed.is_empty() {
                settings.quick_shortcuts = parsed;
            }
        }
    }
    if let Ok(Some(raw_mem)) = db.get_setting("memory_policy") {
        if let Ok(parsed) = serde_json::from_str::<seu_core::memory::MemoryPolicy>(&raw_mem) {
            settings.memory_policy = parsed;
        }
    }
    if let Ok(Some(blk)) = db.get_setting("blocker_enabled") {
        settings.blocker_enabled = blk != "0";
    }
    if let Ok(Some(raw_allow)) = db.get_setting("blocker_allowlist") {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&raw_allow) {
            settings.blocker_allowlist = parsed;
        }
    }
    if let Ok(Some(rs)) = db.get_setting("restore_session_on_startup") {
        settings.restore_session_on_startup = rs != "0";
    }

    settings
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize.x = 480;
            mmi.ptMinTrackSize.y = 320;

            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                rcWork: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                dwFlags: 0,
            };
            if GetMonitorInfoW(monitor, &mut info) != 0 {
                mmi.ptMaxPosition.x = info.rcWork.left - info.rcMonitor.left;
                mmi.ptMaxPosition.y = info.rcWork.top - info.rcMonitor.top;
                mmi.ptMaxSize.x = info.rcWork.right - info.rcWork.left;
                mmi.ptMaxSize.y = info.rcWork.bottom - info.rcWork.top;
            }
            0
        }
        WM_NCCALCSIZE => {
            if wparam != 0 {
                if IsZoomed(hwnd) != 0 {
                    let params = &mut *(lparam as *mut NCCALCSIZE_PARAMS);
                    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        rcMonitor: RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        },
                        rcWork: RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        },
                        dwFlags: 0,
                    };
                    if GetMonitorInfoW(monitor, &mut info) != 0 {
                        params.rgrc[0] = info.rcWork;
                    }
                }
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCHITTEST => {
            if IsZoomed(hwnd) != 0 {
                return HTCLIENT as LRESULT;
            }
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            let mut win_rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(hwnd, &mut win_rect) != 0 {
                let left = x < win_rect.left + RESIZE_BORDER_PX;
                let right = x >= win_rect.right - RESIZE_BORDER_PX;
                let top = y < win_rect.top + RESIZE_BORDER_PX;
                let bottom = y >= win_rect.bottom - RESIZE_BORDER_PX;

                let hit = match (left, right, top, bottom) {
                    (true, _, true, _) => HTTOPLEFT,
                    (_, true, true, _) => HTTOPRIGHT,
                    (true, _, _, true) => HTBOTTOMLEFT,
                    (_, true, _, true) => HTBOTTOMRIGHT,
                    (true, _, _, _) => HTLEFT,
                    (_, true, _, _) => HTRIGHT,
                    (_, _, true, _) => HTTOP,
                    (_, _, _, true) => HTBOTTOM,
                    _ => {
                        let mut pt = POINT { x, y };
                        ScreenToClient(hwnd, &mut pt);
                        HTCLIENT
                    }
                };
                return hit as LRESULT;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETCURSOR => {
            if (lparam & 0xFFFF) as u32 == HTCLIENT {
                let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
                let mut pt = POINT { x: 0, y: 0 };
                if window_id > 0 && GetCursorPos(&mut pt) != 0 {
                    ScreenToClient(hwnd, &mut pt);
                    if let Some(axis) = CefEngine::hit_test_splitter(window_id, pt.x, pt.y) {
                        let cursor = match axis {
                            SplitterAxis::SplitVertical | SplitterAxis::DevToolsVertical => {
                                IDC_SIZEWE
                            }
                            SplitterAxis::DevToolsHorizontal => IDC_SIZENS,
                        };
                        SetCursor(LoadCursorW(std::ptr::null_mut(), cursor));
                        return 1;
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {
            let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if window_id > 0 && CefEngine::begin_splitter_drag(window_id, x, y) {
                SetCapture(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if window_id > 0 && CefEngine::update_splitter_drag(window_id, x, y) {
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONUP => {
            let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
            if window_id > 0 && CefEngine::end_splitter_drag(window_id) {
                ReleaseCapture();
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SIZE => {
            let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
            let width = (lparam & 0xFFFF) as u16 as i32;
            let height = ((lparam >> 16) & 0xFFFF) as u16 as i32;
            if window_id > 0 && width > 0 && height > 0 {
                CefEngine::notify_window_resized(window_id, width, height);
            }
            0
        }
        WM_CLOSE => {
            let window_id = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as WindowId;
            if window_id > 0 {
                CefEngine::notify_window_closed(window_id);
            }
            DestroyWindow(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
