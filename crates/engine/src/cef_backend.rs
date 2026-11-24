//! CEF Alloy backend implementation for Windows x64.

use crate::assets::{resolve_seu_url, STRICT_SEU_CSP};
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
use seu_bridge::{handle_raw_request, is_privileged_origin, HostEffect};
use seu_core::{BrowserCore, CoreSettings, TabId, WindowId, WindowSnapshot};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tracing::{error, info, warn};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, SetParent, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_HIDE, SW_SHOW,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    Shell(WindowId),
    Overlay(WindowId),
    Tab { window_id: WindowId, tab_id: TabId },
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
    overlay_bounds: RectPx,
    incognito: bool,
}

struct SharedState {
    core: BrowserCore,
    host: Arc<dyn NativeWindowHost>,
    router: Arc<BrowserSideRouter>,
    windows: HashMap<WindowId, WindowViews>,
    tab_browsers: HashMap<TabId, Browser>,
    incognito_contexts: HashMap<WindowId, RequestContext>,
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
    ) -> Result<(), EngineError> {
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        let args = cef::args::Args::new();

        let router = BrowserSideRouter::new(MessageRouterConfig::default());
        let mut core = BrowserCore::new();
        core.settings = initial_settings;

        let shared = Arc::new(Mutex::new(SharedState {
            core,
            host,
            router: router.clone(),
            windows: HashMap::new(),
            tab_browsers: HashMap::new(),
            incognito_contexts: HashMap::new(),
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
            }
        }
    }

    fn close_tab_view(&self, tab_id: TabId) {
        let browser = self
            .state
            .lock()
            .ok()
            .and_then(|mut g| g.tab_browsers.remove(&tab_id));
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
            win.overlay_bounds = bounds;
            win.overlay.clone()
        });
        if let Some(b) = overlay {
            if let Some(host) = b.host() {
                apply_child_hwnd_visibility(host.window_handle(), true, Some(bounds), true);
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
    let (snapshot, shell_browser) = {
        let Ok(guard) = state.lock() else {
            return;
        };
        let Ok(snap) = guard.core.window_snapshot(window_id) else {
            return;
        };
        let shell = guard.windows.get(&window_id).and_then(|w| w.shell.clone());
        (snap, shell)
    };

    if let Some(shell) = shell_browser {
        push_js_event(
            &shell,
            &serde_json::json!({ "type": "state", "snapshot": snapshot }),
        );
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
            HostEffect::NavigateTabView { tab_id, url } => engine.navigate_tab(tab_id, &url),
            HostEffect::ReloadTabView { tab_id } => engine.reload_tab(tab_id),
            HostEffect::GoBackTabView { tab_id } => engine.go_back(tab_id),
            HostEffect::GoForwardTabView { tab_id } => engine.go_forward(tab_id),
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
        }
    }
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
            handle_raw_request(&origin_url, request, &mut guard.core)
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
                        Mutex::new(Some(output.effects)),
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
            Some(SeuRequestHandler::new(self.state.clone()))
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
    }

    impl RequestHandler {
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
        effects: Mutex<Option<Vec<HostEffect>>>,
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
