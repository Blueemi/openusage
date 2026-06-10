use tauri::image::Image;
use tauri::menu::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::path::BaseDirectory;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_nspanel::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_store::StoreExt;

use crate::log_path;
use crate::panel::{get_or_init_panel, position_panel_at_tray_icon, show_panel, toggle_panel};

#[cfg(target_os = "macos")]
use objc2::DeclaredClass as _;
#[cfg(target_os = "macos")]
use objc2::Message as _;

const LOG_LEVEL_STORE_KEY: &str = "logLevel";

#[cfg(target_os = "macos")]
const STATUS_ITEM_HORIZONTAL_HIT_PADDING: f64 = 3.0;
#[cfg(target_os = "macos")]
const STATUS_ITEM_VERTICAL_HIT_PADDING: f64 = 12.0;

fn should_open_tray_menu(button: MouseButton, button_state: MouseButtonState) -> bool {
    #[cfg(target_os = "macos")]
    {
        button == MouseButton::Right && button_state == MouseButtonState::Down
    }

    #[cfg(not(target_os = "macos"))]
    {
        button == MouseButton::Right && button_state == MouseButtonState::Up
    }
}

fn get_stored_log_level(app_handle: &AppHandle) -> log::LevelFilter {
    let store = match app_handle.store("settings.json") {
        Ok(s) => s,
        Err(_) => return log::LevelFilter::Error,
    };
    let value = store.get(LOG_LEVEL_STORE_KEY);
    let level_str = value.and_then(|v| v.as_str().map(|s| s.to_string()));
    match level_str.as_deref() {
        Some("error") => log::LevelFilter::Error,
        Some("warn") => log::LevelFilter::Warn,
        Some("info") => log::LevelFilter::Info,
        Some("debug") => log::LevelFilter::Debug,
        Some("trace") => log::LevelFilter::Trace,
        _ => log::LevelFilter::Error, // Default: least verbose
    }
}

fn set_stored_log_level(app_handle: &AppHandle, level: log::LevelFilter) {
    let level_str = match level {
        log::LevelFilter::Error => "error",
        log::LevelFilter::Warn => "warn",
        log::LevelFilter::Info => "info",
        log::LevelFilter::Debug => "debug",
        log::LevelFilter::Trace => "trace",
        log::LevelFilter::Off => "off",
    };
    log::info!("Log level changing to {:?}", level);
    if let Ok(store) = app_handle.store("settings.json") {
        store.set(LOG_LEVEL_STORE_KEY, serde_json::json!(level_str));
        let _ = store.save();
    }
    log::set_max_level(level);
}

pub fn create(app_handle: &AppHandle) -> tauri::Result<()> {
    let tray_icon_path = app_handle
        .path()
        .resolve("icons/tray-icon.png", BaseDirectory::Resource)?;
    let icon = Image::from_path(tray_icon_path)?;

    // Load persisted log level
    let current_level = get_stored_log_level(app_handle);
    log::set_max_level(current_level);

    let show_stats = MenuItem::with_id(app_handle, "show_stats", "Show Stats", true, None::<&str>)?;
    let go_to_settings = MenuItem::with_id(
        app_handle,
        "go_to_settings",
        "Go to Settings",
        true,
        None::<&str>,
    )?;

    // Log level submenu - clone items for use in event handler
    let log_error = CheckMenuItem::with_id(
        app_handle,
        "log_error",
        "Error",
        true,
        current_level == log::LevelFilter::Error,
        None::<&str>,
    )?;
    let log_warn = CheckMenuItem::with_id(
        app_handle,
        "log_warn",
        "Warn",
        true,
        current_level == log::LevelFilter::Warn,
        None::<&str>,
    )?;
    let log_info = CheckMenuItem::with_id(
        app_handle,
        "log_info",
        "Info",
        true,
        current_level == log::LevelFilter::Info,
        None::<&str>,
    )?;
    let log_debug = CheckMenuItem::with_id(
        app_handle,
        "log_debug",
        "Debug",
        true,
        current_level == log::LevelFilter::Debug,
        None::<&str>,
    )?;
    let log_trace = CheckMenuItem::with_id(
        app_handle,
        "log_trace",
        "Trace",
        true,
        current_level == log::LevelFilter::Trace,
        None::<&str>,
    )?;
    let log_level_separator = PredefinedMenuItem::separator(app_handle)?;
    let copy_log_path = MenuItem::with_id(
        app_handle,
        "copy_log_path",
        "Copy Log Path",
        true,
        None::<&str>,
    )?;
    let log_level_submenu = Submenu::with_items(
        app_handle,
        "Debug Level",
        true,
        &[
            &log_error,
            &log_warn,
            &log_info,
            &log_debug,
            &log_trace,
            &log_level_separator,
            &copy_log_path,
        ],
    )?;

    // Clone for capture in event handler
    let log_items = [
        (log_error.clone(), log::LevelFilter::Error),
        (log_warn.clone(), log::LevelFilter::Warn),
        (log_info.clone(), log::LevelFilter::Info),
        (log_debug.clone(), log::LevelFilter::Debug),
        (log_trace.clone(), log::LevelFilter::Trace),
    ];

    let separator = PredefinedMenuItem::separator(app_handle)?;
    let about = MenuItem::with_id(app_handle, "about", "About OpenUsage", true, None::<&str>)?;
    let quit = MenuItem::with_id(app_handle, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app_handle,
        &[
            &show_stats,
            &go_to_settings,
            &log_level_submenu,
            &separator,
            &about,
            &quit,
        ],
    )?;

    let event_menu = menu.clone();

    let tray = TrayIconBuilder::with_id("tray")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("OpenUsage")
        // Intentionally do NOT attach the Tauri menu to the tray icon. On macOS
        // 26+ a menu set on the NSStatusItem pops up on left-click too, ignoring
        // `show_menu_on_left_click(false)`. macOS gets a native contextual menu
        // on the status button after build; other platforms still pop up this
        // detached Tauri menu manually on right-click.
        .on_menu_event(move |app_handle, event| {
            log::debug!("tray menu: {}", event.id.as_ref());
            match event.id.as_ref() {
                "show_stats" => {
                    show_panel(app_handle);
                    let _ = app_handle.emit("tray:navigate", "home");
                }
                "go_to_settings" => {
                    show_panel(app_handle);
                    let _ = app_handle.emit("tray:navigate", "settings");
                }
                "about" => {
                    show_panel(app_handle);
                    let _ = app_handle.emit("tray:show-about", ());
                }
                "quit" => {
                    log::info!("quit requested via tray");
                    app_handle.exit(0);
                }
                "log_error" | "log_warn" | "log_info" | "log_debug" | "log_trace" => {
                    let selected_level = match event.id.as_ref() {
                        "log_error" => log::LevelFilter::Error,
                        "log_warn" => log::LevelFilter::Warn,
                        "log_info" => log::LevelFilter::Info,
                        "log_debug" => log::LevelFilter::Debug,
                        "log_trace" => log::LevelFilter::Trace,
                        _ => unreachable!(),
                    };
                    set_stored_log_level(app_handle, selected_level);
                    // Update all checkmarks - only the selected level should be checked
                    for (item, level) in &log_items {
                        let _ = item.set_checked(*level == selected_level);
                    }
                }
                "copy_log_path" => match log_path::for_app(app_handle) {
                    Ok(path) => {
                        if let Err(error) = app_handle
                            .clipboard()
                            .write_text(path.to_string_lossy().to_string())
                        {
                            log::error!("failed to copy log path to clipboard: {}", error);
                        } else {
                            log::info!("copied log path to clipboard");
                        }
                    }
                    Err(error) => {
                        log::error!("failed to resolve log path: {}", error);
                    }
                },
                _ => {}
            }
        })
        .on_tray_icon_event(move |tray, event| {
            let app_handle = tray.app_handle();

            let TrayIconEvent::Click {
                button,
                button_state,
                rect,
                ..
            } = event
            else {
                return;
            };

            match button {
                // Left click toggles the panel.
                MouseButton::Left => {
                    if button_state != MouseButtonState::Up {
                        return;
                    }
                    let Some(panel) = get_or_init_panel!(app_handle) else {
                        return;
                    };

                    if panel.is_visible() {
                        log::debug!("tray click: hiding panel");
                        panel.hide();
                        return;
                    }
                    log::debug!("tray click: showing panel");

                    // macOS quirk: must show window before positioning to another monitor
                    panel.show_and_make_key();
                    position_panel_at_tray_icon(app_handle, rect.position, rect.size);
                }
                // Non-macOS right-clicks still pop up the detached Tauri menu.
                // macOS handles status-item secondary clicks in the native monitor.
                MouseButton::Right => {
                    if !should_open_tray_menu(button, button_state) {
                        return;
                    }
                    log::debug!("tray right click: showing menu");
                    let Some(window) = app_handle.get_webview_window("main") else {
                        log::warn!("tray menu: main window not found");
                        return;
                    };
                    if let Err(error) = event_menu.popup(window.as_ref().window()) {
                        log::error!("failed to show tray menu: {}", error);
                    }
                }
                _ => {}
            }
        })
        .build(app_handle)?;

    #[cfg(target_os = "macos")]
    install_native_tray_context_menu(app_handle, &tray);

    Ok(())
}

