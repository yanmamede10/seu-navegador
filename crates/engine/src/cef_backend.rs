//! CEF Alloy backend implementation for Windows x64.

use crate::assets::{resolve_seu_url, STRICT_SEU_CSP};
use crate::metrics::{
    capture_hwnd_thumbnail_jpeg, measure_browser_tree_memory, remove_tab_thumbnail,
};
use crate::{BrowserBackend, EngineConfig, EngineError, NativeWindowHandle, RectPx};
use cef::{
    wrapper::{
        byte_read_handler::{ByteReadHandler, ByteStream},
        message_router::{
            BrowserSideCallback, BrowserSideHandler, BrowserSideRouter, MessageRouterBrowserSide,
            MessageRouterBrowserSideHandlerCallbacks, MessageRouterConfig,
            MessageRouterRendererSide, MessageRouterRendererSideHandlerCallbacks,
            RendererSideRouter,
        },
        stream_resource_handler::StreamResourceHandler,
    },
    *,
};
use seu_blocker::{
    filter_lists_dir, needs_background_update, write_last_updated, BlockerService,
    UPSTREAM_LIST_URLS,
};
use seu_bridge::{handle_raw_request, is_privileged_origin, HostEffect};
use seu_core::{
    shortcuts::{match_shortcut_vk, ShortcutAction},
    BrowserCore, CoreSettings, TabId, WindowId, WindowSnapshot,
};
use seu_storage::Storage;
use seu_themes::ThemeRegistry;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, SetParent, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_HIDE, SW_SHOW,
};

const CEF_EVENTFLAG_SHIFT_DOWN: u32 = 1 << 1;
const CEF_EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;
const CEF_EVENTFLAG_ALT_DOWN: u32 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    Shell(WindowId),
    Overlay(WindowId),
    Tab { window_id: WindowId, tab_id: TabId },
}

impl ViewKind {
    fn window_id(self) -> WindowId {
        match self {
            Self::Shell(w) | Self::Overlay(w) | Self::Tab { window_id: w, .. } => w,
        }
    }
}

/// Host window operations supplied by `crates/app` (Win32 frameless windows).
pub trait NativeWindowHost: Send + Sync {
    fn create_window(
        &self,
        window_id: WindowId,
        incognito: bool,
    ) -> Option<(NativeWindowHandle, RectPx)>;
    fn get_window_rect(&self, window_id: WindowId) -> Option<(NativeWindowHandle, RectPx)>;
    fn minimize_window(&self, window_id: WindowId);
    fn maximize_window(&self, window_id: WindowId);
    fn close_window(&self, window_id: WindowId);
    fn start_window_drag(&self, window_id: WindowId);
    fn on_settings_changed(&self, settings: &CoreSettings);
}

struct WindowViews {
    shell: Option<Browser>,
    overlay: Option<Browser>,
    shell_height: i32,
    overlay_open: bool,
    overlay_opened_at: Instant,
    overlay_bounds: RectPx,
    incognito: bool,
}

struct SharedState {
    core: BrowserCore,
    themes: ThemeRegistry,
    storage: Arc<Mutex<Option<Storage>>>,
    blocker: Arc<RwLock<BlockerService>>,
    host: Arc<dyn NativeWindowHost>,
    router: Arc<BrowserSideRouter>,
    windows: HashMap<WindowId, WindowViews>,
    tab_browsers: HashMap<TabId, Browser>,
    incognito_contexts: HashMap<WindowId, RequestContext>,
    pending_scroll_restore: HashMap<TabId, (i32, i32)>,
    pending_sleep_batch: Option<(u32, usize, Vec<TabId>)>,
    probe_token: String,
    monitor_running: Arc<AtomicBool>,
}

static ENGINE_STATE: OnceLock<Arc<Mutex<SharedState>>> = OnceLock::new();

pub struct CefEngine {
    state: Arc<Mutex<SharedState>>,
}

impl CefEngine {
    /// Must be called at the very start of `main()`. Returns `Some(exit_code)`
    /// if this process is a CEF subprocess (renderer/GPU/utility).
    pub fn execute_subprocess_if_needed() -> Option<i32> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let args = cef::args::Args::new();
        let renderer_router = RendererSideRouter::new(MessageRouterConfig::default());
        let mut app = SeuCefApp::new(None, renderer_router);
        let code = execute_process(
            Some(args.as_main_args()),
            Some(&mut app),
            std::ptr::null_mut(),
        );
        if code >= 0 {
            Some(code)
        } else {
            None
        }
    }

    /// Initializes CEF on the browser process, creates the first window, and
    /// runs the CEF message loop until the last window closes.
    pub fn run(
        config: EngineConfig,
        initial_settings: CoreSettings,
        host: Arc<dyn NativeWindowHost>,
        storage: Arc<Mutex<Option<Storage>>>,
    ) -> Result<(), EngineError> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let args = cef::args::Args::new();

        let router = BrowserSideRouter::new(MessageRouterConfig::default());
        let mut core = BrowserCore::new();
        let mut blocker = BlockerService::new();
        blocker.configure(
            initial_settings.blocker_enabled,
            &initial_settings.blocker_allowlist,
        );
        let lists_dir = filter_lists_dir();
        let _ = blocker.reload_from_cache_dir(&lists_dir);
        core.settings = initial_settings;
        let themes = ThemeRegistry::load_builtin()
            .map_err(|e| EngineError::InitFailed(format!("failed to load builtin themes: {e}")))?;

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(42);
        let probe_token = format!("{nonce:x}");

        let shared = Arc::new(Mutex::new(SharedState {
            core,
            themes,
            storage,
            blocker: Arc::new(RwLock::new(blocker)),
            host,
            router: router.clone(),
            windows: HashMap::new(),
            tab_browsers: HashMap::new(),
            incognito_contexts: HashMap::new(),
            pending_scroll_restore: HashMap::new(),
            pending_sleep_batch: None,
            probe_token,
            monitor_running: Arc::new(AtomicBool::new(false)),
        }));

        let _ = ENGINE_STATE.set(shared.clone());

        let renderer_router = RendererSideRouter::new(MessageRouterConfig::default());
        let mut app = SeuCefApp::new(Some(shared.clone()), renderer_router);

        if let Err(err) = std::fs::create_dir_all(&config.root_cache_path) {
            warn!("Could not create CEF root_cache_path: {err}");
        }

        let cache_str = config.root_cache_path.to_string_lossy().to_string();
        let settings = Settings {
            no_sandbox: if config.enable_sandbox { 0 } else { 1 },
            root_cache_path: CefString::from(cache_str.as_str()),
            user_agent: CefString::from(config.user_agent.as_str()),
            ..Default::default()
        };

        let ok = initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        );
        if ok == 0 {
            return Err(EngineError::InitFailed("cef::initialize returned 0".into()));
        }

        run_message_loop();
        shutdown();
        Ok(())
    }

    pub fn notify_window_resized(window_id: WindowId, width: i32, height: i32) {
        let Some(state) = ENGINE_STATE.get() else {
            return;
        };
        if currently_on(ThreadId::UI) == 0 {
            let mut task = ResizeWindowTask::new(state.clone(), window_id, width, height);
            post_task(ThreadId::UI, Some(&mut task));
            return;
        }
        relayout_window(state, window_id, width, height);
    }

    pub fn notify_window_closed(window_id: WindowId) {
        let Some(state) = ENGINE_STATE.get() else {
            return;
        };
        if currently_on(ThreadId::UI) == 0 {
            let mut task = CloseWindowTask::new(state.clone(), window_id);
            post_task(ThreadId::UI, Some(&mut task));
            return;
        }
        handle_native_window_closed(state, window_id);
    }
}

impl BrowserBackend for CefEngine {
    fn create_shell_view(
        &self,
        window_id: WindowId,
        parent: NativeWindowHandle,
        bounds: RectPx,
    ) -> Result<(), EngineError> {
        spawn_child_browser(
            self.state.clone(),
            ViewKind::Shell(window_id),
            parent,
            bounds,
            &format!("seu://shell/?win={window_id}"),
            None,
        )
    }

    fn create_overlay_view(
        &self,
        window_id: WindowId,
        parent: NativeWindowHandle,
        bounds: RectPx,
    ) -> Result<(), EngineError> {
        spawn_child_browser(
            self.state.clone(),
            ViewKind::Overlay(window_id),
            parent,
            bounds,
            &format!("seu://overlay/?win={window_id}"),
            None,
        )
    }

    fn create_tab_view(
        &self,
        window_id: WindowId,
        tab_id: TabId,
        incognito: bool,
        parent: NativeWindowHandle,
        bounds: RectPx,
        url: &str,
    ) -> Result<(), EngineError> {
        let req_ctx = if incognito {
            get_or_create_incognito_context(&self.state, window_id)
        } else {
            None
        };
        let target_url = if url == seu_core::NEWTAB_URL {
            format!("seu://newtab/?win={window_id}")
        } else if url == "seu://settings" || url == "seu://settings/" {
            format!("seu://settings/?win={window_id}")
        } else {
            url.to_string()
        };
        spawn_child_browser(
            self.state.clone(),
            ViewKind::Tab { window_id, tab_id },
            parent,
            bounds,
            &target_url,
            req_ctx,
        )
    }

