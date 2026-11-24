//! Engine abstraction (`BrowserBackend`) isolating CEF from the rest of the browser.
//!
//! Keeps CEF types strictly inside `cef_backend` so future backends (such as
//! GeckoView on Android) can plug in without leaking engine internals into `core`.

pub mod assets;

#[cfg(target_os = "windows")]
pub mod cef_backend;

use seu_core::{TabId, WindowId};
use std::path::PathBuf;
use thiserror::Error;

/// Opaque native parent window handle (HWND on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWindowHandle(pub isize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectPx {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub root_cache_path: PathBuf,
    pub user_agent: String,
    pub enable_sandbox: bool,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("engine initialization failed: {0}")]
    InitFailed(String),
    #[error("view creation failed: {0}")]
    ViewFailed(String),
}

/// Platform-agnostic interface implemented by CEF (Windows v1) and future engines.
pub trait BrowserBackend {
    fn create_shell_view(
        &self,
        window_id: WindowId,
        parent: NativeWindowHandle,
        bounds: RectPx,
    ) -> Result<(), EngineError>;

    fn create_overlay_view(
        &self,
        window_id: WindowId,
        parent: NativeWindowHandle,
        bounds: RectPx,
    ) -> Result<(), EngineError>;

    fn create_tab_view(
        &self,
        window_id: WindowId,
        tab_id: TabId,
        incognito: bool,
        parent: NativeWindowHandle,
        bounds: RectPx,
        url: &str,
    ) -> Result<(), EngineError>;

    fn navigate_tab(&self, tab_id: TabId, url: &str);
    fn reload_tab(&self, tab_id: TabId);
    fn go_back(&self, tab_id: TabId);
    fn go_forward(&self, tab_id: TabId);
    fn close_tab_view(&self, tab_id: TabId);
    fn set_tab_visible(&self, tab_id: TabId, visible: bool, bounds: Option<RectPx>);
    fn resize_shell_view(&self, window_id: WindowId, bounds: RectPx);
    fn show_overlay(&self, window_id: WindowId, bounds: RectPx, panel: &str);
    fn hide_overlay(&self, window_id: WindowId);
    fn close_window_views(&self, window_id: WindowId);
}