#[cfg(target_os = "macos")]
fn install_native_tray_context_menu(app_handle: &AppHandle, tray: &tauri::tray::TrayIcon) {
    let app_handle = app_handle.clone();
    if let Err(error) = tray.with_inner_tray_icon(move |inner| {
        use muda::ContextMenu as _;
        use objc2::{ClassType, Message};
        use objc2_app_kit::{NSEvent, NSEventMask, NSMenu, NSView};
        use objc2_foundation::MainThreadMarker;
        use std::cell::Cell;
        use std::ptr;
        use std::ptr::NonNull;

        let Some(mtm) = MainThreadMarker::new() else {
            log::warn!("tray context menu: not on main thread");
            return;
        };

        let Some(status_item) = inner.ns_status_item() else {
            log::warn!("tray context menu: status item unavailable");
            return;
        };
        let Some(button) = status_item.button(mtm) else {
            log::warn!("tray context menu: status button unavailable");
            return;
        };

        let current_level = get_stored_log_level(&app_handle);
        let menu = match build_native_tray_context_menu(current_level) {
            Ok(menu) => menu,
            Err(error) => {
                log::error!("failed to build native tray context menu: {error}");
                return;
            }
        };
        let ns_menu = unsafe { &*(menu.ns_menu().cast::<NSMenu>()) };
        status_item.setMenu(Some(ns_menu));
        let local_ns_menu = ns_menu.retain();
        let local_status_item = status_item.retain();
        let button_view: &NSView = button.as_super().as_super().as_super();
        remove_status_item_overlay_subviews(button_view);
        install_native_status_button(&button, &status_item, app_handle.clone(), ns_menu);
        set_context_menu_on_view_tree(button_view, ns_menu);
        update_native_tray_rect_from_view(button_view);

        let Some(window) = button.window() else {
            log::warn!("tray context menu: status item window unavailable");
            std::mem::forget(menu);
            return;
        };

        let status_window_number = window.windowNumber();
        let global_ns_menu = ns_menu.retain();
        let global_status_item = status_item.retain();
        let global_status_view = button_view.retain();
        install_tray_input_overlay(&app_handle, ns_menu, &status_item, button_view);
        install_status_item_event_tap(ns_menu, &status_item, button_view);
        install_secondary_click_poll_timer(ns_menu, &status_item, button_view);

        let last_event_number = Cell::new(-1);
        let block = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { event_ptr.as_ref() };
            if event.windowNumber() != status_window_number
                || !should_open_tray_menu_from_native_event(event)
            {
                return event_ptr.as_ptr();
            }

            let event_number = event.eventNumber();
            if event_number == last_event_number.get() {
                return ptr::null_mut();
            }
            last_event_number.set(event_number);

            show_native_tray_menu(&local_status_item, &local_ns_menu);

            ptr::null_mut()
        });
        let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> = &block;
        let mask = NSEventMask::RightMouseDown
            | NSEventMask::RightMouseUp
            | NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseUp
            | NSEventMask::OtherMouseDown
            | NSEventMask::OtherMouseUp
            | NSEventMask::SystemDefined
            | NSEventMask::Gesture
            | NSEventMask::BeginGesture
            | NSEventMask::EndGesture
            | NSEventMask::Pressure
            | NSEventMask::DirectTouch;
        let token =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, block_ref) };
        if let Some(token) = token {
            std::mem::forget(token);
        } else {
            log::warn!("tray context menu: AppKit did not install event monitor");
        }

        let last_global_event_number = Cell::new(-1);
        let global_block = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| {
            let event = unsafe { event_ptr.as_ref() };
            if !should_open_tray_menu_from_native_event(event)
                || !is_mouse_inside_status_view(&global_status_view)
            {
                return;
            }

            let event_number = event.eventNumber();
            if event_number != 0 && event_number == last_global_event_number.get() {
                return;
            }
            last_global_event_number.set(event_number);

            show_native_tray_menu(&global_status_item, &global_ns_menu);
        });
        let global_block_ref: &block2::DynBlock<dyn Fn(NonNull<NSEvent>)> = &global_block;
        let global_token =
            NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, global_block_ref);
        if let Some(global_token) = global_token {
            std::mem::forget(global_token);
        } else {
            log::warn!("tray context menu: AppKit did not install global event monitor");
        }

        // NSMenu keeps a weak delegate; keep all native monitor state alive.
        std::mem::forget(menu);
        std::mem::forget(block);
        std::mem::forget(global_block);
    }) {
        log::warn!("tray context menu: failed to install native menu: {error}");
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct NativeStatusButtonState {
    app_handle: AppHandle,
    menu_ptr: usize,
    status_item_ptr: usize,
    suppress_next_mouse_up: bool,
}

#[cfg(target_os = "macos")]
type NativeStatusButtonStateMap =
    std::sync::Mutex<std::collections::HashMap<usize, NativeStatusButtonState>>;

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSStatusBarButton))]
    #[name = "OpenUsageNativeTrayStatusButton"]
    struct NativeTrayStatusButton;

    impl NativeTrayStatusButton {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&objc2_app_kit::NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.update_tray_rect();
            if should_open_tray_menu_from_native_event(event)
                || should_open_tray_menu_from_mouse_button_number(event.buttonNumber())
            {
                log::debug!(
                    "tray context menu: status button mouse down opening menu type={:?} button={}",
                    event.r#type(),
                    event.buttonNumber()
                );
                self.set_suppress_next_mouse_up(true);
                self.open_context_menu(event);
                return;
            }

            self.set_status_item_menu_enabled(false);
            self.set_button_highlighted(true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            log::debug!("tray context menu: status button left mouse up");
            self.update_tray_rect();
            self.set_button_highlighted(false);
            if self.take_suppress_next_mouse_up() {
                self.set_status_item_menu_enabled(true);
                return;
            }
            if let Some(app_handle) = self.app_handle() {
                toggle_panel(&app_handle);
            }
            self.set_status_item_menu_enabled(true);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            log::debug!("tray context menu: status button right mouse down");
            self.set_suppress_next_mouse_up(true);
            self.open_context_menu(event);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            self.set_suppress_next_mouse_up(false);
            self.set_button_highlighted(false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            log::debug!(
                "tray context menu: status button other mouse down button={}",
                event.buttonNumber()
            );
            self.set_suppress_next_mouse_up(true);
            self.open_context_menu(event);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            self.set_suppress_next_mouse_up(false);
            self.set_button_highlighted(false);
        }

        #[unsafe(method(openContextMenuFromClickGesture:))]
        fn open_context_menu_from_click_gesture(
            &self,
            recognizer: &objc2_app_kit::NSClickGestureRecognizer,
        ) {
            log::debug!(
                "tray context menu: status button click gesture button_mask={} touches={}",
                recognizer.buttonMask(),
                recognizer.numberOfTouchesRequired()
            );
            self.set_suppress_next_mouse_up(true);
            self.open_context_menu_without_event();
        }

        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(
            &self,
            event: &objc2_app_kit::NSEvent,
        ) -> Option<&'static objc2_app_kit::NSMenu> {
            if should_open_tray_menu_from_native_event(event) {
                log::debug!(
                    "tray context menu: status button returning menu for event type={:?}",
                    event.r#type()
                );
                self.update_tray_rect();
                self.set_suppress_next_mouse_up(true);
                self.set_button_highlighted(false);
                self.context_menu()
            } else {
                None
            }
        }

    }
);