    fn navigate_tab(&self, tab_id: TabId, url: &str) {
        // Release the state lock BEFORE calling CEF, because `load_url` can synchronously
        // trigger `on_before_browse` / `on_loading_state_change` on the UI thread.
        let nav = self.state.lock().ok().and_then(|guard| {
            let browser = guard.tab_browsers.get(&tab_id)?.clone();
            let target = if url == seu_core::NEWTAB_URL {
                let win_id = guard.core.get_tab(tab_id).map(|t| t.window_id).unwrap_or(1);
                format!("seu://newtab/?win={win_id}")
            } else if url == "seu://settings" || url == "seu://settings/" {
                let win_id = guard.core.get_tab(tab_id).map(|t| t.window_id).unwrap_or(1);
                format!("seu://settings/?win={win_id}")
            } else {
                url.to_string()
            };
            Some((browser, target))
        });

        if let Some((browser, target)) = nav {
            if let Some(frame) = browser.main_frame() {
                let cef_url = CefString::from(target.as_str());
                frame.load_url(Some(&cef_url));
            }
        }
    }

    fn reload_tab(&self, tab_id: TabId) {
        let browser = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
        if let Some(browser) = browser {
            browser.reload();
        }
    }

    fn go_back(&self, tab_id: TabId) {
        let browser = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
        if let Some(browser) = browser {
            if browser.can_go_back() != 0 {
                browser.go_back();
                return;
            }
            // Fallback to core's preserved Back/Forward history stack (e.g. after waking from sleep).
            let fallback = self
                .state
                .lock()
                .ok()
                .and_then(|mut g| g.core.step_tab_history_back(tab_id));
            if let Some((wid, url)) = fallback {
                self.navigate_tab(tab_id, &url);
                broadcast_window_state(&self.state, wid);
            }
        }
    }

    fn go_forward(&self, tab_id: TabId) {
        let browser = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
        if let Some(browser) = browser {
            if browser.can_go_forward() != 0 {
                browser.go_forward();
                return;
            }
            let fallback = self
                .state
                .lock()
                .ok()
                .and_then(|mut g| g.core.step_tab_history_forward(tab_id));
            if let Some((wid, url)) = fallback {
                self.navigate_tab(tab_id, &url);
                broadcast_window_state(&self.state, wid);
            }
        }
    }

    fn close_tab_view(&self, tab_id: TabId) {
        let browser = self.state.lock().ok().and_then(|mut g| {
            g.pending_scroll_restore.remove(&tab_id);
            g.tab_browsers.remove(&tab_id)
        });
        remove_tab_thumbnail(tab_id);
        if let Some(b) = browser {
            if let Some(host) = b.host() {
                host.close_browser(1);
            }
        }
    }

    fn set_tab_visible(&self, tab_id: TabId, visible: bool, bounds: Option<RectPx>) {
        let browser = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
        if let Some(b) = browser {
            if let Some(host) = b.host() {
                host.was_hidden(if visible { 0 } else { 1 });
                let hwnd = host.window_handle();
                apply_child_hwnd_visibility(hwnd, visible, bounds, false);
            }
        }
    }

    fn resize_shell_view(&self, window_id: WindowId, bounds: RectPx) {
        let shell = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.windows.get(&window_id).and_then(|w| w.shell.clone()));
        if let Some(b) = shell {
            if let Some(host) = b.host() {
                apply_child_hwnd_visibility(host.window_handle(), true, Some(bounds), false);
            }
        }
    }

    fn show_overlay(&self, window_id: WindowId, bounds: RectPx, panel: &str) {
        let overlay = self.state.lock().ok().and_then(|mut g| {
            let win = g.windows.get_mut(&window_id)?;
            win.overlay_open = true;
            win.overlay_opened_at = Instant::now();
            win.overlay_bounds = bounds;
            win.overlay.clone()
        });
        if let Some(b) = overlay {
            if let Some(host) = b.host() {
                apply_child_hwnd_visibility(host.window_handle(), true, Some(bounds), true);
                host.set_focus(1);
            }
            push_js_event(
                &b,
                &serde_json::json!({ "type": "overlay", "panel": panel }),
            );
        }
    }

    fn hide_overlay(&self, window_id: WindowId) {
        let (overlay, shell) = self
            .state
            .lock()
            .ok()
            .map(|mut g| {
                let Some(win) = g.windows.get_mut(&window_id) else {
                    return (None, None);
                };
                win.overlay_open = false;
                (win.overlay.clone(), win.shell.clone())
            })
            .unwrap_or((None, None));
        if let Some(b) = overlay {
            if let Some(host) = b.host() {
                apply_child_hwnd_visibility(host.window_handle(), false, None, false);
            }
        }
        if let Some(s) = shell {
            push_js_event(
                &s,
                &serde_json::json!({ "type": "overlay", "panel": "closed" }),
            );
        }
    }

    fn close_window_views(&self, window_id: WindowId) {
        handle_native_window_closed(&self.state, window_id);
    }
}

fn get_or_create_incognito_context(
    state: &Arc<Mutex<SharedState>>,
    window_id: WindowId,
) -> Option<RequestContext> {
    if let Ok(guard) = state.lock() {
        if let Some(ctx) = guard.incognito_contexts.get(&window_id) {
            return Some(ctx.clone());
        }
    }
    // Empty cache_path creates an in-memory RequestContext (incognito mode).
    let settings = RequestContextSettings {
        persist_session_cookies: 0,
        ..Default::default()
    };
    let created = request_context_create_context(Some(&settings), None);
    if let (Some(ref ctx), Ok(mut guard)) = (&created, state.lock()) {
        guard.incognito_contexts.insert(window_id, ctx.clone());
    }
    created
}

fn spawn_child_browser(
    state: Arc<Mutex<SharedState>>,
    kind: ViewKind,
    parent: NativeWindowHandle,
    bounds: RectPx,
    url: &str,
    request_context: Option<RequestContext>,
) -> Result<(), EngineError> {
    let cef_rect = Rect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(1),
        height: bounds.height.max(1),
    };
    let parent_hwnd = sys::HWND((parent.0 as *mut std::ffi::c_void).cast());
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child(parent_hwnd, &cef_rect);

    let mut client = SeuClient::new(state, kind);
    let cef_url = CefString::from(url);
    let browser_settings = BrowserSettings::default();
    let mut req_ctx = request_context;

    let ok = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&cef_url),
        Some(&browser_settings),
        None,
        req_ctx.as_mut(),
    );
    if ok == 0 {
        return Err(EngineError::ViewFailed(format!(
            "failed to spawn browser view for {url}"
        )));
    }
    Ok(())
}

fn apply_child_hwnd_visibility(
    raw_hwnd: sys::cef_window_handle_t,
    visible: bool,
    bounds: Option<RectPx>,
    bring_to_top: bool,
) {
    let hwnd: HWND = raw_hwnd.0.cast();
    if hwnd.is_null() {
        return;
    }
    unsafe {
        if let Some(r) = bounds {
            let flags = if bring_to_top {
                SWP_NOACTIVATE
            } else {
                SWP_NOZORDER | SWP_NOACTIVATE
            };
            SetWindowPos(
                hwnd,
                if bring_to_top {
                    HWND_TOP
                } else {
                    std::ptr::null_mut()
                },
                r.x,
                r.y,
                r.width.max(1),
                r.height.max(1),
                flags,
            );
        }
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

fn push_js_event(browser: &Browser, value: &serde_json::Value) {
    let Ok(serialized) = serde_json::to_string(value) else {
        return;
    };
    if let Some(frame) = browser.main_frame() {
        let script =
            format!("if (window.__seuReceiveEvent) {{ window.__seuReceiveEvent({serialized}); }}");
        let code = CefString::from(script.as_str());
        let url = CefString::from("seu://internal/event");
        frame.execute_java_script(Some(&code), Some(&url), 0);
    }
}

fn broadcast_window_state(state: &Arc<Mutex<SharedState>>, window_id: WindowId) {
    let (snapshot, shell_browser, tab_browsers) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let Ok(snap) = guard.core.window_snapshot(window_id) else {
            return;
        };
        let shell = guard.windows.get(&window_id).and_then(|w| w.shell.clone());
        let tabs: Vec<Browser> = snap
            .tabs
            .iter()
            .filter_map(|t| guard.tab_browsers.get(&t.id).cloned())
            .collect();
        (snap, shell, tabs)
    };

    let payload = serde_json::json!({ "type": "state", "snapshot": snapshot });
    if let Some(shell) = shell_browser {
        push_js_event(&shell, &payload);
    }
    for b in tab_browsers {
        if let Some(frame) = b.main_frame() {
            let u = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&u) {
                push_js_event(&b, &payload);
            }
        }
    }
}

fn broadcast_theme_all(state: &Arc<Mutex<SharedState>>) {
    let (theme, chrome_views, tab_browsers) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let Some(active) = guard
            .themes
            .get(&guard.core.settings.active_theme_id)
            .cloned()
        else {
            return;
        };
        let mut chrome = Vec::new();
        for win in guard.windows.values() {
            if let Some(s) = &win.shell {
                chrome.push(s.clone());
            }
            if let Some(o) = &win.overlay {
                chrome.push(o.clone());
            }
        }
        let tabs: Vec<Browser> = guard.tab_browsers.values().cloned().collect();
        (active, chrome, tabs)
    };

    let payload = serde_json::json!({ "type": "theme", "theme": theme });
    for b in chrome_views {
        push_js_event(&b, &payload);
    }
    for b in tab_browsers {
        if let Some(frame) = b.main_frame() {
            let u = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&u) {
                push_js_event(&b, &payload);
            }
        }
    }
}

fn broadcast_settings_all(state: &Arc<Mutex<SharedState>>) {
    let (settings, shell_and_overlay, tab_browsers, host) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let mut chrome_views = Vec::new();
        for win in guard.windows.values() {
            if let Some(s) = &win.shell {
                chrome_views.push(s.clone());
            }
            if let Some(o) = &win.overlay {
                chrome_views.push(o.clone());
            }
        }
        let tabs: Vec<Browser> = guard.tab_browsers.values().cloned().collect();
        (
            guard.core.settings.clone(),
            chrome_views,
            tabs,
            guard.host.clone(),
        )
    };

    host.on_settings_changed(&settings);
    let payload = serde_json::json!({ "type": "settings", "settings": settings });
    for b in shell_and_overlay {
        push_js_event(&b, &payload);
    }
    for b in tab_browsers {
        if let Some(frame) = b.main_frame() {
            let u = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&u) {
                push_js_event(&b, &payload);
            }
        }
    }
}

