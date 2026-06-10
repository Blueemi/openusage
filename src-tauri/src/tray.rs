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
use objc2::{DefinedClass as _, Message as _};

const LOG_LEVEL_STORE_KEY: &str = "logLevel";

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
        use objc2_app_kit::{NSEvent, NSEventMask, NSMenu, NSView, NSWindow};
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
        let local_ns_menu = ns_menu.retain();
        let local_status_item = status_item.retain();
        let global_ns_menu = ns_menu.retain();
        let global_status_item = status_item.retain();
        let button_view: &NSView = button.as_super().as_super().as_super();
        remove_status_item_overlay_subviews(button_view);
        set_context_menu_on_view_tree(button_view, ns_menu);
        let status_mouse_target =
            TrayStatusMouseTarget::new(app_handle.clone(), ns_menu, &status_item);
        install_status_mouse_target_view(button_view, &status_mouse_target);

        let Some(window) = button.window() else {
            log::warn!("tray context menu: status item window unavailable");
            std::mem::forget(menu);
            std::mem::forget(status_mouse_target);
            return;
        };

        let status_window_number = window.windowNumber();
        let status_window: objc2::rc::Retained<NSWindow> = window.clone();
        install_status_item_event_tap(ns_menu, &status_item, &status_window);

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
            | NSEventMask::LeftMouseDown
            | NSEventMask::OtherMouseDown
            | NSEventMask::Gesture
            | NSEventMask::BeginGesture
            | NSEventMask::Pressure
            | NSEventMask::DirectTouch
            | NSEventMask::SystemDefined;
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
                || !is_mouse_inside_window(&status_window)
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
        std::mem::forget(status_mouse_target);
        std::mem::forget(block);
        std::mem::forget(global_block);
    }) {
        log::warn!("tray context menu: failed to install native menu: {error}");
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayStatusMouseTargetIvars {
    app_handle: AppHandle,
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_item: objc2::rc::Retained<objc2_app_kit::NSStatusItem>,
    suppress_next_mouse_up: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[unsafe(super(objc2_app_kit::NSView))]
    #[name = "OpenUsageTrayStatusMouseTarget"]
    #[ivars = TrayStatusMouseTargetIvars]
    struct TrayStatusMouseTarget;

    impl TrayStatusMouseTarget {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&objc2_app_kit::NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            if should_open_tray_menu_from_native_event(event)
                || should_open_tray_menu_from_mouse_button_number(event.buttonNumber())
            {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu(event);
                return;
            }

            self.set_button_highlighted(true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            self.set_button_highlighted(false);
            if self.ivars().suppress_next_mouse_up.replace(false) {
                return;
            }
            toggle_panel(&self.ivars().app_handle);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.open_context_menu(event);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.open_context_menu(event);
        }

    }
);

#[cfg(target_os = "macos")]
impl TrayStatusMouseTarget {
    fn new(
        app_handle: AppHandle,
        menu: &objc2_app_kit::NSMenu,
        status_item: &objc2_app_kit::NSStatusItem,
    ) -> objc2::rc::Retained<Self> {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("main thread");
        let target = mtm.alloc().set_ivars(TrayStatusMouseTargetIvars {
            app_handle,
            menu: menu.retain(),
            status_item: status_item.retain(),
            suppress_next_mouse_up: std::cell::Cell::new(false),
        });
        unsafe {
            objc2::msg_send![
                super(target),
                initWithFrame: objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(0.0, 0.0)
                )
            ]
        }
    }

    fn open_context_menu(&self, event: &objc2_app_kit::NSEvent) {
        use objc2::ClassType;

        if should_skip_recent_native_menu_open(std::time::Instant::now()) {
            return;
        }

        let view: &objc2_app_kit::NSView = self.as_super();
        objc2_app_kit::NSMenu::popUpContextMenu_withEvent_forView(&self.ivars().menu, event, view);
    }

    fn set_button_highlighted(&self, highlighted: bool) {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("main thread");
        if let Some(button) = self.ivars().status_item.button(mtm) {
            button.highlight(highlighted);
        }
    }
}

#[cfg(target_os = "macos")]
fn install_status_mouse_target_view(
    button_view: &objc2_app_kit::NSView,
    target: &TrayStatusMouseTarget,
) {
    use objc2::ClassType;

    let target_view: &objc2_app_kit::NSView = target.as_super();
    target_view.setFrame(button_view.bounds());
    target_view.setAutoresizingMask(status_mouse_target_autoresizing_mask());
    button_view.addSubview(target_view);
}