#[cfg(target_os = "macos")]
impl NativeTrayStatusButton {
    fn as_status_button(&self) -> &objc2_app_kit::NSStatusBarButton {
        use objc2::ClassType;

        self.as_super()
    }

    fn as_button(&self) -> &objc2_app_kit::NSButton {
        use objc2::ClassType;

        self.as_status_button().as_super()
    }

    fn as_view(&self) -> &objc2_app_kit::NSView {
        use objc2::ClassType;

        self.as_button().as_super().as_super()
    }

    fn open_context_menu(&self, event: &objc2_app_kit::NSEvent) {
        self.update_tray_rect();
        self.set_button_highlighted(false);

        let Some(menu) = self.context_menu() else {
            log::warn!("tray context menu: status button menu missing");
            return;
        };
        log::debug!("tray context menu: status button opening native menu");
        if !pop_up_native_tray_menu_at_view(&menu, self.as_view()) {
            objc2_app_kit::NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self.as_view());
        }
    }

    fn open_context_menu_without_event(&self) {
        self.update_tray_rect();
        self.set_button_highlighted(false);

        let Some(menu) = self.context_menu() else {
            log::warn!("tray context menu: status button menu missing");
            return;
        };
        let Some(status_item) = self.status_item() else {
            log::warn!("tray context menu: status item missing");
            return;
        };

        log::debug!("tray context menu: status button gesture opening native menu");
        if pop_up_native_tray_menu_at_view(&menu, self.as_view()) {
            return;
        }

        #[allow(deprecated)]
        {
            status_item.popUpStatusItemMenu(&menu);
        }
    }

    fn context_menu(&self) -> Option<&'static objc2_app_kit::NSMenu> {
        let menu_ptr = self.menu_ptr()?;
        Some(unsafe { &*(menu_ptr as *const objc2_app_kit::NSMenu) })
    }

    fn status_item(&self) -> Option<&'static objc2_app_kit::NSStatusItem> {
        let status_item_ptr = self.status_item_ptr()?;
        Some(unsafe { &*(status_item_ptr as *const objc2_app_kit::NSStatusItem) })
    }

    fn set_button_highlighted(&self, highlighted: bool) {
        self.as_button().highlight(highlighted);
    }

    fn update_tray_rect(&self) {
        update_native_tray_rect_from_view(self.as_view());
    }

    fn key(&self) -> usize {
        self.as_status_button() as *const objc2_app_kit::NSStatusBarButton as usize
    }

    fn app_handle(&self) -> Option<AppHandle> {
        with_native_status_button_state(self.key(), |state| state.app_handle.clone())
    }

    fn menu_ptr(&self) -> Option<usize> {
        with_native_status_button_state(self.key(), |state| state.menu_ptr)
    }

    fn status_item_ptr(&self) -> Option<usize> {
        with_native_status_button_state(self.key(), |state| state.status_item_ptr)
    }

    fn set_status_item_menu_enabled(&self, enabled: bool) {
        let Some(status_item) = self.status_item() else {
            return;
        };
        if enabled {
            status_item.setMenu(self.context_menu());
        } else {
            status_item.setMenu(None);
        }
    }

    fn set_suppress_next_mouse_up(&self, value: bool) {
        let _ = with_native_status_button_state(self.key(), |state| {
            state.suppress_next_mouse_up = value;
        });
    }

    fn take_suppress_next_mouse_up(&self) -> bool {
        with_native_status_button_state(self.key(), |state| {
            let value = state.suppress_next_mouse_up;
            state.suppress_next_mouse_up = false;
            value
        })
        .unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
fn install_native_status_button(
    button: &objc2_app_kit::NSStatusBarButton,
    status_item: &objc2_app_kit::NSStatusItem,
    app_handle: AppHandle,
    menu: &objc2_app_kit::NSMenu,
) {
    use objc2::ClassType;

    let key = button as *const objc2_app_kit::NSStatusBarButton as usize;
    let menu_ptr = objc2::rc::Retained::into_raw(menu.retain()) as usize;
    let status_item_ptr = objc2::rc::Retained::into_raw(status_item.retain()) as usize;
    if let Ok(mut states) = native_status_button_states().lock() {
        states.insert(
            key,
            NativeStatusButtonState {
                app_handle,
                menu_ptr,
                status_item_ptr,
                suppress_next_mouse_up: false,
            },
        );
    }

    unsafe {
        object_setClass(
            (button as *const objc2_app_kit::NSStatusBarButton)
                .cast_mut()
                .cast(),
            NativeTrayStatusButton::class() as *const objc2::runtime::AnyClass,
        );
    }

    let target: &objc2::runtime::AnyObject = unsafe {
        &*(button as *const objc2_app_kit::NSStatusBarButton).cast::<objc2::runtime::AnyObject>()
    };
    let view: &objc2_app_kit::NSView = button.as_super().as_super().as_super();
    install_tray_click_gestures(view, target);
}

#[cfg(target_os = "macos")]
fn native_status_button_states() -> &'static NativeStatusButtonStateMap {
    static STATES: std::sync::OnceLock<NativeStatusButtonStateMap> = std::sync::OnceLock::new();

    STATES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(target_os = "macos")]