fn broadcast_blocker_all(state: &Arc<Mutex<SharedState>>) {
    let (stats, chrome_views, tab_browsers) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let stats = guard.blocker.read().map(|b| b.stats()).unwrap_or_default();
        let mut chrome = Vec::new();
        for win in guard.windows.values() {
            if let Some(s) = &win.shell {
                chrome.push(s.clone());
            }
            if let Some(o) = &win.overlay {
                chrome.push(o.clone());
            }
        }
        let tabs: Vec<Browser> = guard.tab_browsers.values().cloned().collect();
        (stats, chrome, tabs)
    };

    let payload = serde_json::json!({ "type": "blocker", "stats": stats });
    for b in chrome_views {
        push_js_event(&b, &payload);
    }
    for b in tab_browsers {
        if let Some(frame) = b.main_frame() {
            let u = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&u) {
                push_js_event(&b, &payload);
            }
        }
    }
}

fn broadcast_memory_all(state: &Arc<Mutex<SharedState>>) {
    let (stats, chrome_views, tab_browsers) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let mut chrome = Vec::new();
        for win in guard.windows.values() {
            if let Some(s) = &win.shell {
                chrome.push(s.clone());
            }
            if let Some(o) = &win.overlay {
                chrome.push(o.clone());
            }
        }
        let tabs: Vec<Browser> = guard.tab_browsers.values().cloned().collect();
        (guard.core.memory_stats.clone(), chrome, tabs)
    };

    let payload = serde_json::json!({ "type": "memory", "stats": stats });
    for b in chrome_views {
        push_js_event(&b, &payload);
    }
    for b in tab_browsers {
        if let Some(frame) = b.main_frame() {
            let u = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&u) {
                push_js_event(&b, &payload);
            }
        }
    }
}

fn relayout_window(state: &Arc<Mutex<SharedState>>, window_id: WindowId, width: i32, height: i32) {
    let engine = CefEngine {
        state: state.clone(),
    };

    let (shell_height, active_tab_id, all_tab_ids, overlay_open, overlay_bounds) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let Some(win) = guard.windows.get(&window_id) else {
            return;
        };
        let snap: Option<WindowSnapshot> = guard.core.window_snapshot(window_id).ok();
        let active = snap.as_ref().and_then(|s| s.active_tab_id);
        let ids: Vec<TabId> = snap
            .map(|s| s.tabs.into_iter().map(|t| t.id).collect())
            .unwrap_or_default();
        (
            win.shell_height,
            active,
            ids,
            win.overlay_open,
            win.overlay_bounds,
        )
    };

    let safe_w = width.max(200);
    let safe_h = height.max(120);
    let sh = shell_height.clamp(48, safe_h.saturating_sub(40).max(48));

    engine.resize_shell_view(
        window_id,
        RectPx {
            x: 0,
            y: 0,
            width: safe_w,
            height: sh,
        },
    );

    let content_rect = RectPx {
        x: 0,
        y: sh,
        width: safe_w,
        height: (safe_h - sh).max(1),
    };

    for tid in all_tab_ids {
        let is_active = Some(tid) == active_tab_id;
        engine.set_tab_visible(
            tid,
            is_active,
            if is_active { Some(content_rect) } else { None },
        );
    }

    if overlay_open {
        let clamped_x = overlay_bounds
            .x
            .clamp(0, (safe_w - overlay_bounds.width).max(0));
        let updated = RectPx {
            x: clamped_x,
            y: sh + 4,
            width: overlay_bounds.width,
            height: overlay_bounds.height,
        };
        engine.show_overlay(window_id, updated, "menu");
    }
}

fn create_full_browser_window(
    state: &Arc<Mutex<SharedState>>,
    window_id: WindowId,
    incognito: bool,
    initial_tab_id: TabId,
    initial_url: &str,
) {
    let host = match state.lock() {
        Ok(g) => g.host.clone(),
        Err(_) => return,
    };

    let Some((parent_handle, rect)) = host.create_window(window_id, incognito) else {
        error!("Failed to create native Win32 window for window_id={window_id}");
        return;
    };

    let default_shell_height = 140;
    if let Ok(mut guard) = state.lock() {
        guard.windows.insert(
            window_id,
            WindowViews {
                shell: None,
                overlay: None,
                shell_height: default_shell_height,
                overlay_open: false,
                overlay_opened_at: Instant::now(),
                overlay_bounds: RectPx {
                    x: 10,
                    y: default_shell_height + 4,
                    width: 270,
                    height: 310,
                },
                incognito,
            },
        );
    }

    let engine = CefEngine {
        state: state.clone(),
    };

    if let Err(err) = engine.create_shell_view(
        window_id,
        parent_handle,
        RectPx {
            x: 0,
            y: 0,
            width: rect.width,
            height: default_shell_height,
        },
    ) {
        error!("Shell view error: {err}");
    }

    if let Err(err) = engine.create_overlay_view(
        window_id,
        parent_handle,
        RectPx {
            x: 0,
            y: default_shell_height,
            width: 270,
            height: 310,
        },
    ) {
        error!("Overlay view error: {err}");
    }

    if let Err(err) = engine.create_tab_view(
        window_id,
        initial_tab_id,
        incognito,
        parent_handle,
        RectPx {
            x: 0,
            y: default_shell_height,
            width: rect.width,
            height: (rect.height - default_shell_height).max(1),
        },
        initial_url,
    ) {
        error!("Initial tab view error: {err}");
    }
}

fn handle_native_window_closed(state: &Arc<Mutex<SharedState>>, window_id: WindowId) {
    let (browsers_to_close, should_quit) = {
        let Ok(mut guard) = state.lock() else {
            return;
        };
        let mut to_close = Vec::new();
        if let Some(win) = guard.windows.remove(&window_id) {
            if let Some(s) = win.shell {
                to_close.push(s);
            }
            if let Some(o) = win.overlay {
                to_close.push(o);
            }
        }
        guard.incognito_contexts.remove(&window_id);
        if let Ok(tab_ids) = guard.core.close_window(window_id) {
            for tid in tab_ids {
                if let Some(b) = guard.tab_browsers.remove(&tid) {
                    to_close.push(b);
                }
            }
        }
        let no_windows_left = guard.core.window_ids().is_empty();
        (to_close, no_windows_left)
    };

    for b in browsers_to_close {
        if let Some(host) = b.host() {
            host.close_browser(1);
        }
    }

    if should_quit {
        if let Ok(guard) = state.lock() {
            guard.monitor_running.store(false, Ordering::SeqCst);
        }
        quit_message_loop();
    }
}

