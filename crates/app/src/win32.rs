//! Win32 frameless multi-window host with custom resize borders, DWM shadow,
//! Windows Snap support, and dynamic drag initiation from the HTML shell.

use seu_core::{CoreSettings, Locale, SearchEngine, WindowId};
use seu_engine::cef_backend::{CefEngine, NativeWindowHost};
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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowRect, IsZoomed, LoadCursorW, PostMessageW, RegisterClassExW, SetForegroundWindow,
    SetWindowLongPtrW, ShowWindow, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    GWLP_USERDATA, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT, HTLEFT, HTRIGHT, HTTOP,
    HTTOPLEFT, HTTOPRIGHT, IDC_ARROW, NCCALCSIZE_PARAMS, SC_MOVE, SW_MAXIMIZE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOW, WM_CLOSE, WM_NCCALCSIZE, WM_NCHITTEST, WM_SIZE, WM_SYSCOMMAND,
    WNDCLASSEXW, WS_CLIPCHILDREN, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    WS_THICKFRAME,
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

        let style = WS_POPUP
            | WS_THICKFRAME
            | WS_MINIMIZEBOX
            | WS_MAXIMIZEBOX
            | WS_SYSMENU
            | WS_CLIPCHILDREN;

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

    settings
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => {
            if wparam != 0 {
                // Keep maximized windows aligned inside the monitor work area.
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