fn with_native_status_button_state<R>(
    key: usize,
    f: impl FnOnce(&mut NativeStatusButtonState) -> R,
) -> Option<R> {
    native_status_button_states()
        .lock()
        .ok()
        .and_then(|mut states| states.get_mut(&key).map(f))
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn object_setClass(
        obj: *mut objc2::runtime::NSObject,
        cls: *const objc2::runtime::AnyClass,
    ) -> *const objc2::runtime::AnyClass;
}

#[cfg(target_os = "macos")]
fn update_native_tray_rect_from_view(view: &objc2_app_kit::NSView) {
    let Some(screen_frame) = status_view_screen_frame(view) else {
        return;
    };

    let Some(window) = view.window() else {
        return;
    };
    let mtm = objc2_foundation::MainThreadMarker::new().expect("main thread");
    let main_screen_height = objc2_app_kit::NSScreen::mainScreen(mtm)
        .map(|screen| screen.frame().size.height)
        .unwrap_or_else(|| {
            let frame = screen_frame;
            frame.origin.y + frame.size.height
        });
    let (position, size) = native_tray_rect_from_status_window_frame(
        screen_frame,
        window.backingScaleFactor(),
        main_screen_height,
    );
    crate::panel::set_native_tray_rect(position, size);
}

#[cfg(target_os = "macos")]
fn status_view_screen_frame(view: &objc2_app_kit::NSView) -> Option<objc2_foundation::NSRect> {
    let window = view.window()?;
    let bounds = view.bounds();
    let window_rect = view.convertRect_toView(bounds, None);
    let screen_rect = window.convertRectToScreen(window_rect);

    (screen_rect.size.width >= 1.0 && screen_rect.size.height >= 1.0).then_some(screen_rect)
}

#[cfg(target_os = "macos")]
fn native_tray_rect_from_status_window_frame(
    frame: objc2_foundation::NSRect,
    scale_factor: f64,
    main_screen_height: f64,
) -> (tauri::Position, tauri::Size) {
    let scale_factor = scale_factor.max(1.0);
    let physical_x = round_to_i32(frame.origin.x * scale_factor);
    let physical_y =
        round_to_i32((main_screen_height - frame.origin.y - frame.size.height) * scale_factor);
    let physical_w = round_to_u32(frame.size.width * scale_factor);
    let physical_h = round_to_u32(frame.size.height * scale_factor);

    (
        tauri::Position::Physical(tauri::PhysicalPosition::new(physical_x, physical_y)),
        tauri::Size::Physical(tauri::PhysicalSize::new(physical_w, physical_h)),
    )
}

#[cfg(target_os = "macos")]
fn round_to_i32(value: f64) -> i32 {
    value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

#[cfg(target_os = "macos")]
fn round_to_u32(value: f64) -> u32 {
    value.round().clamp(1.0, u32::MAX as f64) as u32
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_mouse_button_number(
    button_number: objc2_foundation::NSInteger,
) -> bool {
    button_number > 0
}

#[cfg(target_os = "macos")]
fn remove_status_item_overlay_subviews(view: &objc2_app_kit::NSView) {
    let subviews: Vec<_> = view.subviews().into_iter().collect();
    let mut removed_count = 0;

    for subview in subviews {
        if subview.class().name().to_bytes() == b"TaoTrayTarget" {
            subview.removeFromSuperview();
            removed_count += 1;
        }
    }

    if removed_count > 0 {
        log::debug!("tray context menu: removed {removed_count} status button overlay subview(s)");
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayEventTapState {
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_item: objc2::rc::Retained<objc2_app_kit::NSStatusItem>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayInputOverlayViewIvars {
    app_handle: AppHandle,
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_item: objc2::rc::Retained<objc2_app_kit::NSStatusItem>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    touch_context_menu_opened: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSView))]
    #[name = "OpenUsageTrayInputOverlayView"]
    #[ivars = TrayInputOverlayViewIvars]
    #[derive(Debug)]
    struct TrayInputOverlayView;

    impl TrayInputOverlayView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&objc2_app_kit::NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &objc2_app_kit::NSEvent) {
            self.ivars().status_item.setMenu(None);
            self.update_tray_rect();
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            log::debug!("tray context menu: overlay left mouse up");
            self.update_tray_rect();
            let app_handle = self.ivars().app_handle.clone();
            toggle_panel(&app_handle);
            self.ivars().status_item.setMenu(Some(&self.ivars().menu));
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, _event: &objc2_app_kit::NSEvent) {
            log::debug!("tray context menu: overlay right mouse down");
            self.open_context_menu();
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            if should_open_tray_menu_from_mouse_button_number(event.buttonNumber()) {
                log::debug!(
                    "tray context menu: overlay other mouse down button={}",
                    event.buttonNumber()
                );
                self.open_context_menu();
            }
        }

        #[unsafe(method(touchesBeganWithEvent:))]
        fn touches_began(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touches(event);
        }

        #[unsafe(method(touchesMovedWithEvent:))]
        fn touches_moved(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touches(event);
        }

        #[unsafe(method(touchesEndedWithEvent:))]
        fn touches_ended(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touches(event);
        }

        #[unsafe(method(touchesCancelledWithEvent:))]
        fn touches_cancelled(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touches(event);
            self.ivars().touch_context_menu_opened.set(false);
        }

        #[unsafe(method(openContextMenuFromClickGesture:))]
        fn open_context_menu_from_click_gesture(
            &self,
            recognizer: &objc2_app_kit::NSClickGestureRecognizer,
        ) {
            log::debug!(
                "tray context menu: overlay click gesture button_mask={} touches={}",
                recognizer.buttonMask(),
                recognizer.numberOfTouchesRequired()
            );
            self.open_context_menu();
        }

        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(
            &self,
            event: &objc2_app_kit::NSEvent,
        ) -> Option<&'static objc2_app_kit::NSMenu> {
            if should_open_tray_menu_from_native_event(event) {
                log::debug!(
                    "tray context menu: overlay returning menu for event type={:?}",
                    event.r#type()
                );
                self.update_tray_rect();
                Some(self.context_menu())
            } else {
                None
            }
        }

    }
);

#[cfg(target_os = "macos")]
impl TrayInputOverlayView {
    fn update_tray_rect(&self) {
        update_native_tray_rect_from_view(&self.ivars().status_view);
    }

    fn open_context_menu(&self) {
        self.update_tray_rect();
        show_native_tray_menu_without_recent_guard(&self.ivars().status_item, &self.ivars().menu);
    }