fn execute_effects(state: &Arc<Mutex<SharedState>>, effects: Vec<HostEffect>) {
    let engine = CefEngine {
        state: state.clone(),
    };
    let host = match state.lock() {
        Ok(g) => g.host.clone(),
        Err(_) => return,
    };

    for effect in effects {
        match effect {
            HostEffect::CreateNativeWindow {
                window_id,
                incognito,
                initial_tab_id,
                initial_url,
            } => {
                create_full_browser_window(
                    state,
                    window_id,
                    incognito,
                    initial_tab_id,
                    &initial_url,
                );
            }
            HostEffect::MinimizeWindow { window_id } => host.minimize_window(window_id),
            HostEffect::MaximizeWindow { window_id } => host.maximize_window(window_id),
            HostEffect::CloseWindow { window_id } => host.close_window(window_id),
            HostEffect::StartWindowDrag { window_id } => host.start_window_drag(window_id),
            HostEffect::SetShellHeight { window_id, height } => {
                if let Ok(mut g) = state.lock() {
                    if let Some(w) = g.windows.get_mut(&window_id) {
                        w.shell_height = height as i32;
                    }
                }
                if let Some((_, rect)) = host.get_window_rect(window_id) {
                    relayout_window(state, window_id, rect.width, rect.height);
                }
            }
            HostEffect::CreateTabView {
                window_id,
                tab_id,
                url,
            } => {
                let incognito = state
                    .lock()
                    .ok()
                    .and_then(|g| g.windows.get(&window_id).map(|w| w.incognito))
                    .unwrap_or(false);
                let shell_h = state
                    .lock()
                    .ok()
                    .and_then(|g| g.windows.get(&window_id).map(|w| w.shell_height))
                    .unwrap_or(112);
                if let Some((parent, rect)) = host.get_window_rect(window_id) {
                    if let Err(err) = engine.create_tab_view(
                        window_id,
                        tab_id,
                        incognito,
                        parent,
                        RectPx {
                            x: 0,
                            y: shell_h,
                            width: rect.width,
                            height: (rect.height - shell_h).max(1),
                        },
                        &url,
                    ) {
                        error!("CreateTabView failed: {err}");
                    }
                }
            }
            HostEffect::ActivateTabView { window_id, .. } => {
                if let Some((_, rect)) = host.get_window_rect(window_id) {
                    relayout_window(state, window_id, rect.width, rect.height);
                }
            }
            HostEffect::CloseTabView { tab_id, .. } => engine.close_tab_view(tab_id),
            HostEffect::SleepTabView { tab_id, .. } => {
                sleep_tab_browser(state, tab_id);
            }
            HostEffect::WakeTabView {
                window_id,
                tab_id,
                url,
                scroll_x,
                scroll_y,
            } => {
                let (incognito, shell_h) = state
                    .lock()
                    .ok()
                    .map(|mut g| {
                        if scroll_x > 0 || scroll_y > 0 {
                            g.pending_scroll_restore
                                .insert(tab_id, (scroll_x, scroll_y));
                        }
                        let inc = g
                            .windows
                            .get(&window_id)
                            .map(|w| w.incognito)
                            .unwrap_or(false);
                        let sh = g
                            .windows
                            .get(&window_id)
                            .map(|w| w.shell_height)
                            .unwrap_or(112);
                        (inc, sh)
                    })
                    .unwrap_or((false, 112));
                if let Some((parent, rect)) = host.get_window_rect(window_id) {
                    if let Err(err) = engine.create_tab_view(
                        window_id,
                        tab_id,
                        incognito,
                        parent,
                        RectPx {
                            x: 0,
                            y: shell_h,
                            width: rect.width,
                            height: (rect.height - shell_h).max(1),
                        },
                        &url,
                    ) {
                        error!("WakeTabView failed: {err}");
                    }
                }
            }
            HostEffect::NavigateTabView { tab_id, url } => engine.navigate_tab(tab_id, &url),
            HostEffect::ReloadTabView { tab_id } => engine.reload_tab(tab_id),
            HostEffect::HardReloadTabView { tab_id } => {
                let browser = state
                    .lock()
                    .ok()
                    .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
                if let Some(b) = browser {
                    b.reload_ignore_cache();
                }
            }
            HostEffect::GoBackTabView { tab_id } => engine.go_back(tab_id),
            HostEffect::GoForwardTabView { tab_id } => engine.go_forward(tab_id),
            HostEffect::FocusOmnibox { window_id } => {
                let shell = state
                    .lock()
                    .ok()
                    .and_then(|g| g.windows.get(&window_id).and_then(|w| w.shell.clone()));
                if let Some(s) = shell {
                    if let Some(h) = s.host() {
                        h.set_focus(1);
                    }
                    push_js_event(&s, &serde_json::json!({ "type": "focusOmnibox" }));
                }
            }
            HostEffect::ZoomTabView {
                tab_id,
                delta_steps,
            } => {
                let browser = state
                    .lock()
                    .ok()
                    .and_then(|g| g.tab_browsers.get(&tab_id).cloned());
                if let Some(b) = browser {
                    if let Some(h) = b.host() {
                        let next_zoom = if delta_steps == 0 {
                            0.0
                        } else {
                            (h.zoom_level() + f64::from(delta_steps) * 0.5).clamp(-3.0, 5.0)
                        };
                        h.set_zoom_level(next_zoom);
                    }
                }
            }
            HostEffect::OpenOverlay {
                window_id,
                panel,
                x,
                y,
                width,
                height,
            } => {
                engine.show_overlay(
                    window_id,
                    RectPx {
                        x,
                        y,
                        width: width as i32,
                        height: height as i32,
                    },
                    &panel,
                );
            }
            HostEffect::CloseOverlay { window_id } => engine.hide_overlay(window_id),
            HostEffect::BroadcastState { window_id } => broadcast_window_state(state, window_id),
            HostEffect::BroadcastSettings => broadcast_settings_all(state),
            HostEffect::BroadcastTheme => broadcast_theme_all(state),
            HostEffect::BroadcastMemory => broadcast_memory_all(state),
            HostEffect::BroadcastBlocker => broadcast_blocker_all(state),
            HostEffect::UpdateBlockerLists => spawn_blocker_list_updater(state.clone(), true),
        }
    }
}

fn sleep_tab_browser(state: &Arc<Mutex<SharedState>>, tab_id: TabId) {
    let before_mem = measure_browser_tree_memory();
    let (browser, should_spawn_delta_timer) = match state.lock() {
        Ok(mut g) => {
            let b = g.tab_browsers.remove(&tab_id);
            let spawn_timer = if b.is_some() {
                match g.pending_sleep_batch.as_mut() {
                    Some((_, _, ids)) => {
                        if !ids.contains(&tab_id) {
                            ids.push(tab_id);
                        }
                        false
                    }
                    None => {
                        g.pending_sleep_batch = Some((
                            before_mem.total_private_mb(),
                            before_mem.process_count,
                            vec![tab_id],
                        ));
                        true
                    }
                }
            } else {
                false
            };
            (b, spawn_timer)
        }
        Err(_) => return,
    };

    let Some(browser) = browser else {
        return;
    };

    if let Some(host) = browser.host() {
        let hwnd_raw = host.window_handle().0 as isize;
        let captured = capture_hwnd_thumbnail_jpeg(hwnd_raw, tab_id);
        if captured {
            if let Ok(mut g) = state.lock() {
                g.core.set_tab_thumbnail_ready(tab_id, true);
            }
        }
        host.close_browser(1);
    }

    if should_spawn_delta_timer {
        // Measure ONE coalesced OS Private Bytes delta ~1.5s after closing the batch of
        // CEF tab views so multi-tab sleeps never double-count the same RAM drop.
        let state_clone = state.clone();
        let _ = std::thread::Builder::new()
            .name("seu-sleep-delta".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                let mut task = MeasureSleepDeltaTask::new(state_clone);
                post_task(ThreadId::UI, Some(&mut task));
            });
    }
}

fn spawn_blocker_list_updater(state: Arc<Mutex<SharedState>>, force: bool) {
    let lists_dir = filter_lists_dir();
    if !force && !needs_background_update(&lists_dir) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("seu-blocker-update".into())
        .spawn(move || {
            let mut any_downloaded = false;
            for (filename, url) in UPSTREAM_LIST_URLS {
                let target = lists_dir.join(filename);
                let tmp = lists_dir.join(format!("{filename}.tmp"));
                let status = std::process::Command::new("curl.exe")
                    .args(["-sSfL", "--max-time", "20", url, "-o"])
                    .arg(&tmp)
                    .status();
                if let Ok(st) = status {
                    if st.success() && std::fs::rename(&tmp, &target).is_ok() {
                        any_downloaded = true;
                    } else {
                        let _ = std::fs::remove_file(&tmp);
                    }
                }
            }
            if any_downloaded {
                write_last_updated(&lists_dir);
                let blocker_arc = state.lock().ok().map(|g| g.blocker.clone());
                if let Some(blocker_arc) = blocker_arc {
                    if let Ok(mut b) = blocker_arc.write() {
                        let count = b.reload_from_cache_dir(&lists_dir);
                        info!("Updated blocker filter lists ({count} active)");
                    }
                }
                let mut task = BroadcastBlockerTask::new(state);
                post_task(ThreadId::UI, Some(&mut task));
            }
        });
}

struct SeuBridgeQueryHandler {
    state: Arc<Mutex<SharedState>>,
}

