#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};

use cef::{
    api_hash, args::Args, browser_view_create, execute_process, initialize, quit_message_loop,
    run_message_loop, shutdown, sys, window_create_top_level, wrap_app,
    wrap_browser_process_handler, wrap_browser_view_delegate, wrap_client, wrap_window_delegate,
    App, BrowserProcessHandler, BrowserSettings, BrowserView, BrowserViewDelegate, CefString,
    Client, ImplApp, ImplBrowserProcessHandler, ImplBrowserViewDelegate, ImplClient, ImplPanel,
    ImplPanelDelegate, ImplViewDelegate, ImplWindow, ImplWindowDelegate, PanelDelegate,
    RuntimeStyle, Settings, Size, View, ViewDelegate, Window, WindowDelegate,
};

const START_URL: &str = "https://www.youtube.com";

wrap_app! {
    struct SpikeApp {
        window: Arc<Mutex<Option<Window>>>,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(SpikeBrowserProcessHandler::new(self.window.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct SpikeBrowserProcessHandler {
        window: Arc<Mutex<Option<Window>>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            let mut client = SpikeClient::new();
            let url = CefString::from(START_URL);
            let settings = BrowserSettings::default();
            let mut view_delegate = SpikeBrowserViewDelegate::new();

            let browser_view = browser_view_create(
                Some(&mut client),
                Some(&url),
                Some(&settings),
                None,
                None,
                Some(&mut view_delegate),
            );

            let Some(browser_view) = browser_view else {
                eprintln!("Falha ao criar BrowserView do CEF.");
                quit_message_loop();
                return;
            };

            let mut window_delegate = SpikeWindowDelegate::new(browser_view);
            let created_window = window_create_top_level(Some(&mut window_delegate));
            if let Ok(mut guard) = self.window.lock() {
                *guard = created_window;
            }
        }
    }
}

wrap_client! {
    struct SpikeClient;

    impl Client {}
}

wrap_browser_view_delegate! {
    struct SpikeBrowserViewDelegate;

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_window_delegate! {
    struct SpikeWindowDelegate {
        browser_view: BrowserView,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size {
                width: 1280,
                height: 800,
            }
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else {
                return;
            };
            let title = CefString::from("Seu Navegador — Phase 0 Spike (CEF Alloy)");
            window.set_title(Some(&title));

            let mut view = View::from(&self.browser_view);
            window.add_child_view(Some(&mut view));
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            quit_message_loop();
        }

        fn can_resize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> ::std::os::raw::c_int {
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);

    let args = Args::new();
    let window = Arc::new(Mutex::new(None));
    let mut app = SpikeApp::new(window.clone());

    let exit_code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if exit_code >= 0 {
        std::process::exit(exit_code);
    }

    let settings = Settings {
        no_sandbox: 1,
        ..Default::default()
    };

    let initialized = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if initialized == 0 {
        eprintln!("Falha ao inicializar o CEF.");
        std::process::exit(1);
    }

    run_message_loop();

    if let Ok(mut guard) = window.lock() {
        *guard = None;
    }

    shutdown();
}