    fn context_menu(&self) -> &'static objc2_app_kit::NSMenu {
        unsafe { &*(self.ivars().menu.as_ref() as *const objc2_app_kit::NSMenu) }
    }

    fn handle_touches(&self, event: &objc2_app_kit::NSEvent) {
        use objc2::ClassType;

        let view: &objc2_app_kit::NSView = self.as_super();
        let touching_touch_count = event
            .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, Some(view))
            .count() as usize;
        let event_touch_count = event.touchesForView(view).count() as usize;

        if should_open_tray_menu_from_touch_counts(touching_touch_count, event_touch_count)
            && !self.ivars().touch_context_menu_opened.replace(true)
        {
            log::debug!(
                "tray context menu: overlay two-touch open touching={} event={}",
                touching_touch_count,
                event_touch_count
            );
            self.open_context_menu();
        }

        if touching_touch_count < 2 {
            self.ivars().touch_context_menu_opened.set(false);
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayInputOverlaySyncState {
    overlay_window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
    overlay_view: objc2::rc::Retained<TrayInputOverlayView>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
}

#[cfg(target_os = "macos")]
fn install_tray_input_overlay(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2::ClassType;
    use objc2_app_kit::{
        NSBackingStoreType, NSColor, NSPopUpMenuWindowLevel, NSWindow, NSWindowCollectionBehavior,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect};

    let Some(frame) = tray_input_overlay_frame_from_status_view(status_view) else {
        log::warn!("tray context menu: status button screen frame unavailable");
        return;
    };
    let overlay_window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            objc2_foundation::MainThreadMarker::new()
                .expect("main thread")
                .alloc(),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        overlay_window.setReleasedWhenClosed(false);
    }
    overlay_window.setOpaque(false);
    overlay_window.setBackgroundColor(Some(&NSColor::clearColor()));
    overlay_window.setIgnoresMouseEvents(false);
    overlay_window.setLevel(NSPopUpMenuWindowLevel - 1);
    overlay_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );

    let overlay_view = unsafe {
        let view = objc2_foundation::MainThreadMarker::new()
            .expect("main thread")
            .alloc()
            .set_ivars(TrayInputOverlayViewIvars {
                app_handle: app_handle.clone(),
                menu: menu.retain(),
                status_item: status_item.retain(),
                status_view: status_view.retain(),
                touch_context_menu_opened: std::cell::Cell::new(false),
            });
        let view: objc2::rc::Retained<TrayInputOverlayView> = objc2::msg_send![
            super(view),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), frame.size)
        ];
        view
    };
    let overlay_ns_view: &objc2_app_kit::NSView = overlay_view.as_super();
    overlay_ns_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    #[allow(deprecated)]
    overlay_ns_view.setAcceptsTouchEvents(true);
    overlay_ns_view.setWantsRestingTouches(true);
    overlay_ns_view.setAllowedTouchTypes(objc2_app_kit::NSTouchTypeMask::Indirect);
    install_tray_overlay_click_gestures(overlay_ns_view, &overlay_view);
    overlay_window.setContentView(Some(overlay_ns_view));
    overlay_window.orderFrontRegardless();
    log::debug!(
        "tray context menu: installed input overlay frame=({:.1},{:.1},{:.1},{:.1}) status_view_bounds=({:.1},{:.1},{:.1},{:.1}) level={}",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
        status_view.bounds().origin.x,
        status_view.bounds().origin.y,
        status_view.bounds().size.width,
        status_view.bounds().size.height,
        NSPopUpMenuWindowLevel - 1
    );

    install_tray_input_overlay_sync_timer(&overlay_window, &overlay_view, status_view);

    std::mem::forget(overlay_window);
    std::mem::forget(overlay_view);
}

#[cfg(target_os = "macos")]
fn install_tray_overlay_click_gestures(
    view: &objc2_app_kit::NSView,
    target: &TrayInputOverlayView,
) {
    let target: &objc2::runtime::AnyObject =
        unsafe { &*(target as *const TrayInputOverlayView).cast::<objc2::runtime::AnyObject>() };

    install_tray_click_gestures(view, target);
}

#[cfg(target_os = "macos")]
fn install_tray_click_gestures(view: &objc2_app_kit::NSView, target: &objc2::runtime::AnyObject) {
    use objc2::ClassType;
    use objc2_app_kit::{NSClickGestureRecognizer, NSGestureRecognizer};

    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot install click gestures off main thread");
        return;
    };

    let secondary_click = NSClickGestureRecognizer::new(mtm);
    configure_tray_overlay_click_gesture(&secondary_click, target, 1 << 1, 1, false, true);
    let secondary_click: &NSGestureRecognizer = secondary_click.as_super();
    view.addGestureRecognizer(secondary_click);

    let two_touch_click = NSClickGestureRecognizer::new(mtm);
    configure_tray_overlay_click_gesture(&two_touch_click, target, 1 << 0, 2, true, false);
    let two_touch_click: &NSGestureRecognizer = two_touch_click.as_super();
    view.addGestureRecognizer(two_touch_click);

    log::debug!("tray context menu: installed click gestures");
}

#[cfg(target_os = "macos")]
fn configure_tray_overlay_click_gesture(
    recognizer: &objc2_app_kit::NSClickGestureRecognizer,
    target: &objc2::runtime::AnyObject,
    button_mask: objc2_foundation::NSUInteger,
    touch_count: objc2_foundation::NSInteger,
    delay_primary: bool,
    delay_secondary: bool,
) {
    use objc2::ClassType;
    use objc2_app_kit::NSGestureRecognizer;

    recognizer.setButtonMask(button_mask);
    recognizer.setNumberOfClicksRequired(1);
    recognizer.setNumberOfTouchesRequired(touch_count);

    let gesture: &NSGestureRecognizer = recognizer.as_super();
    unsafe {
        gesture.setTarget(Some(target));
        gesture.setAction(Some(objc2::sel!(openContextMenuFromClickGesture:)));
    }
    gesture.setDelaysPrimaryMouseButtonEvents(delay_primary);
    gesture.setDelaysSecondaryMouseButtonEvents(delay_secondary);
    gesture.setDelaysOtherMouseButtonEvents(false);
}