impl BrowserSideHandler for SeuBridgeQueryHandler {
    fn on_query_str(
        &self,
        _browser: Option<Browser>,
        frame: Option<Frame>,
        _query_id: i64,
        request: &str,
        _persistent: bool,
        callback: Arc<Mutex<dyn BrowserSideCallback>>,
    ) -> bool {
        let origin_url = frame
            .map(|f| CefString::from(&f.url()).to_string())
            .unwrap_or_default();

        let result = {
            let Ok(mut guard) = self.state.lock() else {
                if let Ok(cb) = callback.lock() {
                    cb.failure(500, "internal state lock error");
                }
                return true;
            };
            let SharedState {
                core,
                themes,
                storage,
                blocker,
                ..
            } = &mut *guard;
            let storage_guard = storage.lock().ok();
            let db_ref = storage_guard.as_ref().and_then(|g| g.as_ref());
            let res = match blocker.write() {
                Ok(mut blocker_guard) => handle_raw_request(
                    &origin_url,
                    request,
                    core,
                    themes,
                    db_ref,
                    &mut blocker_guard,
                ),
                Err(_) => Err(seu_bridge::BridgeError::InvalidArgument(
                    "blocker lock error".into(),
                )),
            };
            drop(storage_guard);
            res
        };

        match result {
            Ok(output) => {
                let json = serde_json::to_string(&output.response)
                    .unwrap_or_else(|_| r#"{"kind":"Ok"}"#.to_string());
                if let Ok(cb) = callback.lock() {
                    cb.success_str(&json);
                }
                if !output.effects.is_empty() {
                    // Always defer effects to the next UI-thread tick so `BrowserSideRouter`
                    // releases its internal locks before any navigation/close callbacks fire.
                    let mut task = ExecuteEffectsTask::new(
                        self.state.clone(),
                        Arc::new(Mutex::new(Some(output.effects))),
                    );
                    post_task(ThreadId::UI, Some(&mut task));
                }
            }
            Err(err) => {
                if let Ok(cb) = callback.lock() {
                    cb.failure(400, &err.to_string());
                }
            }
        }
        true
    }
}

wrap_scheme_handler_factory! {
    struct SeuSchemeHandlerFactory;

    impl SchemeHandlerFactory {
        fn create(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _scheme_name: Option<&CefString>,
            request: Option<&mut Request>,
        ) -> Option<ResourceHandler> {
            let url = request
                .map(|r| CefString::from(&r.url()).to_string())
                .unwrap_or_default();
            let resolved = resolve_seu_url(&url);

            let mut read_handler =
                ByteReadHandler::new(Arc::new(Mutex::new(ByteStream::new(resolved.bytes))));
            let stream = stream_reader_create_for_handler(Some(&mut read_handler));

            let mut headers = CefStringMultimap::new();
            headers.append("Content-Security-Policy", STRICT_SEU_CSP);
            headers.append("Access-Control-Allow-Origin", "*");
            if resolved.mime_type.starts_with("text/")
                || resolved.mime_type == "application/javascript"
                || resolved.mime_type == "application/json"
            {
                headers.append(
                    "Content-Type",
                    &format!("{}; charset=utf-8", resolved.mime_type),
                );
            }

            Some(StreamResourceHandler::new(
                resolved.status_code,
                resolved.status_text.to_string(),
                resolved.mime_type.to_string(),
                Some(headers),
                stream,
            ))
        }
    }
}

wrap_app! {
    struct SeuCefApp {
        state: Option<Arc<Mutex<SharedState>>>,
        renderer_router: Arc<RendererSideRouter>,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            let Some(cmd) = command_line else {
                return;
            };
            // Documented in docs/adr/002-chromium-memory-flags.md:
            // Disable background services and BackForwardCache heap retention to reduce
            // baseline RAM without breaking web apps.
            for flag in [
                "disable-background-networking",
                "disable-component-update",
                "disable-domain-reliability",
                "disable-sync",
                "disable-breakpad",
                "metrics-recording-only",
            ] {
                let sw = CefString::from(flag);
                cmd.append_switch(Some(&sw));
            }
            let disable_feat = CefString::from("disable-features");
            let feat_val = CefString::from("BackForwardCache");
            cmd.append_switch_with_value(Some(&disable_feat), Some(&feat_val));
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            let Some(registrar) = registrar else {
                return;
            };
            let options = SchemeOptions::STANDARD.get_raw()
                | SchemeOptions::SECURE.get_raw()
                | SchemeOptions::CORS_ENABLED.get_raw()
                | SchemeOptions::FETCH_ENABLED.get_raw();
            let scheme = CefString::from("seu");
            registrar.add_custom_scheme(Some(&scheme), options);
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            self.state
                .as_ref()
                .map(|s| SeuBrowserProcessHandler::new(s.clone()))
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(SeuRenderProcessHandler::new(self.renderer_router.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct SeuBrowserProcessHandler {
        state: Arc<Mutex<SharedState>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            info!("CEF context initialized (Alloy style)");

            let mut factory = SeuSchemeHandlerFactory::new();
            let scheme = CefString::from("seu");
            let domain = CefString::from("");
            register_scheme_handler_factory(Some(&scheme), Some(&domain), Some(&mut factory));

            let (router, window_id, tab_id, initial_url) = {
                let Ok(mut guard) = self.state.lock() else {
                    return;
                };
                let router = guard.router.clone();
                let (wid, tid) = guard.core.create_window(false, None);
                let url = guard
                    .core
                    .get_tab(tid)
                    .map(|t| t.url.clone())
                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                (router, wid, tid, url)
            };

            router.add_handler(
                Arc::new(SeuBridgeQueryHandler {
                    state: self.state.clone(),
                }),
                true,
            );

            create_full_browser_window(&self.state, window_id, false, tab_id, &initial_url);
            start_memory_monitor_thread(self.state.clone());
            spawn_blocker_list_updater(self.state.clone(), false);
        }

        fn on_already_running_app_relaunch(
            &self,
            _command_line: Option<&mut CommandLine>,
            _current_directory: Option<&CefString>,
        ) -> i32 {
            let created = {
                let Ok(mut guard) = self.state.lock() else {
                    return 1;
                };
                let (wid, tid) = guard.core.create_window(false, None);
                let url = guard
                    .core
                    .get_tab(tid)
                    .map(|t| t.url.clone())
                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                (wid, tid, url)
            };
            create_full_browser_window(&self.state, created.0, false, created.1, &created.2);
            1
        }
    }
}

wrap_render_process_handler! {
    struct SeuRenderProcessHandler {
        router: Arc<RendererSideRouter>,
    }

    impl RenderProcessHandler {
        fn on_context_created(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            let frame_url = frame
                .as_ref()
                .map(|f| CefString::from(&f.url()).to_string())
                .unwrap_or_default();

            // Strict security boundary: only `seu://` origins receive the bridge.
            if !is_privileged_origin(&frame_url) {
                return;
            }

            self.router
                .on_context_created(browser.cloned(), frame.cloned(), context.cloned());
        }

        fn on_context_released(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            context: Option<&mut V8Context>,
        ) {
            self.router
                .on_context_released(browser.cloned(), frame.cloned(), context.cloned());
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let handled = self.router.on_process_message_received(
                browser.cloned(),
                frame.cloned(),
                Some(source_process),
                message.cloned(),
            );
            if handled {
                1
            } else {
                0
            }
        }
    }
}

wrap_client! {
    struct SeuClient {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(SeuDisplayHandler::new(self.state.clone(), self.kind))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(SeuLoadHandler::new(self.state.clone(), self.kind))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(SeuLifeSpanHandler::new(self.state.clone(), self.kind))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(SeuRequestHandler::new(self.state.clone(), self.kind))
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(SeuFocusHandler::new(self.state.clone(), self.kind))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(SeuKeyboardHandler::new(self.state.clone(), self.kind))
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            let router = match self.state.lock() {
                Ok(g) => g.router.clone(),
                Err(_) => return 0,
            };
            if router.on_process_message_received(
                browser.cloned(),
                frame.cloned(),
                source_process,
                message.cloned(),
            ) {
                1
            } else {
                0
            }
        }
    }
}

wrap_display_handler! {
    struct SeuDisplayHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            let ViewKind::Tab { window_id, tab_id } = self.kind else {
                return;
            };
            let title_str = title.map(|t| t.to_string()).unwrap_or_default();
            if let Ok(mut guard) = self.state.lock() {
                let _ = guard
                    .core
                    .update_tab_navigation(tab_id, None, Some(title_str), None, None, None);
            }
            broadcast_window_state(&self.state, window_id);
        }

        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            let ViewKind::Tab { window_id, tab_id } = self.kind else {
                return;
            };
            if frame.as_ref().map(|f| f.is_main()).unwrap_or(0) == 0 {
                return;
            }
            let raw_url = url.map(|u| u.to_string()).unwrap_or_default();
            let display_url = if raw_url.starts_with("seu://newtab/") {
                seu_core::NEWTAB_URL.to_string()
            } else if raw_url.starts_with("seu://settings/") {
                "seu://settings/".to_string()
            } else {
                raw_url
            };
            let can_back = browser.as_ref().map(|b| b.can_go_back() != 0);
            let can_fwd = browser.as_ref().map(|b| b.can_go_forward() != 0);
            if let Ok(mut guard) = self.state.lock() {
                let _ = guard.core.update_tab_navigation(
                    tab_id,
                    Some(display_url),
                    None,
                    None,
                    can_back,
                    can_fwd,
                );
            }
            broadcast_window_state(&self.state, window_id);
        }

        fn on_media_access_change(
            &self,
            _browser: Option<&mut Browser>,
            has_video_access: ::std::os::raw::c_int,
            has_audio_access: ::std::os::raw::c_int,
        ) {
            let ViewKind::Tab { window_id, tab_id } = self.kind else {
                return;
            };
            if let Ok(mut guard) = self.state.lock() {
                guard
                    .core
                    .set_tab_media_capture(tab_id, has_video_access != 0 || has_audio_access != 0);
            }
            broadcast_window_state(&self.state, window_id);
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            _level: LogSeverity,
            message: Option<&CefString>,
            _source: Option<&CefString>,
            _line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let ViewKind::Tab { tab_id, .. } = self.kind else {
                return 0;
            };
            let msg = message.map(|m| m.to_string()).unwrap_or_default();
            let prefix = self
                .state
                .lock()
                .ok()
                .map(|g| format!("__SEU_PROBE__{}:", g.probe_token))
                .unwrap_or_default();
            if prefix.is_empty() {
                return 0;
            }
            let Some(json_str) = msg.strip_prefix(&prefix) else {
                return 0;
            };
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                let dirty = val.get("dirty").and_then(|v| v.as_bool());
                let beforeunload = val.get("beforeunload").and_then(|v| v.as_bool());
                let sx = val.get("sx").and_then(|v| v.as_i64()).map(|n| n as i32);
                let sy = val.get("sy").and_then(|v| v.as_i64()).map(|n| n as i32);
                let audible = val.get("audible").and_then(|v| v.as_bool());

                let changed_win = self.state.lock().ok().and_then(|mut g| {
                    g.core
                        .update_tab_probe(tab_id, dirty, beforeunload, sx, sy, audible)
                });
                if let Some(wid) = changed_win {
                    broadcast_window_state(&self.state, wid);
                }
            }
            // Suppress internal probe messages from the console.
            1
        }
    }
}