#[cfg(target_os = "macos")]
fn status_mouse_target_autoresizing_mask() -> objc2_app_kit::NSAutoresizingMaskOptions {
    use objc2_app_kit::NSAutoresizingMaskOptions;

    NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable
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
    window: objc2::rc::Retained<objc2_app_kit::NSWindow>,
}

#[cfg(target_os = "macos")]
fn install_status_item_event_tap(
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    window: &objc2_app_kit::NSWindow,
) {
    use objc2_core_foundation::{kCFRunLoopCommonModes, CFMachPort, CFRunLoop};
    use objc2_core_graphics::{
        CGEvent, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    };

    let state = Box::new(TrayEventTapState {
        menu: menu.retain(),
        status_item: status_item.retain(),
        window: window.retain(),
    });
    let state_ptr = Box::into_raw(state);
    let event_mask = cg_event_mask(CGEventType::RightMouseDown)
        | cg_event_mask(CGEventType::OtherMouseDown)
        | cg_event_mask(CGEventType::LeftMouseDown);

    let Some(tap) = (unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            event_mask,
            Some(status_item_event_tap_callback),
            state_ptr.cast(),
        )
    }) else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!("tray context menu: CoreGraphics event tap unavailable");
        return;
    };

    let Some(source) = CFMachPort::new_run_loop_source(None, Some(&tap), 0) else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!("tray context menu: CoreGraphics event tap source unavailable");
        return;
    };

    if let Some(run_loop) = CFRunLoop::main() {
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopCommonModes });
        CGEvent::tap_enable(&tap, true);
        std::mem::forget(tap);
        std::mem::forget(source);
        log::debug!("tray context menu: installed CoreGraphics event tap");
    } else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        log::warn!("tray context menu: CoreGraphics main run loop unavailable");
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
        && is_mouse_inside_window(&state.window)
    {
        show_native_tray_menu(&state.status_item, &state.menu);
    }

    event.as_ptr()
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event(
    event_type: objc2_core_graphics::CGEventType,
    event: &objc2_core_graphics::CGEvent,
) -> bool {
    use objc2_core_graphics::{CGEvent, CGEventFlags, CGEventType};

    event_type == CGEventType::RightMouseDown
        || event_type == CGEventType::OtherMouseDown
        || (event_type == CGEventType::LeftMouseDown
            && CGEvent::flags(Some(event)).contains(CGEventFlags::MaskControl))
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

    status_item.popUpStatusItemMenu(menu);
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
fn is_mouse_inside_window(window: &objc2_app_kit::NSWindow) -> bool {
    let point = objc2_app_kit::NSEvent::mouseLocation();
    is_point_inside_rect_with_padding(point, window.frame(), 3.0)
}

#[cfg(target_os = "macos")]
fn is_point_inside_rect_with_padding(
    point: objc2_foundation::NSPoint,
    rect: objc2_foundation::NSRect,
    padding: f64,
) -> bool {
    point.x >= rect.origin.x - padding
        && point.x <= rect.origin.x + rect.size.width + padding
        && point.y >= rect.origin.y - padding
        && point.y <= rect.origin.y + rect.size.height + padding
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_native_event(event: &objc2_app_kit::NSEvent) -> bool {
    should_open_tray_menu_from_native_event_type(event.r#type(), event.modifierFlags())
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_native_event_type(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSEventType};

    event_type == NSEventType::RightMouseDown
        || event_type == NSEventType::OtherMouseDown
        || (event_type == NSEventType::LeftMouseDown
            && modifier_flags.contains(NSEventModifierFlags::Control))
        || event_type == NSEventType::Gesture
        || event_type == NSEventType::BeginGesture
        || event_type == NSEventType::Pressure
        || event_type == NSEventType::DirectTouch
        || event_type == NSEventType::SystemDefined
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
            NSEventType::Gesture,
            NSEventModifierFlags::empty()
        ));
        assert!(!should_open_tray_menu_from_native_event_type(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty()
        ));
        assert!(!should_open_tray_menu_from_native_event_type(
            NSEventType::RightMouseUp,
            NSEventModifierFlags::empty()
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn detects_points_inside_status_window_frame() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let rect = NSRect::new(NSPoint::new(100.0, 900.0), NSSize::new(24.0, 22.0));

        assert!(is_point_inside_rect_with_padding(
            NSPoint::new(112.0, 911.0),
            rect,
            3.0
        ));
        assert!(is_point_inside_rect_with_padding(
            NSPoint::new(98.0, 899.0),
            rect,
            3.0
        ));
        assert!(!is_point_inside_rect_with_padding(
            NSPoint::new(80.0, 911.0),
            rect,
            3.0
        ));
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
}