#[cfg(target_os = "macos")]
fn install_tray_input_overlay_sync_timer(
    overlay_window: &objc2_app_kit::NSWindow,
    overlay_view: &TrayInputOverlayView,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2::ClassType;
    use objc2_foundation::{NSPoint, NSRect, NSTimer};
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(TrayInputOverlaySyncState {
        overlay_window: overlay_window.retain(),
        overlay_view: overlay_view.retain(),
        status_view: status_view.retain(),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        if let Some(frame) = tray_input_overlay_frame_from_status_view(&block_state.status_view) {
            block_state.overlay_window.setFrame_display(frame, false);
            let overlay_ns_view: &objc2_app_kit::NSView = block_state.overlay_view.as_super();
            overlay_ns_view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        }
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSTimer>)> = &block;
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, block_ref) };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(timer);
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_frame_from_status_view(
    view: &objc2_app_kit::NSView,
) -> Option<objc2_foundation::NSRect> {
    status_view_screen_frame(view).map(tray_input_overlay_frame_from_status_frame)
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_frame_from_status_frame(
    frame: objc2_foundation::NSRect,
) -> objc2_foundation::NSRect {
    objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(
            frame.origin.x - STATUS_ITEM_HORIZONTAL_HIT_PADDING,
            frame.origin.y - STATUS_ITEM_VERTICAL_HIT_PADDING,
        ),
        objc2_foundation::NSSize::new(
            frame.size.width + (STATUS_ITEM_HORIZONTAL_HIT_PADDING * 2.0),
            frame.size.height + (STATUS_ITEM_VERTICAL_HIT_PADDING * 2.0),
        ),
    )
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TraySecondaryClickPollState {
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_item: objc2::rc::Retained<objc2_app_kit::NSStatusItem>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    secondary_button_was_down: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
fn install_secondary_click_poll_timer(
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2_foundation::NSTimer;
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(TraySecondaryClickPollState {
        menu: menu.retain(),
        status_item: status_item.retain(),
        status_view: status_view.retain(),
        secondary_button_was_down: std::cell::Cell::new(false),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let secondary_button_is_down = is_secondary_mouse_button_pressed();
        let secondary_button_was_down = block_state
            .secondary_button_was_down
            .replace(secondary_button_is_down);

        if should_open_tray_menu_from_polled_button_state(
            secondary_button_was_down,
            secondary_button_is_down,
            is_mouse_inside_status_view(&block_state.status_view),
        ) {
            log::debug!("tray context menu: polled secondary button down");
            show_native_tray_menu(&block_state.status_item, &block_state.menu);
        }
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSTimer>)> = &block;
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.025, true, block_ref) };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(timer);
    log::debug!("tray context menu: installed secondary-click poll timer");
}

#[cfg(target_os = "macos")]
fn install_status_item_event_tap(
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2_core_graphics::CGEventTapLocation;

    install_status_item_event_tap_at_location(
        menu,
        status_item,
        status_view,
        CGEventTapLocation::HIDEventTap,
    );
    install_status_item_event_tap_at_location(
        menu,
        status_item,
        status_view,
        CGEventTapLocation::SessionEventTap,
    );
    install_status_item_event_tap_at_location(
        menu,
        status_item,
        status_view,
        CGEventTapLocation::AnnotatedSessionEventTap,
    );
}

#[cfg(target_os = "macos")]
fn install_status_item_event_tap_at_location(
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
    location: objc2_core_graphics::CGEventTapLocation,
) {
    use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
    use objc2_core_graphics::{CGEvent, CGEventTapOptions, CGEventTapPlacement, CGEventType};

    let state = Box::new(TrayEventTapState {
        menu: menu.retain(),
        status_item: status_item.retain(),
        status_view: status_view.retain(),
    });
    let state_ptr = Box::into_raw(state);
    let event_mask = cg_event_mask(CGEventType::RightMouseDown)
        | cg_event_mask(CGEventType::RightMouseUp)
        | cg_event_mask(CGEventType::OtherMouseDown)
        | cg_event_mask(CGEventType::OtherMouseUp)
        | cg_event_mask(CGEventType::LeftMouseDown)
        | cg_event_mask(CGEventType::LeftMouseUp);

    let Some(tap) = (unsafe {
        CGEvent::tap_create(
            location,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            event_mask,
            Some(status_item_event_tap_callback),
            state_ptr.cast(),
        )
    }) else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!(
            "tray context menu: CoreGraphics {:?} event tap unavailable",
            location
        );
        return;
    };

    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!(
            "tray context menu: CoreGraphics {:?} event tap source unavailable",
            location
        );
        return;
    };

    if let Some(run_loop) = CFRunLoop::main() {
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        CGEvent::tap_enable(&tap, true);
        std::mem::forget(tap);
        std::mem::forget(source);
        log::debug!(
            "tray context menu: installed CoreGraphics {:?} event tap",
            location
        );
    } else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!(
            "tray context menu: CoreGraphics {:?} main run loop unavailable",
            location
        );
    }
}

#[cfg(target_os = "macos")]
fn cg_event_mask(event_type: objc2_core_graphics::CGEventType) -> objc2_core_graphics::CGEventMask {
    1u64 << event_type.0
}