wrap_load_handler! {
    struct SeuLoadHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            _browser: Option<&mut Browser>,
            is_loading: i32,
            can_go_back: i32,
            can_go_forward: i32,
        ) {
            let ViewKind::Tab { window_id, tab_id } = self.kind else {
                return;
            };
            if let Ok(mut guard) = self.state.lock() {
                let _ = guard.core.update_tab_navigation(
                    tab_id,
                    None,
                    None,
                    Some(is_loading != 0),
                    Some(can_go_back != 0),
                    Some(can_go_forward != 0),
                );
            }
            broadcast_window_state(&self.state, window_id);
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            let ViewKind::Tab { tab_id, .. } = self.kind else {
                return;
            };
            let Some(frame) = frame else {
                return;
            };
            if frame.is_main() == 0 {
                return;
            }
            let url = CefString::from(&frame.url()).to_string();
            if is_privileged_origin(&url) {
                return;
            }

            let (token, pending_scroll) = self
                .state
                .lock()
                .ok()
                .map(|mut g| {
                    (
                        g.probe_token.clone(),
                        g.pending_scroll_restore.remove(&tab_id),
                    )
                })
                .unwrap_or_default();

            if !token.is_empty() {
                let probe_js = build_tab_probe_script(&token);
                let code = CefString::from(probe_js.as_str());
                let script_url = CefString::from("seu://internal/probe");
                frame.execute_java_script(Some(&code), Some(&script_url), 0);
            }

            if let Some((sx, sy)) = pending_scroll {
                // Retry scroll restoration briefly after load to handle SPAs & lazy-loaded layouts.
                let scroll_js = format!(
                    "(function(){{var x={sx},y={sy};function go(){{window.scrollTo(x,y);}}go();setTimeout(go,250);setTimeout(go,850);}})();"
                );
                let code = CefString::from(scroll_js.as_str());
                let script_url = CefString::from("seu://internal/scroll");
                frame.execute_java_script(Some(&code), Some(&script_url), 0);
            }

            // Apply adblock cosmetic hiding rules + `##+js(...)` scriptlets.
            let cosmetic = self
                .state
                .lock()
                .ok()
                .map(|g| g.blocker.clone())
                .and_then(|b| b.read().ok().map(|guard| guard.cosmetic_resources_for_url(&url)));

            if let Some(cos) = cosmetic {
                if !cos.css.is_empty() {
                    if let Ok(css_json) = serde_json::to_string(&cos.css) {
                        let inject_css = format!(
                            "(function(){{var id='seu-adblock-cosmetic';var s=document.getElementById(id);if(!s){{s=document.createElement('style');s.id=id;(document.head||document.documentElement).appendChild(s);}}s.textContent={css_json};}})();"
                        );
                        let code = CefString::from(inject_css.as_str());
                        let script_url = CefString::from("seu://internal/cosmetic");
                        frame.execute_java_script(Some(&code), Some(&script_url), 0);
                    }
                }
                if !cos.script.is_empty() {
                    let code = CefString::from(cos.script.as_str());
                    let script_url = CefString::from("seu://internal/scriptlets");
                    frame.execute_java_script(Some(&code), Some(&script_url), 0);
                }
            }
        }
    }
}

wrap_life_span_handler! {
    struct SeuLifeSpanHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser.cloned() else {
                return;
            };
            let mut broadcast_win: Option<WindowId> = None;
            let mut visibility_update: Option<bool> = None;

            if let Ok(mut guard) = self.state.lock() {
                match self.kind {
                    ViewKind::Shell(wid) => {
                        if let Some(w) = guard.windows.get_mut(&wid) {
                            w.shell = Some(browser.clone());
                        }
                        broadcast_win = Some(wid);
                    }
                    ViewKind::Overlay(wid) => {
                        if let Some(w) = guard.windows.get_mut(&wid) {
                            w.overlay = Some(browser.clone());
                        }
                        visibility_update = Some(false);
                    }
                    ViewKind::Tab { window_id, tab_id } => {
                        guard.tab_browsers.insert(tab_id, browser.clone());
                        let is_active = guard
                            .core
                            .window_snapshot(window_id)
                            .ok()
                            .and_then(|s| s.active_tab_id)
                            == Some(tab_id);
                        visibility_update = Some(is_active);
                        broadcast_win = Some(window_id);
                    }
                }
            }

            if let Some(visible) = visibility_update {
                if let Some(host) = browser.host() {
                    apply_child_hwnd_visibility(host.window_handle(), visible, None, false);
                }
            }

            if let Some(wid) = broadcast_win {
                broadcast_window_state(&self.state, wid);
            }
        }

        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let ViewKind::Tab { window_id, tab_id } = self.kind {
                let url = target_url.map(|u| u.to_string()).unwrap_or_default();
                if !url.is_empty() && !url.eq_ignore_ascii_case("about:blank") {
                    let open_bg = target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB;
                    let mut task = OpenTabUrlTask::new(
                        self.state.clone(),
                        window_id,
                        tab_id,
                        url,
                        open_bg,
                    );
                    post_task(ThreadId::UI, Some(&mut task));
                }
            }
            // Return 1 to always cancel CEF's default standalone popup window.
            1
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            // All CEF browsers in Seu Navegador are embedded child HWNDs (`set_as_child`).
            // Detach and destroy ONLY the child CEF HWND asynchronously on the next UI tick,
            // returning 1 so CEF never posts WM_CLOSE to our top-level parent window.
            if let Some(host) = browser.and_then(|b| b.host()) {
                let hwnd_raw = host.window_handle().0 as isize;
                if hwnd_raw != 0 {
                    let mut task = DestroyChildHwndTask::new(hwnd_raw);
                    post_task(ThreadId::UI, Some(&mut task));
                }
            }
            1
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let router = self.state.lock().ok().map(|g| g.router.clone());
            if let Some(router) = router {
                router.on_before_close(browser.cloned());
            }
        }
    }
}

wrap_request_handler! {
    struct SeuRequestHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl RequestHandler {
        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: ::std::os::raw::c_int,
            _is_download: ::std::os::raw::c_int,
            _request_initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut ::std::os::raw::c_int>,
        ) -> Option<ResourceRequestHandler> {
            if !matches!(self.kind, ViewKind::Tab { .. }) {
                return None;
            }
            let blocker = self.state.lock().ok().map(|g| g.blocker.clone())?;
            Some(SeuResourceRequestHandler::new(
                self.state.clone(),
                blocker,
                self.kind,
            ))
        }

        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            if target_disposition == WindowOpenDisposition::CURRENT_TAB {
                return 0;
            }
            if let ViewKind::Tab { window_id, tab_id } = self.kind {
                let url = target_url.map(|u| u.to_string()).unwrap_or_default();
                if !url.is_empty() {
                    let open_bg = target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB;
                    let mut task = OpenTabUrlTask::new(
                        self.state.clone(),
                        window_id,
                        tab_id,
                        url,
                        open_bg,
                    );
                    post_task(ThreadId::UI, Some(&mut task));
                    return 1;
                }
            }
            0
        }

        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let router = self.state.lock().ok().map(|g| g.router.clone());
            if let Some(router) = router {
                router.on_before_browse(browser.cloned(), frame.cloned());
            }
            0
        }

        fn on_render_process_terminated(
            &self,
            browser: Option<&mut Browser>,
            _status: TerminationStatus,
            _error_code: i32,
            _error_string: Option<&CefString>,
        ) {
            let router = self.state.lock().ok().map(|g| g.router.clone());
            if let Some(router) = router {
                router.on_render_process_terminated(browser.cloned());
            }
        }
    }
}

wrap_task! {
    struct ResizeWindowTask {
        state: Arc<Mutex<SharedState>>,
        window_id: WindowId,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            relayout_window(&self.state, self.window_id, self.width, self.height);
        }
    }
}

wrap_task! {
    struct CloseWindowTask {
        state: Arc<Mutex<SharedState>>,
        window_id: WindowId,
    }

    impl Task {
        fn execute(&self) {
            handle_native_window_closed(&self.state, self.window_id);
        }
    }
}

wrap_task! {
    struct ExecuteEffectsTask {
        state: Arc<Mutex<SharedState>>,
        effects: Arc<Mutex<Option<Vec<HostEffect>>>>,
    }

    impl Task {
        fn execute(&self) {
            let effects = self.effects.lock().ok().and_then(|mut g| g.take());
            if let Some(effects) = effects {
                execute_effects(&self.state, effects);
            }
        }
    }
}

wrap_task! {
    struct DestroyChildHwndTask {
        hwnd_raw: isize,
    }

    impl Task {
        fn execute(&self) {
            let hwnd = self.hwnd_raw as HWND;
            if !hwnd.is_null() {
                unsafe {
                    SetParent(hwnd, std::ptr::null_mut());
                    DestroyWindow(hwnd);
                }
            }
        }
    }
}

wrap_focus_handler! {
    struct SeuFocusHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl FocusHandler {
        fn on_got_focus(&self, _browser: Option<&mut Browser>) {
            // Clicking back into the Shell or active Tab dismisses any open Overlay menu.
            if matches!(self.kind, ViewKind::Overlay(_)) {
                return;
            }
            let wid = self.kind.window_id();
            let should_close = self
                .state
                .lock()
                .ok()
                .and_then(|g| g.windows.get(&wid).map(|w| (w.overlay_open, w.overlay_opened_at)))
                .map(|(open, at)| open && at.elapsed() >= Duration::from_millis(180))
                .unwrap_or(false);
            if should_close {
                let mut task = CloseOverlayTask::new(self.state.clone(), wid);
                post_task(ThreadId::UI, Some(&mut task));
            }
        }
    }
}

wrap_keyboard_handler! {
    struct SeuKeyboardHandler {
        state: Arc<Mutex<SharedState>>,
        kind: ViewKind,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut sys::MSG>,
            _is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let Some(ev) = event else {
                return 0;
            };
            if ev.type_ != KeyEventType::RAWKEYDOWN {
                return 0;
            }

            let wid = self.kind.window_id();

            // Escape closes open overlay menus from any view.
            if ev.windows_key_code == 0x1B {
                let overlay_open = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|g| g.windows.get(&wid).map(|w| w.overlay_open))
                    .unwrap_or(false);
                if overlay_open {
                    let mut task = CloseOverlayTask::new(self.state.clone(), wid);
                    post_task(ThreadId::UI, Some(&mut task));
                    return 1;
                }
            }

            // Allow `seu://settings/` to record custom key combinations without triggering
            // the currently bound action mid-recording.
            if let Some(frame) = browser.and_then(|b| b.main_frame()) {
                let url = CefString::from(&frame.url()).to_string();
                if url.starts_with("seu://settings/") {
                    return 0;
                }
            }

            let ctrl = (ev.modifiers & CEF_EVENTFLAG_CONTROL_DOWN) != 0;
            let shift = (ev.modifiers & CEF_EVENTFLAG_SHIFT_DOWN) != 0;
            let alt = (ev.modifiers & CEF_EVENTFLAG_ALT_DOWN) != 0;

            let matched = self.state.lock().ok().and_then(|g| {
                match_shortcut_vk(
                    &g.core.settings.shortcuts,
                    ev.windows_key_code,
                    ctrl,
                    shift,
                    alt,
                )
            });

            if let Some(action) = matched {
                let mut task = ExecuteShortcutTask::new(self.state.clone(), wid, action);
                post_task(ThreadId::UI, Some(&mut task));
                return 1;
            }

            0
        }
    }
}

wrap_task! {
    struct CloseOverlayTask {
        state: Arc<Mutex<SharedState>>,
        window_id: WindowId,
    }

    impl Task {
        fn execute(&self) {
            let engine = CefEngine {
                state: self.state.clone(),
            };
            engine.hide_overlay(self.window_id);
        }
    }
}

wrap_task! {
    struct ExecuteShortcutTask {
        state: Arc<Mutex<SharedState>>,
        window_id: WindowId,
        action: ShortcutAction,
    }

    impl Task {
        fn execute(&self) {
            if self.action == ShortcutAction::ToggleMenu {
                let shell = self
                    .state
                    .lock()
                    .ok()
                    .and_then(|g| g.windows.get(&self.window_id).and_then(|w| w.shell.clone()));
                if let Some(s) = shell {
                    push_js_event(&s, &serde_json::json!({ "type": "toggleMenu" }));
                }
                return;
            }

            let effects = {
                let Ok(mut guard) = self.state.lock() else {
                    return;
                };
                let active_tab = guard
                    .core
                    .window_snapshot(self.window_id)
                    .ok()
                    .and_then(|s| s.active_tab_id);

                match self.action {
                    ShortcutAction::NewTab => {
                        match guard.core.create_tab(self.window_id, None, true) {
                            Ok(tid) => vec![
                                HostEffect::CreateTabView {
                                    window_id: self.window_id,
                                    tab_id: tid,
                                    url: seu_core::NEWTAB_URL.to_string(),
                                },
                                HostEffect::ActivateTabView {
                                    window_id: self.window_id,
                                    tab_id: tid,
                                },
                                HostEffect::BroadcastState {
                                    window_id: self.window_id,
                                },
                            ],
                            Err(_) => Vec::new(),
                        }
                    }
                    ShortcutAction::CloseTab => match active_tab {
                        Some(tid) => match guard.core.close_tab(tid) {
                            Ok((wid, next_active)) => {
                                let mut out = vec![HostEffect::CloseTabView {
                                    window_id: wid,
                                    tab_id: tid,
                                }];
                                if let Some(next_id) = next_active {
                                    out.push(HostEffect::ActivateTabView {
                                        window_id: wid,
                                        tab_id: next_id,
                                    });
                                    out.push(HostEffect::BroadcastState { window_id: wid });
                                } else {
                                    out.push(HostEffect::CloseWindow { window_id: wid });
                                }
                                out
                            }
                            Err(_) => Vec::new(),
                        },
                        None => Vec::new(),
                    },
                    ShortcutAction::ReopenClosedTab => {
                        match guard.core.reopen_closed_tab(self.window_id) {
                            Ok(Some(tid)) => {
                                let url = guard
                                    .core
                                    .get_tab(tid)
                                    .map(|t| t.url.clone())
                                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                                vec![
                                    HostEffect::CreateTabView {
                                        window_id: self.window_id,
                                        tab_id: tid,
                                        url,
                                    },
                                    HostEffect::ActivateTabView {
                                        window_id: self.window_id,
                                        tab_id: tid,
                                    },
                                    HostEffect::BroadcastState {
                                        window_id: self.window_id,
                                    },
                                ]
                            }
                            _ => Vec::new(),
                        }
                    }
                    ShortcutAction::NextTab => {
                        wake_or_activate_tab_effects(&mut guard, self.window_id, |core, wid| {
                            core.cycle_tab(wid, true).ok().flatten()
                        })
                    }
                    ShortcutAction::PrevTab => {
                        wake_or_activate_tab_effects(&mut guard, self.window_id, |core, wid| {
                            core.cycle_tab(wid, false).ok().flatten()
                        })
                    }
                    ShortcutAction::Reload => active_tab
                        .map(|tab_id| vec![HostEffect::ReloadTabView { tab_id }])
                        .unwrap_or_default(),
                    ShortcutAction::HardReload => active_tab
                        .map(|tab_id| vec![HostEffect::HardReloadTabView { tab_id }])
                        .unwrap_or_default(),
                    ShortcutAction::Back => active_tab
                        .map(|tab_id| vec![HostEffect::GoBackTabView { tab_id }])
                        .unwrap_or_default(),
                    ShortcutAction::Forward => active_tab
                        .map(|tab_id| vec![HostEffect::GoForwardTabView { tab_id }])
                        .unwrap_or_default(),
                    ShortcutAction::FocusOmnibox => vec![HostEffect::FocusOmnibox {
                        window_id: self.window_id,
                    }],
                    ShortcutAction::NewWindow => {
                        let (wid, tid) = guard.core.create_window(false, None);
                        vec![HostEffect::CreateNativeWindow {
                            window_id: wid,
                            incognito: false,
                            initial_tab_id: tid,
                            initial_url: seu_core::NEWTAB_URL.to_string(),
                        }]
                    }
                    ShortcutAction::NewIncognitoWindow => {
                        let (wid, tid) = guard.core.create_window(true, None);
                        vec![HostEffect::CreateNativeWindow {
                            window_id: wid,
                            incognito: true,
                            initial_tab_id: tid,
                            initial_url: seu_core::NEWTAB_URL.to_string(),
                        }]
                    }
                    ShortcutAction::PinTab => match active_tab {
                        Some(tid) if guard.core.toggle_pin_tab(tid).is_ok() => {
                            vec![HostEffect::BroadcastState {
                                window_id: self.window_id,
                            }]
                        }
                        _ => Vec::new(),
                    },
                    ShortcutAction::DuplicateTab => match active_tab {
                        Some(tid) => match guard.core.duplicate_tab(tid) {
                            Ok(new_id) => {
                                let url = guard
                                    .core
                                    .get_tab(new_id)
                                    .map(|t| t.url.clone())
                                    .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
                                vec![
                                    HostEffect::CreateTabView {
                                        window_id: self.window_id,
                                        tab_id: new_id,
                                        url,
                                    },
                                    HostEffect::ActivateTabView {
                                        window_id: self.window_id,
                                        tab_id: new_id,
                                    },
                                    HostEffect::BroadcastState {
                                        window_id: self.window_id,
                                    },
                                ]
                            }
                            Err(_) => Vec::new(),
                        },
                        None => Vec::new(),
                    },
                    ShortcutAction::OpenSettings => match active_tab {
                        Some(tid) => match guard.core.navigate_tab(tid, "seu://settings/") {
                            Ok(url) => vec![
                                HostEffect::NavigateTabView { tab_id: tid, url },
                                HostEffect::BroadcastState {
                                    window_id: self.window_id,
                                },
                            ],
                            Err(_) => Vec::new(),
                        },
                        None => Vec::new(),
                    },
                    ShortcutAction::ZoomIn => active_tab
                        .map(|tab_id| {
                            vec![HostEffect::ZoomTabView {
                                tab_id,
                                delta_steps: 1,
                            }]
                        })
                        .unwrap_or_default(),
                    ShortcutAction::ZoomOut => active_tab
                        .map(|tab_id| {
                            vec![HostEffect::ZoomTabView {
                                tab_id,
                                delta_steps: -1,
                            }]
                        })
                        .unwrap_or_default(),
                    ShortcutAction::ZoomReset => active_tab
                        .map(|tab_id| {
                            vec![HostEffect::ZoomTabView {
                                tab_id,
                                delta_steps: 0,
                            }]
                        })
                        .unwrap_or_default(),
                    ShortcutAction::SwitchTab1
                    | ShortcutAction::SwitchTab2
                    | ShortcutAction::SwitchTab3
                    | ShortcutAction::SwitchTab4
                    | ShortcutAction::SwitchTab5
                    | ShortcutAction::SwitchTab6
                    | ShortcutAction::SwitchTab7
                    | ShortcutAction::SwitchTab8 => {
                        let idx = match self.action {
                            ShortcutAction::SwitchTab1 => 0,
                            ShortcutAction::SwitchTab2 => 1,
                            ShortcutAction::SwitchTab3 => 2,
                            ShortcutAction::SwitchTab4 => 3,
                            ShortcutAction::SwitchTab5 => 4,
                            ShortcutAction::SwitchTab6 => 5,
                            ShortcutAction::SwitchTab7 => 6,
                            _ => 7,
                        };
                        wake_or_activate_tab_effects(&mut guard, self.window_id, |core, wid| {
                            core.activate_tab_by_index(wid, idx).ok().flatten()
                        })
                    }
                    ShortcutAction::SwitchTabLast => {
                        wake_or_activate_tab_effects(&mut guard, self.window_id, |core, wid| {
                            core.activate_last_tab(wid).ok().flatten()
                        })
                    }
                    ShortcutAction::ToggleMenu => Vec::new(),
                }
            };

            if !effects.is_empty() {
                execute_effects(&self.state, effects);
            }
        }
    }
}

wrap_task! {
    struct OpenTabUrlTask {
        state: Arc<Mutex<SharedState>>,
        window_id: WindowId,
        tab_id: TabId,
        url: String,
        new_background_tab: bool,
    }

    impl Task {
        fn execute(&self) {
            let effects = {
                let Ok(mut guard) = self.state.lock() else {
                    return;
                };
                if self.new_background_tab {
                    match guard
                        .core
                        .create_tab(self.window_id, Some(&self.url), false)
                    {
                        Ok(new_tab_id) => vec![
                            HostEffect::CreateTabView {
                                window_id: self.window_id,
                                tab_id: new_tab_id,
                                url: self.url.clone(),
                            },
                            HostEffect::BroadcastState {
                                window_id: self.window_id,
                            },
                        ],
                        Err(_) => Vec::new(),
                    }
                } else {
                    match guard.core.navigate_tab(self.tab_id, &self.url) {
                        Ok(resolved) => vec![
                            HostEffect::NavigateTabView {
                                tab_id: self.tab_id,
                                url: resolved,
                            },
                            HostEffect::BroadcastState {
                                window_id: self.window_id,
                            },
                        ],
                        Err(_) => Vec::new(),
                    }
                }
            };
            if !effects.is_empty() {
                execute_effects(&self.state, effects);
            }
        }
    }
}