#[cfg(target_os = "macos")]
unsafe extern "C-unwind" fn status_item_event_tap_callback(
    _proxy: objc2_core_graphics::CGEventTapProxy,
    event_type: objc2_core_graphics::CGEventType,
    event: std::ptr::NonNull<objc2_core_graphics::CGEvent>,
    user_info: *mut std::ffi::c_void,
) -> *mut objc2_core_graphics::CGEvent {
    if user_info.is_null()
        || event_type == objc2_core_graphics::CGEventType::TapDisabledByTimeout
        || event_type == objc2_core_graphics::CGEventType::TapDisabledByUserInput
    {
        return event.as_ptr();
    }

    let state = unsafe { &*(user_info.cast::<TrayEventTapState>()) };
    let cg_event = unsafe { event.as_ref() };
    if should_open_tray_menu_from_cg_event(event_type, cg_event)
        && is_mouse_inside_status_view(&state.status_view)
    {
        log::debug!(
            "tray context menu: CoreGraphics event tap open event_type={:?}",
            event_type
        );
        show_native_tray_menu(&state.status_item, &state.menu);
    }

    event.as_ptr()
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event(
    event_type: objc2_core_graphics::CGEventType,
    event: &objc2_core_graphics::CGEvent,
) -> bool {
    use objc2_core_graphics::CGEvent;

    should_open_tray_menu_from_cg_event_type(event_type, CGEvent::flags(Some(event)))
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event_type(
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
) -> bool {
    use objc2_core_graphics::{CGEventFlags, CGEventType};

    event_type == CGEventType::RightMouseDown
        || event_type == CGEventType::RightMouseUp
        || event_type == CGEventType::OtherMouseDown
        || event_type == CGEventType::OtherMouseUp
        || ((event_type == CGEventType::LeftMouseDown || event_type == CGEventType::LeftMouseUp)
            && flags.contains(CGEventFlags::MaskControl))
}

#[cfg(target_os = "macos")]
fn is_secondary_mouse_button_pressed() -> bool {
    use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGMouseButton};

    is_appkit_secondary_mouse_button_pressed(objc2_app_kit::NSEvent::pressedMouseButtons() as usize)
        || CGEventSource::button_state(
            CGEventSourceStateID::CombinedSessionState,
            CGMouseButton::Right,
        )
        || CGEventSource::button_state(CGEventSourceStateID::HIDSystemState, CGMouseButton::Right)
}

#[cfg(target_os = "macos")]
fn is_appkit_secondary_mouse_button_pressed(pressed_mouse_buttons: usize) -> bool {
    const SECONDARY_MOUSE_BUTTON_MASK: usize = 1 << 1;

    pressed_mouse_buttons & SECONDARY_MOUSE_BUTTON_MASK != 0
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_polled_button_state(
    secondary_button_was_down: bool,
    secondary_button_is_down: bool,
    cursor_inside_status_window: bool,
) -> bool {
    !secondary_button_was_down && secondary_button_is_down && cursor_inside_status_window
}

#[cfg(target_os = "macos")]
fn set_context_menu_on_view_tree(view: &objc2_app_kit::NSView, menu: &objc2_app_kit::NSMenu) {
    use objc2::ClassType;
    use objc2_app_kit::NSResponder;

    let responder: &NSResponder = view.as_super();
    unsafe {
        responder.setMenu(Some(menu));
    }

    for subview in view.subviews() {
        set_context_menu_on_view_tree(&subview, menu);
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn show_native_tray_menu(status_item: &objc2_app_kit::NSStatusItem, menu: &objc2_app_kit::NSMenu) {
    if should_skip_recent_native_menu_open(std::time::Instant::now()) {
        return;
    }

    show_native_tray_menu_without_recent_guard(status_item, menu);
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn show_native_tray_menu_without_recent_guard(
    status_item: &objc2_app_kit::NSStatusItem,
    menu: &objc2_app_kit::NSMenu,
) {
    use objc2::ClassType;

    log::debug!("tray context menu: showing native menu");
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot show menu off main thread");
        return;
    };
    let Some(button) = status_item.button(mtm) else {
        log::debug!("tray context menu: status button missing, using status item popup");
        status_item.popUpStatusItemMenu(menu);
        return;
    };
    let view: &objc2_app_kit::NSView = button.as_super().as_super().as_super();
    update_native_tray_rect_from_view(view);
    if pop_up_native_tray_menu_at_view(menu, view) {
        return;
    }

    status_item.popUpStatusItemMenu(menu);
}

#[cfg(target_os = "macos")]
fn pop_up_native_tray_menu_at_view(
    menu: &objc2_app_kit::NSMenu,
    view: &objc2_app_kit::NSView,
) -> bool {
    menu.popUpMenuPositioningItem_atLocation_inView(
        None,
        objc2_foundation::NSPoint::new(0.0, 0.0),
        Some(view),
    )
}

#[cfg(target_os = "macos")]
fn should_skip_recent_native_menu_open(now: std::time::Instant) -> bool {
    static LAST_OPENED_AT: std::sync::Mutex<Option<std::time::Instant>> =
        std::sync::Mutex::new(None);

    let Ok(mut last_opened_at) = LAST_OPENED_AT.lock() else {
        return false;
    };

    if last_opened_at
        .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(200))
    {
        return true;
    }

    *last_opened_at = Some(now);
    false
}

#[cfg(target_os = "macos")]
fn is_mouse_inside_status_view(view: &objc2_app_kit::NSView) -> bool {
    let Some(frame) = status_view_screen_frame(view) else {
        return false;
    };
    let point = objc2_app_kit::NSEvent::mouseLocation();
    is_point_inside_rect_with_padding(
        point,
        frame,
        STATUS_ITEM_HORIZONTAL_HIT_PADDING,
        STATUS_ITEM_VERTICAL_HIT_PADDING,
    )
}

#[cfg(target_os = "macos")]
fn is_point_inside_rect_with_padding(
    point: objc2_foundation::NSPoint,
    rect: objc2_foundation::NSRect,
    horizontal_padding: f64,
    vertical_padding: f64,
) -> bool {
    point.x >= rect.origin.x - horizontal_padding
        && point.x <= rect.origin.x + rect.size.width + horizontal_padding
        && point.y >= rect.origin.y - vertical_padding
        && point.y <= rect.origin.y + rect.size.height + vertical_padding
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_native_event(event: &objc2_app_kit::NSEvent) -> bool {
    should_open_tray_menu_from_native_event_type(event.r#type(), event.modifierFlags())
        || should_open_tray_menu_from_trackpad_event_type(
            event.r#type(),
            objc2_app_kit::NSEvent::pressedMouseButtons() as usize,
        )
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_native_event_type(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSEventType};

    event_type == NSEventType::RightMouseDown
        || event_type == NSEventType::RightMouseUp
        || event_type == NSEventType::OtherMouseDown
        || event_type == NSEventType::OtherMouseUp
        || event_type == NSEventType::SystemDefined
        || event_type == NSEventType::Gesture
        || event_type == NSEventType::BeginGesture
        || event_type == NSEventType::EndGesture
        || event_type == NSEventType::Pressure
        || event_type == NSEventType::DirectTouch
        || (event_type == NSEventType::LeftMouseDown
            && modifier_flags.contains(NSEventModifierFlags::Control))
        || (event_type == NSEventType::LeftMouseUp
            && modifier_flags.contains(NSEventModifierFlags::Control))
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_trackpad_event_type(
    event_type: objc2_app_kit::NSEventType,
    pressed_mouse_buttons: usize,
) -> bool {
    use objc2_app_kit::NSEventType;

    is_appkit_secondary_mouse_button_pressed(pressed_mouse_buttons)
        && (event_type == NSEventType::SystemDefined
            || event_type == NSEventType::Gesture
            || event_type == NSEventType::BeginGesture
            || event_type == NSEventType::EndGesture
            || event_type == NSEventType::Pressure
            || event_type == NSEventType::DirectTouch)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_touch_counts(
    touching_touch_count: usize,
    event_touch_count: usize,
) -> bool {
    touching_touch_count >= 2 || event_touch_count >= 2
}

#[cfg(target_os = "macos")]
fn build_native_tray_context_menu(
    current_level: log::LevelFilter,
) -> Result<muda::Menu, muda::Error> {
    let show_stats = muda::MenuItem::with_id("show_stats", "Show Stats", true, None);
    let go_to_settings = muda::MenuItem::with_id("go_to_settings", "Go to Settings", true, None);
    let log_error = muda::CheckMenuItem::with_id(
        "log_error",
        "Error",
        true,
        current_level == log::LevelFilter::Error,
        None,
    );
    let log_warn = muda::CheckMenuItem::with_id(
        "log_warn",
        "Warn",
        true,
        current_level == log::LevelFilter::Warn,
        None,
    );
    let log_info = muda::CheckMenuItem::with_id(
        "log_info",
        "Info",
        true,
        current_level == log::LevelFilter::Info,
        None,
    );
    let log_debug = muda::CheckMenuItem::with_id(
        "log_debug",
        "Debug",
        true,
        current_level == log::LevelFilter::Debug,
        None,
    );
    let log_trace = muda::CheckMenuItem::with_id(
        "log_trace",
        "Trace",
        true,
        current_level == log::LevelFilter::Trace,
        None,
    );
    let log_level_separator = muda::PredefinedMenuItem::separator();
    let copy_log_path = muda::MenuItem::with_id("copy_log_path", "Copy Log Path", true, None);
    let log_level_submenu = muda::Submenu::with_items(
        "Debug Level",
        true,
        &[
            &log_error,
            &log_warn,
            &log_info,
            &log_debug,
            &log_trace,
            &log_level_separator,
            &copy_log_path,
        ],
    )?;
    let separator = muda::PredefinedMenuItem::separator();
    let about = muda::MenuItem::with_id("about", "About OpenUsage", true, None);
    let quit = muda::MenuItem::with_id("quit", "Quit", true, None);

    muda::Menu::with_items(&[
        &show_stats,
        &go_to_settings,
        &log_level_submenu,
        &separator,
        &about,
        &quit,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_tray_menu_opening_matches_platform_behavior() {
        #[cfg(target_os = "macos")]
        {
            assert!(should_open_tray_menu(
                MouseButton::Right,
                MouseButtonState::Down
            ));
            assert!(!should_open_tray_menu(
                MouseButton::Right,
                MouseButtonState::Up
            ));
            assert!(!should_open_tray_menu(
                MouseButton::Left,
                MouseButtonState::Up
            ));
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert!(should_open_tray_menu(
                MouseButton::Right,
                MouseButtonState::Up
            ));
            assert!(!should_open_tray_menu(
                MouseButton::Right,
                MouseButtonState::Down
            ));
            assert!(!should_open_tray_menu(
                MouseButton::Left,
                MouseButtonState::Up
            ));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_status_item_events_open_tray_menu() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::RightMouseDown,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::OtherMouseDown,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::Control
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::RightMouseUp,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::OtherMouseUp,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::LeftMouseUp,
            NSEventModifierFlags::Control
        ));
        assert!(!should_open_tray_menu_from_native_event_type(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::SystemDefined,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::Gesture,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::DirectTouch,
            NSEventModifierFlags::empty()
        ));
        assert!(should_open_tray_menu_from_native_event_type(
            NSEventType::Pressure,
            NSEventModifierFlags::empty()
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_points_inside_status_frame() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let rect = NSRect::new(NSPoint::new(100.0, 900.0), NSSize::new(24.0, 22.0));

        assert!(is_point_inside_rect_with_padding(
            NSPoint::new(112.0, 911.0),
            rect,
            STATUS_ITEM_HORIZONTAL_HIT_PADDING,
            STATUS_ITEM_VERTICAL_HIT_PADDING
        ));
        assert!(is_point_inside_rect_with_padding(
            NSPoint::new(112.0, 928.0),
            rect,
            STATUS_ITEM_HORIZONTAL_HIT_PADDING,
            STATUS_ITEM_VERTICAL_HIT_PADDING
        ));
        assert!(!is_point_inside_rect_with_padding(
            NSPoint::new(80.0, 911.0),
            rect,
            STATUS_ITEM_HORIZONTAL_HIT_PADDING,
            STATUS_ITEM_VERTICAL_HIT_PADDING
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_frame_expands_status_hit_area() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let frame = NSRect::new(NSPoint::new(100.0, 900.0), NSSize::new(24.0, 22.0));
        let overlay = tray_input_overlay_frame_from_status_frame(frame);

        assert_eq!(
            overlay.origin.x,
            frame.origin.x - STATUS_ITEM_HORIZONTAL_HIT_PADDING
        );
        assert_eq!(
            overlay.origin.y,
            frame.origin.y - STATUS_ITEM_VERTICAL_HIT_PADDING
        );
        assert_eq!(
            overlay.size.width,
            frame.size.width + (STATUS_ITEM_HORIZONTAL_HIT_PADDING * 2.0)
        );
        assert_eq!(
            overlay.size.height,
            frame.size.height + (STATUS_ITEM_VERTICAL_HIT_PADDING * 2.0)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_status_window_frame_converts_to_physical_tray_rect() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let frame = NSRect::new(NSPoint::new(100.0, 978.0), NSSize::new(24.0, 22.0));
        let (position, size) = native_tray_rect_from_status_window_frame(frame, 2.0, 1000.0);

        let tauri::Position::Physical(position) = position else {
            panic!("expected physical position");
        };
        let tauri::Size::Physical(size) = size else {
            panic!("expected physical size");
        };

        assert_eq!(position.x, 200);
        assert_eq!(position.y, 0);
        assert_eq!(size.width, 48);
        assert_eq!(size.height, 44);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn non_primary_mouse_buttons_open_tray_menu() {
        assert!(!should_open_tray_menu_from_mouse_button_number(0));
        assert!(should_open_tray_menu_from_mouse_button_number(1));
        assert!(should_open_tray_menu_from_mouse_button_number(2));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_graphics_right_mouse_down_mask_matches_event_type() {
        assert_eq!(
            cg_event_mask(objc2_core_graphics::CGEventType::RightMouseDown),
            1u64 << 3
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn core_graphics_mouse_down_or_up_can_open_tray_menu() {
        use objc2_core_graphics::{CGEventFlags, CGEventType};

        assert!(should_open_tray_menu_from_cg_event_type(
            CGEventType::RightMouseDown,
            CGEventFlags::empty()
        ));
        assert!(should_open_tray_menu_from_cg_event_type(
            CGEventType::RightMouseUp,
            CGEventFlags::empty()
        ));
        assert!(should_open_tray_menu_from_cg_event_type(
            CGEventType::OtherMouseDown,
            CGEventFlags::empty()
        ));
        assert!(should_open_tray_menu_from_cg_event_type(
            CGEventType::OtherMouseUp,
            CGEventFlags::empty()
        ));
        assert!(should_open_tray_menu_from_cg_event_type(
            CGEventType::LeftMouseUp,
            CGEventFlags::MaskControl
        ));
        assert!(!should_open_tray_menu_from_cg_event_type(
            CGEventType::LeftMouseUp,
            CGEventFlags::empty()
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn polled_secondary_click_opens_only_on_inside_down_edge() {
        assert!(should_open_tray_menu_from_polled_button_state(
            false, true, true
        ));
        assert!(!should_open_tray_menu_from_polled_button_state(
            true, true, true
        ));
        assert!(!should_open_tray_menu_from_polled_button_state(
            false, false, true
        ));
        assert!(!should_open_tray_menu_from_polled_button_state(
            false, true, false
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_pressed_mouse_buttons_detect_secondary_click() {
        assert!(!is_appkit_secondary_mouse_button_pressed(0));
        assert!(!is_appkit_secondary_mouse_button_pressed(1 << 0));
        assert!(is_appkit_secondary_mouse_button_pressed(1 << 1));
        assert!(is_appkit_secondary_mouse_button_pressed(
            (1 << 0) | (1 << 1)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn trackpad_events_open_only_when_secondary_button_is_pressed() {
        use objc2_app_kit::NSEventType;

        assert!(should_open_tray_menu_from_trackpad_event_type(
            NSEventType::DirectTouch,
            1 << 1
        ));
        assert!(should_open_tray_menu_from_trackpad_event_type(
            NSEventType::SystemDefined,
            1 << 1
        ));
        assert!(!should_open_tray_menu_from_trackpad_event_type(
            NSEventType::DirectTouch,
            0
        ));
        assert!(!should_open_tray_menu_from_trackpad_event_type(
            NSEventType::MouseMoved,
            1 << 1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn two_trackpad_touches_open_tray_menu() {
        assert!(should_open_tray_menu_from_touch_counts(2, 0));
        assert!(should_open_tray_menu_from_touch_counts(0, 2));
        assert!(!should_open_tray_menu_from_touch_counts(1, 1));
        assert!(!should_open_tray_menu_from_touch_counts(0, 0));
    }
}