fn wake_or_activate_tab_effects<F>(
    guard: &mut SharedState,
    window_id: WindowId,
    select_fn: F,
) -> Vec<HostEffect>
where
    F: FnOnce(&mut BrowserCore, WindowId) -> Option<TabId>,
{
    let Some(tid) = select_fn(&mut guard.core, window_id) else {
        return Vec::new();
    };
    let needs_spawn = !guard.tab_browsers.contains_key(&tid);
    let mut out = Vec::new();
    if needs_spawn {
        let url = guard
            .core
            .get_tab(tid)
            .map(|t| t.url.clone())
            .unwrap_or_else(|| seu_core::NEWTAB_URL.to_string());
        let (sx, sy) = guard
            .core
            .tab_memory_meta(tid)
            .map(|m| (m.scroll_x, m.scroll_y))
            .unwrap_or((0, 0));
        out.push(HostEffect::WakeTabView {
            window_id,
            tab_id: tid,
            url,
            scroll_x: sx,
            scroll_y: sy,
        });
    }
    out.push(HostEffect::ActivateTabView {
        window_id,
        tab_id: tid,
    });
    out.push(HostEffect::BroadcastState { window_id });
    if needs_spawn {
        out.push(HostEffect::BroadcastMemory);
    }
    out
}

fn start_memory_monitor_thread(state: Arc<Mutex<SharedState>>) {
    let running = match state.lock() {
        Ok(g) => {
            if g.monitor_running.swap(true, Ordering::SeqCst) {
                return;
            }
            g.monitor_running.clone()
        }
        Err(_) => return,
    };

    let _ = std::thread::Builder::new()
        .name("seu-mem-monitor".into())
        .spawn(move || {
            while running.load(Ordering::SeqCst) {
                let mut task = MemoryTickTask::new(state.clone());
                post_task(ThreadId::UI, Some(&mut task));
                std::thread::sleep(Duration::from_secs(3));
            }
        });
}

fn build_tab_probe_script(token: &str) -> String {
    format!(
        r#"(function(){{
  if (window.__seuProbeInstalled) return;
  window.__seuProbeInstalled = true;
  var prefix = "__SEU_PROBE__{token}:";
  var dirty = false, hasBeforeUnload = false, lastAudible = false;
  var timer = null;
  function emit() {{
    var playing = false;
    try {{
      var media = document.querySelectorAll('video,audio');
      for (var i = 0; i < media.length; i++) {{
        var m = media[i];
        if (!m.paused && !m.ended && !m.muted && m.volume > 0 && m.readyState > 1) {{
          playing = true; break;
        }}
      }}
    }} catch(_) {{}}
    lastAudible = playing;
    try {{
      console.debug(prefix + JSON.stringify({{
        dirty: dirty,
        beforeunload: hasBeforeUnload || (typeof window.onbeforeunload === 'function'),
        sx: Math.round(window.scrollX || 0),
        sy: Math.round(window.scrollY || 0),
        audible: playing
      }}));
    }} catch(_) {{}}
  }}
  function schedule() {{
    if (timer) return;
    timer = setTimeout(function(){{ timer = null; emit(); }}, 350);
  }}
  document.addEventListener('input', function(e) {{
    var t = e.target;
    if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)) {{
      dirty = true;
      schedule();
    }}
  }}, true);
  document.addEventListener('submit', function() {{ dirty = false; schedule(); }}, true);
  window.addEventListener('scroll', schedule, {{ passive: true }});
  var origAdd = window.addEventListener;
  window.addEventListener = function(type, listener, opts) {{
    if (type === 'beforeunload') {{ hasBeforeUnload = true; schedule(); }}
    return origAdd.apply(this, arguments);
  }};
  setInterval(function() {{
    var playing = false;
    try {{
      var media = document.querySelectorAll('video,audio');
      for (var i = 0; i < media.length; i++) {{
        var m = media[i];
        if (!m.paused && !m.ended && !m.muted && m.volume > 0 && m.readyState > 1) {{
          playing = true; break;
        }}
      }}
    }} catch(_) {{}}
    if (playing !== lastAudible) emit();
  }}, 2000);
}})();"#
    )
}

wrap_task! {
    struct MemoryTickTask {
        state: Arc<Mutex<SharedState>>,
    }

    impl Task {
        fn execute(&self) {
            let mem = measure_browser_tree_memory();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let effects = {
                let Ok(mut guard) = self.state.lock() else {
                    return;
                };
                guard.core.update_os_memory_metrics(
                    mem.total_private_mb(),
                    mem.total_working_set_mb(),
                    mem.process_count,
                );

                let due = guard.core.select_due_sleep_tabs(now_ms, false);
                let mut out = Vec::new();
                let mut touched_windows = Vec::new();
                for tid in due {
                    if let Ok(wid) = guard.core.mark_tab_sleeping(tid, false) {
                        out.push(HostEffect::SleepTabView {
                            window_id: wid,
                            tab_id: tid,
                        });
                        if !touched_windows.contains(&wid) {
                            touched_windows.push(wid);
                        }
                    }
                }
                for wid in touched_windows {
                    out.push(HostEffect::BroadcastState { window_id: wid });
                }
                out
            };

            if !effects.is_empty() {
                execute_effects(&self.state, effects);
            }
            broadcast_memory_all(&self.state);
        }
    }
}

wrap_task! {
    struct MeasureSleepDeltaTask {
        state: Arc<Mutex<SharedState>>,
    }

    impl Task {
        fn execute(&self) {
            let after = measure_browser_tree_memory();
            let after_mb = after.total_private_mb();

            if let Ok(mut guard) = self.state.lock() {
                if let Some((before_mb, before_procs, tab_ids)) = guard.pending_sleep_batch.take() {
                    let delta_mb = before_mb.saturating_sub(after_mb);
                    let shared = after.process_count >= before_procs && delta_mb < 5;
                    guard
                        .core
                        .record_batch_sleep_ram_delta(&tab_ids, delta_mb, shared);
                }
                guard.core.update_os_memory_metrics(
                    after_mb,
                    after.total_working_set_mb(),
                    after.process_count,
                );
            }
            broadcast_memory_all(&self.state);
        }
    }
}

wrap_resource_request_handler! {
    struct SeuResourceRequestHandler {
        state: Arc<Mutex<SharedState>>,
        blocker: Arc<RwLock<BlockerService>>,
        kind: ViewKind,
    }

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            let Some(req) = request else {
                return ReturnValue::CONTINUE;
            };
            let url = CefString::from(&req.url()).to_string();
            if url.starts_with("seu://") || url.starts_with("data:") || url.starts_with("blob:") {
                return ReturnValue::CONTINUE;
            }
            let rtype = req.resource_type();
            if rtype == ResourceType::MAIN_FRAME {
                return ReturnValue::CONTINUE;
            }

            let source_url = browser
                .and_then(|b| b.main_frame())
                .map(|f| CefString::from(&f.url()).to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| CefString::from(&req.first_party_for_cookies()).to_string());

            let method = CefString::from(&req.method())
                .to_string()
                .to_ascii_lowercase();
            let req_type_str = cef_resource_type_str(rtype);

            let should_block = self
                .blocker
                .read()
                .map(|b| b.should_block_request(&url, &source_url, req_type_str, &method))
                .unwrap_or(false);

            if should_block {
                if let ViewKind::Tab { tab_id, .. } = self.kind {
                    let mut task = RecordBlockedTask::new(self.state.clone(), tab_id, url);
                    post_task(ThreadId::UI, Some(&mut task));
                }
                return ReturnValue::CANCEL;
            }

            ReturnValue::CONTINUE
        }
    }
}

fn cef_resource_type_str(rtype: ResourceType) -> &'static str {
    if rtype == ResourceType::SCRIPT {
        "script"
    } else if rtype == ResourceType::IMAGE {
        "image"
    } else if rtype == ResourceType::STYLESHEET {
        "stylesheet"
    } else if rtype == ResourceType::SUB_FRAME {
        "sub_frame"
    } else if rtype == ResourceType::FONT_RESOURCE {
        "font"
    } else if rtype == ResourceType::MEDIA {
        "media"
    } else if rtype == ResourceType::XHR {
        "xmlhttprequest"
    } else if rtype == ResourceType::PING {
        "ping"
    } else if rtype == ResourceType::OBJECT {
        "object"
    } else {
        "other"
    }
}

wrap_task! {
    struct RecordBlockedTask {
        state: Arc<Mutex<SharedState>>,
        tab_id: TabId,
        blocked_url: String,
    }

    impl Task {
        fn execute(&self) {
            let wid = {
                let Ok(mut guard) = self.state.lock() else {
                    return;
                };
                if let Ok(mut b) = guard.blocker.write() {
                    b.record_blocked(&self.blocked_url);
                }
                guard.core.increment_tab_blocked(self.tab_id)
            };
            if let Some(window_id) = wid {
                broadcast_window_state(&self.state, window_id);
            }
            broadcast_blocker_all(&self.state);
        }
    }
}

wrap_task! {
    struct BroadcastBlockerTask {
        state: Arc<Mutex<SharedState>>,
    }

    impl Task {
        fn execute(&self) {
            broadcast_blocker_all(&self.state);
        }
    }
}
