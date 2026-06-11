#![allow(dead_code)]

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
use objc2::MainThreadOnly;
#[cfg(target_os = "macos")]
use objc2::{AnyThread as _, ClassType as _, DeclaredClass as _, Message as _, msg_send};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSMenuDelegate;
#[cfg(target_os = "macos")]
use objc2_foundation::NSObjectProtocol;

const LOG_LEVEL_STORE_KEY: &str = "logLevel";

#[cfg(target_os = "macos")]
const STATUS_ITEM_HORIZONTAL_HIT_PADDING: f64 = 3.0;
#[cfg(target_os = "macos")]
const STATUS_ITEM_VERTICAL_HIT_PADDING: f64 = 12.0;
#[cfg(target_os = "macos")]
const MAX_TRAY_OVERLAY_EVENT_LOGS: usize = 24;
#[cfg(target_os = "macos")]
static TRAY_OVERLAY_EVENT_LOGS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

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
                    #[cfg(target_os = "macos")]
                    if native_menu_recently_opened_for_mouse_up(std::time::Instant::now()) {
                        log::debug!("tray click: ignoring left mouse up after native menu open");
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
        use objc2::ClassType;
        use objc2_app_kit::{NSMenu, NSView};
        use objc2_foundation::MainThreadMarker;

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
        #[allow(deprecated)]
        {
            status_item.setView(None);
        }
        let status_view: &NSView = button.as_super().as_super().as_super();
        set_context_menu_on_view_tree(status_view, ns_menu);
        accept_indirect_touch_events(status_view);
        update_native_tray_rect_from_view(status_view);
        install_status_item_system_menu(&app_handle, ns_menu, &status_item, status_view);
        install_status_button_action_target(
            &app_handle,
            ns_menu,
            &status_item,
            &button,
            status_view,
        );
        install_status_view_context_click_gestures(&app_handle, ns_menu, &status_item, status_view);
        install_status_item_event_tap(ns_menu, status_view);
        install_secondary_click_poll_timer(ns_menu, status_view);
        crate::macos_status_item_event_monitor::install(ns_menu, status_view);
        crate::macos_hid_secondary_click::install(ns_menu, status_view);
        crate::macos_trackpad::install_context_click_fallback(ns_menu, status_view);
        log::debug!("tray context menu: using native status button system menu");

        // Keep the muda menu alive for manually popped AppKit menu actions.
        std::mem::forget(menu);
    }) {
        log::warn!("tray context menu: failed to install native menu: {error}");
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayStatusItemMenuDelegateIvars {
    app_handle: AppHandle,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[derive(Debug)]
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OpenUsageTrayStatusItemMenuDelegate"]
    #[ivars = TrayStatusItemMenuDelegateIvars]
    struct TrayStatusItemMenuDelegate;

    unsafe impl NSObjectProtocol for TrayStatusItemMenuDelegate {}

    unsafe impl NSMenuDelegate for TrayStatusItemMenuDelegate {
        #[allow(non_snake_case)]
        #[unsafe(method(menuWillOpen:))]
        fn menuWillOpen(&self, menu: &objc2_app_kit::NSMenu) {
            let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
                log::warn!("tray context menu: status item menu delegate off main thread");
                return;
            };
            let application = objc2_app_kit::NSApplication::sharedApplication(mtm);
            let Some(event) = application.currentEvent() else {
                mark_native_menu_opened(std::time::Instant::now());
                log::debug!("tray context menu: system status menu opening without current event");
                return;
            };

            log::debug!(
                "tray context menu: system status menu opening event_type={:?} button={}",
                event.r#type(),
                event.buttonNumber()
            );
            mark_native_menu_opened(std::time::Instant::now());
            if !should_cancel_status_item_system_menu_for_primary_click(&event) {
                update_native_tray_rect_from_view(&self.ivars().status_view);
                return;
            }

            log::debug!("tray context menu: cancelling system menu for primary click");
            menu.cancelTrackingWithoutAnimation();
            update_native_tray_rect_from_view(&self.ivars().status_view);
            toggle_panel(&self.ivars().app_handle);
        }
    }
);

#[cfg(target_os = "macos")]
impl TrayStatusItemMenuDelegate {
    fn new(
        app_handle: &AppHandle,
        status_view: &objc2_app_kit::NSView,
    ) -> objc2::rc::Retained<Self> {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("main thread");
        let this = Self::alloc(mtm).set_ivars(TrayStatusItemMenuDelegateIvars {
            app_handle: app_handle.clone(),
            status_view: status_view.retain(),
        });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(target_os = "macos")]
fn install_status_item_system_menu(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
) {
    let delegate = TrayStatusItemMenuDelegate::new(app_handle, status_view);
    let delegate: objc2::rc::Retained<
        objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSMenuDelegate>,
    > = objc2::runtime::ProtocolObject::from_retained(delegate);

    menu.setDelegate(Some(&delegate));
    status_item.setMenu(Some(menu));

    // NSMenu's delegate is weak.
    std::mem::forget(delegate);
    log::debug!("tray context menu: installed status item system menu");
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayStatusButtonActionTargetIvars {
    app_handle: AppHandle,
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_item: objc2::rc::Retained<objc2_app_kit::NSStatusItem>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    last_event_number: std::cell::Cell<isize>,
    two_touch_menu_open: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[derive(Debug)]
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "OpenUsageTrayStatusButtonActionTarget"]
    #[ivars = TrayStatusButtonActionTargetIvars]
    struct TrayStatusButtonActionTarget;

    impl TrayStatusButtonActionTarget {
        #[unsafe(method(handleOpenUsageTrayStatusButtonAction:))]
        fn handle_status_button_action(&self, _sender: &objc2_app_kit::NSStatusBarButton) {
            let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
                log::warn!("tray context menu: status button action off main thread");
                return;
            };
            let application = objc2_app_kit::NSApplication::sharedApplication(mtm);
            let Some(event) = application.currentEvent() else {
                log::debug!("tray context menu: status button action without current event");
                return;
            };

            log::debug!(
                "tray context menu: status button action event_type={:?} event_number={} button={} touches={}",
                event.r#type(),
                event.eventNumber(),
                event.buttonNumber(),
                active_touch_count_for_event(&event, &self.ivars().status_view)
            );

            let ivars = self.ivars();
            let touch_count = active_touch_count_for_event(&event, &ivars.status_view);
            if should_open_tray_menu_from_touch_count(ivars.two_touch_menu_open.get(), touch_count)
            {
                ivars.two_touch_menu_open.set(true);
                log::debug!(
                    "tray context menu: status button action opening from two touches"
                );
                show_native_tray_menu(&ivars.status_item, &ivars.menu);
                return;
            }
            if should_reset_touch_menu_gate(touch_count) {
                ivars.two_touch_menu_open.set(false);
            }

            if should_handle_primary_status_item_click(&event) {
                match primary_status_item_click_action(
                    &event,
                    native_menu_recently_opened_for_mouse_up(std::time::Instant::now()),
                ) {
                    PrimaryStatusItemClickAction::TogglePanel => {
                        update_native_tray_rect_from_view(&ivars.status_view);
                        toggle_panel(&ivars.app_handle);
                    }
                    PrimaryStatusItemClickAction::Ignore => {}
                    PrimaryStatusItemClickAction::PassThrough => {}
                }
                return;
            }

            if !should_open_tray_menu_from_status_button_action_event(&event) {
                return;
            }

            let event_number = event.eventNumber();
            if event_number != 0 && event_number == ivars.last_event_number.get() {
                return;
            }
            ivars.last_event_number.set(event_number);

            show_native_tray_menu(&ivars.status_item, &ivars.menu);
        }

        #[unsafe(method(openOpenUsageTrayContextMenuFromClickGesture:))]
        fn open_context_menu_from_click_gesture(
            &self,
            recognizer: &objc2_app_kit::NSClickGestureRecognizer,
        ) {
            log::debug!(
                "tray context menu: status view click gesture button_mask={} touches={}",
                recognizer.buttonMask(),
                recognizer.numberOfTouchesRequired()
            );
            let ivars = self.ivars();
            show_native_tray_menu(&ivars.status_item, &ivars.menu);
        }
    }
);

#[cfg(target_os = "macos")]
impl TrayStatusButtonActionTarget {
    fn new(
        app_handle: &AppHandle,
        menu: &objc2_app_kit::NSMenu,
        status_item: &objc2_app_kit::NSStatusItem,
        status_view: &objc2_app_kit::NSView,
    ) -> objc2::rc::Retained<Self> {
        let this = Self::alloc().set_ivars(TrayStatusButtonActionTargetIvars {
            app_handle: app_handle.clone(),
            menu: menu.retain(),
            status_item: status_item.retain(),
            status_view: status_view.retain(),
            last_event_number: std::cell::Cell::new(-1),
            two_touch_menu_open: std::cell::Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(target_os = "macos")]
fn install_status_button_action_target(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    button: &objc2_app_kit::NSStatusBarButton,
    status_view: &objc2_app_kit::NSView,
) {
    let target = TrayStatusButtonActionTarget::new(app_handle, menu, status_item, status_view);
    let target_object: objc2::rc::Retained<objc2::runtime::AnyObject> = target.into();
    let control: &objc2_app_kit::NSControl = button.as_super().as_super();

    unsafe {
        control.setTarget(Some(&target_object));
        control.setAction(Some(objc2::sel!(handleOpenUsageTrayStatusButtonAction:)));
    }
    control.sendActionOn(status_button_action_event_mask());

    // NSControl's target is weak, so keep the action bridge alive for the app lifetime.
    std::mem::forget(target_object);
    log::debug!("tray context menu: installed status button action target");
}

#[cfg(target_os = "macos")]
fn install_status_view_context_click_gestures(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_item: &objc2_app_kit::NSStatusItem,
    status_view: &objc2_app_kit::NSView,
) {
    let target = TrayStatusButtonActionTarget::new(app_handle, menu, status_item, status_view);
    install_status_view_context_click_gestures_with_target(status_view, &target);
    let target_object: objc2::rc::Retained<objc2::runtime::AnyObject> = target.into();

    // NSGestureRecognizer targets are weak.
    std::mem::forget(target_object);
    log::debug!("tray context menu: installed status view click gestures");
}

#[cfg(target_os = "macos")]
fn install_status_view_context_click_gestures_with_target(
    view: &objc2_app_kit::NSView,
    target: &TrayStatusButtonActionTarget,
) {
    accept_indirect_touch_events(view);
    install_status_view_context_click_gestures_on_view(view, target);

    for subview in view.subviews() {
        install_status_view_context_click_gestures_with_target(&subview, target);
    }
}

#[cfg(target_os = "macos")]
fn install_status_view_context_click_gestures_on_view(
    view: &objc2_app_kit::NSView,
    target: &TrayStatusButtonActionTarget,
) {
    use objc2::ClassType;
    use objc2_app_kit::{NSClickGestureRecognizer, NSGestureRecognizer};

    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot install click gesture off main thread");
        return;
    };
    let target: &objc2::runtime::AnyObject =
        unsafe { &*(target as *const TrayStatusButtonActionTarget).cast() };

    for spec in status_view_context_click_gesture_specs() {
        let click = NSClickGestureRecognizer::new(mtm);
        configure_status_view_context_click_gesture(&click, target, spec);
        let click: &NSGestureRecognizer = click.as_super();
        view.addGestureRecognizer(click);
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatusViewClickGestureSpec {
    button_mask: objc2_foundation::NSUInteger,
    touch_count: objc2_foundation::NSInteger,
    delay_primary: bool,
    delay_secondary: bool,
}

#[cfg(target_os = "macos")]
fn status_view_context_click_gesture_specs() -> [StatusViewClickGestureSpec; 3] {
    [
        StatusViewClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 1,
            delay_primary: false,
            delay_secondary: true,
        },
        StatusViewClickGestureSpec {
            button_mask: primary_click_button_mask(),
            touch_count: 2,
            delay_primary: true,
            delay_secondary: false,
        },
        StatusViewClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 2,
            delay_primary: false,
            delay_secondary: true,
        },
    ]
}

#[cfg(target_os = "macos")]
fn configure_status_view_context_click_gesture(
    recognizer: &objc2_app_kit::NSClickGestureRecognizer,
    target: &objc2::runtime::AnyObject,
    spec: StatusViewClickGestureSpec,
) {
    use objc2::ClassType;
    use objc2_app_kit::NSGestureRecognizer;

    recognizer.setButtonMask(spec.button_mask);
    recognizer.setNumberOfClicksRequired(1);
    recognizer.setNumberOfTouchesRequired(spec.touch_count);

    let gesture: &NSGestureRecognizer = recognizer.as_super();
    unsafe {
        gesture.setTarget(Some(target));
        gesture.setAction(Some(objc2::sel!(
            openOpenUsageTrayContextMenuFromClickGesture:
        )));
    }
    gesture.setDelaysPrimaryMouseButtonEvents(spec.delay_primary);
    gesture.setDelaysSecondaryMouseButtonEvents(spec.delay_secondary);
    gesture.setDelaysOtherMouseButtonEvents(false);
}

#[cfg(target_os = "macos")]
fn primary_click_button_mask() -> objc2_foundation::NSUInteger {
    1 << 0
}

#[cfg(target_os = "macos")]
fn secondary_click_button_mask() -> objc2_foundation::NSUInteger {
    1 << 1
}

#[cfg(target_os = "macos")]
fn status_button_action_event_mask() -> objc2_app_kit::NSEventMask {
    use objc2_app_kit::NSEventMask;

    NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::OtherMouseDown
        | NSEventMask::OtherMouseUp
        | NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_status_button_action_event(event: &objc2_app_kit::NSEvent) -> bool {
    should_open_tray_menu_from_status_button_action_event_details(
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
    )
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_status_button_action_event_details(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSEventType};

    event_type == NSEventType::RightMouseDown
        || event_type == NSEventType::RightMouseUp
        || event_type == NSEventType::OtherMouseDown
        || event_type == NSEventType::OtherMouseUp
        || ((event_type == NSEventType::LeftMouseDown || event_type == NSEventType::LeftMouseUp)
            && (modifier_flags.contains(NSEventModifierFlags::Control) || button_number == 1))
}

#[cfg(target_os = "macos")]
fn should_handle_primary_status_item_click(event: &objc2_app_kit::NSEvent) -> bool {
    should_handle_primary_status_item_click_details(
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
    )
}

#[cfg(target_os = "macos")]
fn should_handle_primary_status_item_click_details(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSEventType};

    (event_type == NSEventType::LeftMouseDown || event_type == NSEventType::LeftMouseUp)
        && button_number == 0
        && !modifier_flags.contains(NSEventModifierFlags::Control)
}

#[cfg(target_os = "macos")]
fn should_cancel_status_item_system_menu_for_primary_click(event: &objc2_app_kit::NSEvent) -> bool {
    should_cancel_status_item_system_menu_for_primary_click_details(
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
    )
}

#[cfg(target_os = "macos")]
fn should_cancel_status_item_system_menu_for_primary_click_details(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
) -> bool {
    use objc2_app_kit::{NSEventModifierFlags, NSEventType};

    (event_type == NSEventType::LeftMouseDown || event_type == NSEventType::LeftMouseUp)
        && button_number == 0
        && !modifier_flags.contains(NSEventModifierFlags::Control)
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrimaryStatusItemClickAction {
    Ignore,
    TogglePanel,
    PassThrough,
}

#[cfg(target_os = "macos")]
fn primary_status_item_click_action(
    event: &objc2_app_kit::NSEvent,
    native_menu_recently_opened: bool,
) -> PrimaryStatusItemClickAction {
    primary_status_item_click_action_details(
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
        native_menu_recently_opened,
    )
}

#[cfg(target_os = "macos")]
fn primary_status_item_click_action_details(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
    native_menu_recently_opened: bool,
) -> PrimaryStatusItemClickAction {
    use objc2_app_kit::NSEventType;

    if !should_handle_primary_status_item_click_details(event_type, modifier_flags, button_number) {
        return PrimaryStatusItemClickAction::PassThrough;
    }

    if native_menu_recently_opened {
        return PrimaryStatusItemClickAction::Ignore;
    }

    if event_type == NSEventType::LeftMouseUp {
        return PrimaryStatusItemClickAction::TogglePanel;
    }

    PrimaryStatusItemClickAction::Ignore
}

#[cfg(target_os = "macos")]
pub(crate) fn update_native_tray_rect_from_view(view: &objc2_app_kit::NSView) {
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
#[derive(Debug)]
struct TrayInputOverlayViewIvars {
    app_handle: AppHandle,
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    suppress_next_mouse_up: std::cell::Cell<bool>,
    two_touch_menu_open: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
objc2::define_class!(
    #[derive(Debug)]
    #[unsafe(super(objc2_app_kit::NSView))]
    #[name = "OpenUsageTrayInputOverlayView"]
    #[ivars = TrayInputOverlayViewIvars]
    struct TrayInputOverlayView;

    impl TrayInputOverlayView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&objc2_app_kit::NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.update_tray_rect();
            self.log_event("mouseDown", event);
            if self.should_open_context_menu_from_event(event) {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &objc2_app_kit::NSEvent) {
            self.update_tray_rect();
            self.log_event("mouseUp", event);
            if self.should_open_context_menu_from_event(event) {
                self.ivars().suppress_next_mouse_up.set(false);
                self.open_context_menu_for_event(event);
                return;
            }

            match overlay_mouse_up_action(self.ivars().suppress_next_mouse_up.replace(false), event)
            {
                TrayOverlayMouseUpAction::Ignore => {}
                TrayOverlayMouseUpAction::OpenMenu => self.open_context_menu_for_event(event),
                TrayOverlayMouseUpAction::TogglePanel => toggle_panel(&self.ivars().app_handle),
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.log_event("rightMouseDown", event);
            self.ivars().suppress_next_mouse_up.set(true);
            self.open_context_menu_for_event(event);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &objc2_app_kit::NSEvent) {
            self.log_event("rightMouseUp", event);
            self.ivars().suppress_next_mouse_up.set(true);
            self.open_context_menu_for_event(event);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.log_event("otherMouseDown", event);
            if event.buttonNumber() > 0 {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &objc2_app_kit::NSEvent) {
            self.log_event("otherMouseUp", event);
            if event.buttonNumber() > 0 {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(swipeWithEvent:))]
        fn swipe_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(rotateWithEvent:))]
        fn rotate_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(smartMagnifyWithEvent:))]
        fn smart_magnify_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(pressureChangeWithEvent:))]
        fn pressure_change_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(quickLookWithEvent:))]
        fn quick_look_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesBeganWithEvent:))]
        fn touches_began_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesMovedWithEvent:))]
        fn touches_moved_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesEndedWithEvent:))]
        fn touches_ended_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesCancelledWithEvent:))]
        fn touches_cancelled_with_event(&self, event: &objc2_app_kit::NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(
            &self,
            event: &objc2_app_kit::NSEvent,
        ) -> Option<&'static objc2_app_kit::NSMenu> {
            self.update_tray_rect();
            self.log_event("menuForEvent", event);
            self.ivars().suppress_next_mouse_up.set(true);
            retained_tray_menu_as_static_ref(&self.ivars().menu)
        }
    }
);

#[cfg(target_os = "macos")]
impl TrayInputOverlayView {
    fn as_view(&self) -> &objc2_app_kit::NSView {
        use objc2::ClassType;

        self.as_super()
    }

    fn update_tray_rect(&self) {
        update_native_tray_rect_from_view(&self.ivars().status_view);
    }

    fn open_context_menu(&self) {
        self.update_tray_rect();
        show_native_tray_menu_at_view(&self.ivars().menu, &self.ivars().status_view);
    }

    fn open_context_menu_for_event(&self, event: &objc2_app_kit::NSEvent) {
        self.update_tray_rect();
        if should_skip_recent_native_menu_open(std::time::Instant::now()) {
            return;
        }

        log::warn!(
            "tray context menu: overlay opening menu event_type={:?} button={}",
            event.r#type(),
            event.buttonNumber()
        );
        show_native_tray_menu_at_view(&self.ivars().menu, &self.ivars().status_view);
    }

    fn handle_touch_event(&self, event: &objc2_app_kit::NSEvent) {
        let touch_count = active_touch_count_for_event(event, self.as_view());
        if should_open_tray_menu_from_touch_count(
            self.ivars().two_touch_menu_open.get(),
            touch_count,
        ) {
            self.ivars().two_touch_menu_open.set(true);
            log::debug!("tray context menu: opening from two-touch overlay event");
            self.open_context_menu();
            return;
        }

        if should_reset_touch_menu_gate(touch_count) {
            self.ivars().two_touch_menu_open.set(false);
        }
    }

    fn should_open_context_menu_from_event(&self, event: &objc2_app_kit::NSEvent) -> bool {
        should_open_tray_menu_from_native_event(event)
            || should_open_tray_menu_from_touch_count(
                self.ivars().two_touch_menu_open.get(),
                active_touch_count_for_event(event, self.as_view()),
            )
    }

    fn log_event(&self, label: &str, event: &objc2_app_kit::NSEvent) {
        let count = TRAY_OVERLAY_EVENT_LOGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count >= MAX_TRAY_OVERLAY_EVENT_LOGS {
            return;
        }

        log::warn!(
            "tray context menu: overlay {label} event_type={:?} button={} touches={} pressed_buttons={}",
            event.r#type(),
            event.buttonNumber(),
            active_touch_count_for_event(event, self.as_view()),
            objc2_app_kit::NSEvent::pressedMouseButtons()
        );
    }
}

#[cfg(target_os = "macos")]
fn install_tray_input_view(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2::ClassType;
    use objc2_app_kit::NSWindowOrderingMode;
    use objc2_foundation::{NSPoint, NSRect};

    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot install input view off main thread");
        return;
    };
    let frame = status_view.bounds();

    let overlay_view = unsafe {
        let view = mtm.alloc().set_ivars(TrayInputOverlayViewIvars {
            app_handle: app_handle.clone(),
            menu: menu.retain(),
            status_view: status_view.retain(),
            suppress_next_mouse_up: std::cell::Cell::new(false),
            two_touch_menu_open: std::cell::Cell::new(false),
        });
        let view: objc2::rc::Retained<TrayInputOverlayView> = msg_send![
            super(view),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), frame.size)
        ];
        view
    };
    let overlay_ns_view: &objc2_app_kit::NSView = overlay_view.as_super();
    accept_indirect_touch_events(overlay_ns_view);
    overlay_ns_view.setAutoresizingMask(
        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    overlay_ns_view.setFrame(status_view.bounds());
    status_view.addSubview_positioned_relativeTo(
        overlay_ns_view,
        NSWindowOrderingMode::Above,
        None,
    );

    std::mem::forget(overlay_view);
    log::warn!("tray context menu: installed embedded input view");
}

#[cfg(target_os = "macos")]
fn install_tray_input_overlay_window(
    app_handle: &AppHandle,
    menu: &objc2_app_kit::NSMenu,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2::ClassType;
    use objc2_app_kit::{NSBackingStoreType, NSColor, NSPanel, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{MainThreadMarker, NSTimer};
    use std::ptr::NonNull;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot install overlay window off main thread");
        return;
    };
    let Some(screen_frame) = status_view_screen_frame(status_view) else {
        log::warn!("tray context menu: status item frame unavailable for overlay window");
        return;
    };
    let screen_top_y = screen_top_y_for_status_frame(screen_frame);
    let overlay_frame = tray_input_overlay_window_frame(screen_frame, screen_top_y);

    let overlay_view = unsafe {
        let view = mtm.alloc().set_ivars(TrayInputOverlayViewIvars {
            app_handle: app_handle.clone(),
            menu: menu.retain(),
            status_view: status_view.retain(),
            suppress_next_mouse_up: std::cell::Cell::new(false),
            two_touch_menu_open: std::cell::Cell::new(false),
        });
        let view: objc2::rc::Retained<TrayInputOverlayView> = msg_send![
            super(view),
            initWithFrame: tray_input_overlay_content_frame(overlay_frame)
        ];
        view
    };
    let overlay_ns_view: &objc2_app_kit::NSView = overlay_view.as_super();
    accept_indirect_touch_events(overlay_ns_view);
    overlay_ns_view.setFrame(tray_input_overlay_content_frame(overlay_frame));

    let window = unsafe {
        let window = NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            overlay_frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window
    };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(false);
    window.setAcceptsMouseMovedEvents(true);
    window.setCanHide(false);
    window.setLevel(tray_input_overlay_window_level());
    window.setCollectionBehavior(tray_input_overlay_collection_behavior());
    window.setContentView(Some(overlay_ns_view));
    window.orderFrontRegardless();

    let timer_window = window.retain();
    let timer_status_view = status_view.retain();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let timer_window: &NSWindow = timer_window.as_super();
        sync_tray_input_overlay_window(timer_window, &timer_status_view);
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSTimer>)> = &block;
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, block_ref) };

    std::mem::forget(timer);
    std::mem::forget(block);
    std::mem::forget(window);
    std::mem::forget(overlay_view);
    log::warn!(
        "tray context menu: installed status-bar input overlay window level={}",
        tray_input_overlay_window_level()
    );
}

#[cfg(target_os = "macos")]
fn should_install_tray_input_overlay_window(installed_custom_status_view: bool) -> bool {
    let _ = installed_custom_status_view;
    true
}

#[cfg(target_os = "macos")]
fn sync_tray_input_overlay_window(
    window: &objc2_app_kit::NSWindow,
    status_view: &objc2_app_kit::NSView,
) {
    let Some(screen_frame) = status_view_screen_frame(status_view) else {
        return;
    };
    let overlay_frame =
        tray_input_overlay_window_frame(screen_frame, screen_top_y_for_status_frame(screen_frame));

    window.setFrame_display(overlay_frame, false);
    if let Some(content_view) = window.contentView() {
        content_view.setFrame(tray_input_overlay_content_frame(overlay_frame));
    }
    if !window.isVisible() {
        window.orderFrontRegardless();
    }
}

#[cfg(target_os = "macos")]
fn screen_top_y_for_status_frame(status_frame: objc2_foundation::NSRect) -> f64 {
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return status_frame.origin.y + status_frame.size.height;
    };

    let screens = objc2_app_kit::NSScreen::screens(mtm);
    for index in 0..screens.count() {
        let screen = screens.objectAtIndex(index);
        let frame = screen.frame();
        if screen_frame_matches_status_frame(frame, status_frame) {
            return frame.origin.y + frame.size.height;
        }
    }

    objc2_app_kit::NSScreen::mainScreen(mtm)
        .map(|screen| screen.frame().origin.y + screen.frame().size.height)
        .unwrap_or(status_frame.origin.y + status_frame.size.height)
}

#[cfg(target_os = "macos")]
fn screen_frame_matches_status_frame(
    screen_frame: objc2_foundation::NSRect,
    status_frame: objc2_foundation::NSRect,
) -> bool {
    point_is_inside_rect(rect_center(status_frame), screen_frame)
        || rects_intersect(screen_frame, status_frame)
}

#[cfg(target_os = "macos")]
fn rect_center(rect: objc2_foundation::NSRect) -> objc2_foundation::NSPoint {
    objc2_foundation::NSPoint::new(
        rect.origin.x + (rect.size.width / 2.0),
        rect.origin.y + (rect.size.height / 2.0),
    )
}

#[cfg(target_os = "macos")]
fn point_is_inside_rect(point: objc2_foundation::NSPoint, rect: objc2_foundation::NSRect) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

#[cfg(target_os = "macos")]
fn rects_intersect(first: objc2_foundation::NSRect, second: objc2_foundation::NSRect) -> bool {
    first.origin.x < second.origin.x + second.size.width
        && first.origin.x + first.size.width > second.origin.x
        && first.origin.y < second.origin.y + second.size.height
        && first.origin.y + first.size.height > second.origin.y
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_window_frame(
    status_frame: objc2_foundation::NSRect,
    screen_top_y: f64,
) -> objc2_foundation::NSRect {
    let status_top_y = status_frame.origin.y + status_frame.size.height;
    let top_y = if screen_top_y >= status_top_y
        && screen_top_y - status_top_y
            <= crate::macos_status_item_icon::STATUS_ITEM_MENU_BAR_HIT_HEIGHT
    {
        screen_top_y
    } else {
        status_top_y
    };
    let height = status_frame
        .size
        .height
        .max(crate::macos_status_item_icon::STATUS_ITEM_MENU_BAR_HIT_HEIGHT);
    let bottom_y = top_y - height;

    objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(
            status_frame.origin.x - STATUS_ITEM_HORIZONTAL_HIT_PADDING,
            bottom_y,
        ),
        objc2_foundation::NSSize::new(
            status_frame.size.width + (STATUS_ITEM_HORIZONTAL_HIT_PADDING * 2.0),
            height,
        ),
    )
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_content_frame(
    window_frame: objc2_foundation::NSRect,
) -> objc2_foundation::NSRect {
    objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), window_frame.size)
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_window_level() -> objc2_app_kit::NSWindowLevel {
    objc2_app_kit::NSScreenSaverWindowLevel + 1
}

#[cfg(target_os = "macos")]
fn tray_input_overlay_collection_behavior() -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior;

    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
        | NSWindowCollectionBehavior::FullScreenAuxiliary
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn accept_indirect_touch_events(view: &objc2_app_kit::NSView) {
    view.setAcceptsTouchEvents(true);
    view.setWantsRestingTouches(true);
    view.setAllowedTouchTypes(objc2_app_kit::NSTouchTypeMask::Indirect);
}

#[cfg(target_os = "macos")]
fn status_item_native_event_monitor_mask() -> objc2_app_kit::NSEventMask {
    use objc2_app_kit::NSEventMask;

    NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::OtherMouseDown
        | NSEventMask::OtherMouseUp
        | NSEventMask::SystemDefined
        | NSEventMask::ScrollWheel
        | NSEventMask::Gesture
        | NSEventMask::Magnify
        | NSEventMask::Swipe
        | NSEventMask::Rotate
        | NSEventMask::BeginGesture
        | NSEventMask::EndGesture
        | NSEventMask::SmartMagnify
        | NSEventMask::Pressure
        | NSEventMask::DirectTouch
}

#[cfg(target_os = "macos")]
fn active_touch_count_for_event(
    event: &objc2_app_kit::NSEvent,
    view: &objc2_app_kit::NSView,
) -> usize {
    let touches_in_view = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, Some(view))
        .count();
    let touches_in_event = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, None)
        .count();
    let any_touches_in_view = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Any, Some(view))
        .count();
    let any_touches_in_event = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Any, None)
        .count();

    touches_in_view
        .max(touches_in_event)
        .max(any_touches_in_view)
        .max(any_touches_in_event)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_touch_count(
    menu_open_for_current_touch: bool,
    touch_count: usize,
) -> bool {
    !menu_open_for_current_touch && touch_count >= 2
}

#[cfg(target_os = "macos")]
fn should_reset_touch_menu_gate(touch_count: usize) -> bool {
    touch_count < 2
}

#[cfg(target_os = "macos")]
fn retained_tray_menu_as_static_ref(
    menu: &objc2::rc::Retained<objc2_app_kit::NSMenu>,
) -> Option<&'static objc2_app_kit::NSMenu> {
    let menu: &objc2_app_kit::NSMenu = menu;
    Some(unsafe { &*(menu as *const objc2_app_kit::NSMenu) })
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayOverlayMouseUpAction {
    Ignore,
    OpenMenu,
    TogglePanel,
}

#[cfg(target_os = "macos")]
fn overlay_mouse_up_action(
    suppress_next_mouse_up: bool,
    event: &objc2_app_kit::NSEvent,
) -> TrayOverlayMouseUpAction {
    overlay_mouse_up_action_details(
        suppress_next_mouse_up,
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
        native_menu_recently_opened_for_mouse_up(std::time::Instant::now()),
    )
}

#[cfg(target_os = "macos")]
fn overlay_mouse_up_action_details(
    suppress_next_mouse_up: bool,
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
    native_menu_recently_opened: bool,
) -> TrayOverlayMouseUpAction {
    if suppress_next_mouse_up {
        return TrayOverlayMouseUpAction::Ignore;
    }
    if should_open_tray_menu_from_native_event_details(event_type, modifier_flags, button_number) {
        return TrayOverlayMouseUpAction::OpenMenu;
    }
    if native_menu_recently_opened {
        return TrayOverlayMouseUpAction::Ignore;
    }

    TrayOverlayMouseUpAction::TogglePanel
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TrayEventTapState {
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    mode: TrayEventTapMode,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayEventTapMode {
    Active,
    ListenOnly,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
struct TraySecondaryClickPollState {
    menu: objc2::rc::Retained<objc2_app_kit::NSMenu>,
    status_view: objc2::rc::Retained<objc2_app_kit::NSView>,
    secondary_button_was_down: std::cell::Cell<bool>,
}

#[cfg(target_os = "macos")]
fn install_secondary_click_poll_timer(
    menu: &objc2_app_kit::NSMenu,
    status_view: &objc2_app_kit::NSView,
) {
    use objc2_foundation::NSTimer;
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(TraySecondaryClickPollState {
        menu: menu.retain(),
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
            show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
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
    status_view: &objc2_app_kit::NSView,
) {
    for (location, mode) in status_item_event_tap_specs() {
        install_status_item_event_tap_at_location(menu, status_view, location, mode);
    }
}

#[cfg(target_os = "macos")]
fn status_item_event_tap_specs() -> [(objc2_core_graphics::CGEventTapLocation, TrayEventTapMode); 5]
{
    use objc2_core_graphics::CGEventTapLocation;

    [
        (CGEventTapLocation::HIDEventTap, TrayEventTapMode::Active),
        (
            CGEventTapLocation::HIDEventTap,
            TrayEventTapMode::ListenOnly,
        ),
        (
            CGEventTapLocation::SessionEventTap,
            TrayEventTapMode::Active,
        ),
        (
            CGEventTapLocation::SessionEventTap,
            TrayEventTapMode::ListenOnly,
        ),
        (
            CGEventTapLocation::AnnotatedSessionEventTap,
            TrayEventTapMode::ListenOnly,
        ),
    ]
}

#[cfg(target_os = "macos")]
fn install_status_item_event_tap_at_location(
    menu: &objc2_app_kit::NSMenu,
    status_view: &objc2_app_kit::NSView,
    location: objc2_core_graphics::CGEventTapLocation,
    mode: TrayEventTapMode,
) {
    use objc2_core_foundation::{CFMachPort, CFRunLoop, kCFRunLoopCommonModes};
    use objc2_core_graphics::{CGEvent, CGEventTapPlacement, CGEventType};

    let state = Box::new(TrayEventTapState {
        menu: menu.retain(),
        status_view: status_view.retain(),
        mode,
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
            event_tap_options_for_mode(mode),
            event_mask,
            Some(status_item_event_tap_callback),
            state_ptr.cast(),
        )
    }) else {
        let _ = unsafe { Box::from_raw(state_ptr) };
        maybe_request_input_monitoring_for_event_tap(mode);
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
            "tray context menu: installed CoreGraphics {:?} {:?} event tap",
            location,
            mode
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
    if should_open_tray_menu_from_cg_event_for_tap_mode(state.mode, event_type, cg_event)
        && is_mouse_inside_status_view(&state.status_view)
    {
        log::debug!(
            "tray context menu: CoreGraphics event tap open event_type={:?}",
            event_type
        );
        show_native_tray_menu_at_view_on_main(&state.menu, &state.status_view);
        if should_swallow_opened_cg_event(state.mode, event_type, cg_event) {
            return std::ptr::null_mut();
        }
    }

    event.as_ptr()
}

#[cfg(target_os = "macos")]
fn event_tap_options_for_mode(mode: TrayEventTapMode) -> objc2_core_graphics::CGEventTapOptions {
    match mode {
        TrayEventTapMode::Active => objc2_core_graphics::CGEventTapOptions::Default,
        TrayEventTapMode::ListenOnly => objc2_core_graphics::CGEventTapOptions::ListenOnly,
    }
}

#[cfg(target_os = "macos")]
fn maybe_request_input_monitoring_for_event_tap(mode: TrayEventTapMode) {
    if !should_request_input_monitoring_for_event_tap_failure(mode) {
        return;
    }

    request_input_monitoring_access_once();
}

#[cfg(target_os = "macos")]
fn should_request_input_monitoring_for_event_tap_failure(mode: TrayEventTapMode) -> bool {
    mode == TrayEventTapMode::Active
}

#[cfg(target_os = "macos")]
fn request_input_monitoring_access_once() {
    static REQUEST_INPUT_MONITORING_ACCESS: std::sync::Once = std::sync::Once::new();

    REQUEST_INPUT_MONITORING_ACCESS.call_once(|| {
        if objc2_core_graphics::CGPreflightListenEventAccess() {
            log::debug!("tray context menu: Input Monitoring already granted");
            return;
        }

        log::warn!(
            "tray context menu: requesting Input Monitoring permission for macOS context-click fallback"
        );
        if objc2_core_graphics::CGRequestListenEventAccess() {
            log::warn!("tray context menu: Input Monitoring permission granted");
        } else {
            log::warn!(
                "tray context menu: Input Monitoring still disabled; enable OpenUsage in System Settings > Privacy & Security > Input Monitoring, then relaunch"
            );
        }
    });
}

#[cfg(target_os = "macos")]
fn should_swallow_opened_cg_event(
    mode: TrayEventTapMode,
    event_type: objc2_core_graphics::CGEventType,
    event: &objc2_core_graphics::CGEvent,
) -> bool {
    mode == TrayEventTapMode::Active && should_open_tray_menu_from_cg_event(event_type, event)
}

#[cfg(target_os = "macos")]
#[cfg(test)]
fn should_swallow_opened_cg_event_details(
    mode: TrayEventTapMode,
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
    button_number: i64,
) -> bool {
    mode == TrayEventTapMode::Active
        && should_open_tray_menu_from_cg_event_details(event_type, flags, button_number)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event(
    event_type: objc2_core_graphics::CGEventType,
    event: &objc2_core_graphics::CGEvent,
) -> bool {
    use objc2_core_graphics::{CGEvent, CGEventField};

    should_open_tray_menu_from_cg_event_details(
        event_type,
        CGEvent::flags(Some(event)),
        CGEvent::integer_value_field(Some(event), CGEventField::MouseEventButtonNumber),
    )
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event_for_tap_mode(
    mode: TrayEventTapMode,
    event_type: objc2_core_graphics::CGEventType,
    event: &objc2_core_graphics::CGEvent,
) -> bool {
    use objc2_core_graphics::{CGEvent, CGEventField};

    should_open_tray_menu_from_cg_event_details_for_tap_mode(
        mode,
        event_type,
        CGEvent::flags(Some(event)),
        CGEvent::integer_value_field(Some(event), CGEventField::MouseEventButtonNumber),
    )
}

#[cfg(target_os = "macos")]
#[cfg(test)]
fn should_open_tray_menu_from_cg_event_type(
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
) -> bool {
    should_open_tray_menu_from_cg_event_details(event_type, flags, 0)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event_details(
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
    button_number: i64,
) -> bool {
    use objc2_core_graphics::{CGEventFlags, CGEventType};

    event_type == CGEventType::RightMouseDown
        || event_type == CGEventType::RightMouseUp
        || event_type == CGEventType::OtherMouseDown
        || event_type == CGEventType::OtherMouseUp
        || ((event_type == CGEventType::LeftMouseDown || event_type == CGEventType::LeftMouseUp)
            && (flags.contains(CGEventFlags::MaskControl) || button_number == 1))
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_cg_event_details_for_tap_mode(
    mode: TrayEventTapMode,
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
    button_number: i64,
) -> bool {
    if mode == TrayEventTapMode::Active {
        return should_open_tray_menu_from_cg_event_details(event_type, flags, button_number);
    }

    should_open_tray_menu_from_passive_cg_event_details(event_type, flags, button_number)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_passive_cg_event_details(
    event_type: objc2_core_graphics::CGEventType,
    flags: objc2_core_graphics::CGEventFlags,
    button_number: i64,
) -> bool {
    use objc2_core_graphics::{CGEventFlags, CGEventType};

    event_type == CGEventType::RightMouseUp
        || event_type == CGEventType::OtherMouseUp
        || (event_type == CGEventType::LeftMouseUp
            && (flags.contains(CGEventFlags::MaskControl) || button_number == 1))
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
pub(crate) fn show_native_tray_menu(
    status_item: &objc2_app_kit::NSStatusItem,
    menu: &objc2_app_kit::NSMenu,
) {
    if should_skip_recent_native_menu_open(std::time::Instant::now()) {
        return;
    }

    show_native_tray_menu_without_recent_guard(status_item, menu);
}

#[cfg(target_os = "macos")]
pub(crate) fn show_native_tray_menu_at_view(
    menu: &objc2_app_kit::NSMenu,
    view: &objc2_app_kit::NSView,
) {
    if should_skip_recent_native_menu_open(std::time::Instant::now()) {
        return;
    }

    log::debug!("tray context menu: showing native menu at status view");
    update_native_tray_rect_from_view(view);
    pop_up_native_tray_menu_at_view(menu, view);
}

#[cfg(target_os = "macos")]
fn show_native_tray_menu_at_view_on_main(
    menu: &objc2_app_kit::NSMenu,
    view: &objc2_app_kit::NSView,
) {
    if objc2_foundation::MainThreadMarker::new().is_some() {
        show_native_tray_menu_at_view(menu, view);
        return;
    }

    let menu = menu.retain();
    let view = view.retain();
    let block = block2::RcBlock::new(move || {
        show_native_tray_menu_at_view(&menu, &view);
    });
    let block_ref: &block2::DynBlock<dyn Fn()> = &block;
    unsafe {
        objc2_foundation::NSRunLoop::mainRunLoop().performBlock(block_ref);
    }
    if let Some(run_loop) = objc2_core_foundation::CFRunLoop::main() {
        run_loop.wake_up();
    }
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub(crate) fn show_native_tray_menu_for_event(
    menu: &objc2_app_kit::NSMenu,
    event: &objc2_app_kit::NSEvent,
    view: &objc2_app_kit::NSView,
) {
    if should_skip_recent_native_menu_open(std::time::Instant::now()) {
        return;
    }

    log::debug!("tray context menu: showing native context menu for event");
    update_native_tray_rect_from_view(view);
    objc2_app_kit::NSMenu::popUpContextMenu_withEvent_forView(menu, event, view);
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
    let Ok(mut last_opened_at) = native_menu_last_opened_at().lock() else {
        return false;
    };

    if instant_is_within_window(*last_opened_at, now, std::time::Duration::from_millis(200)) {
        return true;
    }

    *last_opened_at = Some(now);
    false
}

#[cfg(target_os = "macos")]
fn mark_native_menu_opened(now: std::time::Instant) {
    let Ok(mut last_opened_at) = native_menu_last_opened_at().lock() else {
        return;
    };

    *last_opened_at = Some(now);
}

#[cfg(target_os = "macos")]
fn native_menu_recently_opened_for_mouse_up(now: std::time::Instant) -> bool {
    let Ok(last_opened_at) = native_menu_last_opened_at().lock() else {
        return false;
    };

    instant_is_within_window(*last_opened_at, now, std::time::Duration::from_millis(700))
}

#[cfg(target_os = "macos")]
fn native_menu_last_opened_at() -> &'static std::sync::Mutex<Option<std::time::Instant>> {
    static LAST_OPENED_AT: std::sync::Mutex<Option<std::time::Instant>> =
        std::sync::Mutex::new(None);

    &LAST_OPENED_AT
}

#[cfg(target_os = "macos")]
fn instant_is_within_window(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    window: std::time::Duration,
) -> bool {
    last.and_then(|last| now.checked_duration_since(last))
        .is_some_and(|elapsed| elapsed < window)
}

#[cfg(target_os = "macos")]
pub(crate) fn is_mouse_inside_status_view(view: &objc2_app_kit::NSView) -> bool {
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
pub(crate) fn should_open_tray_menu_from_native_event(event: &objc2_app_kit::NSEvent) -> bool {
    should_open_tray_menu_from_native_event_details(
        event.r#type(),
        event.modifierFlags(),
        event.buttonNumber(),
    ) || should_open_tray_menu_from_trackpad_event_type(
        event.r#type(),
        objc2_app_kit::NSEvent::pressedMouseButtons() as usize,
    )
}

#[cfg(target_os = "macos")]
#[cfg(test)]
fn should_open_tray_menu_from_native_event_type(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
) -> bool {
    should_open_tray_menu_from_native_event_details(event_type, modifier_flags, 0)
}

#[cfg(target_os = "macos")]
fn should_open_tray_menu_from_native_event_details(
    event_type: objc2_app_kit::NSEventType,
    modifier_flags: objc2_app_kit::NSEventModifierFlags,
    button_number: isize,
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
            && (modifier_flags.contains(NSEventModifierFlags::Control) || button_number == 1))
        || (event_type == NSEventType::LeftMouseUp
            && (modifier_flags.contains(NSEventModifierFlags::Control) || button_number == 1))
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
        assert!(should_open_tray_menu_from_native_event_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            1
        ));
        assert!(should_open_tray_menu_from_native_event_details(
            NSEventType::LeftMouseUp,
            NSEventModifierFlags::empty(),
            1
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
    fn status_button_action_mask_includes_secondary_mouse_events() {
        use objc2_app_kit::NSEventMask;

        let mask = status_button_action_event_mask();

        assert!(mask.contains(NSEventMask::RightMouseDown));
        assert!(mask.contains(NSEventMask::RightMouseUp));
        assert!(mask.contains(NSEventMask::OtherMouseDown));
        assert!(mask.contains(NSEventMask::OtherMouseUp));
        assert!(mask.contains(NSEventMask::LeftMouseDown));
        assert!(mask.contains(NSEventMask::LeftMouseUp));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_event_monitor_mask_includes_trackpad_gesture_events() {
        use objc2_app_kit::NSEventMask;

        let mask = status_item_native_event_monitor_mask();

        assert!(mask.contains(NSEventMask::ScrollWheel));
        assert!(mask.contains(NSEventMask::Gesture));
        assert!(mask.contains(NSEventMask::Magnify));
        assert!(mask.contains(NSEventMask::Swipe));
        assert!(mask.contains(NSEventMask::Rotate));
        assert!(mask.contains(NSEventMask::SmartMagnify));
        assert!(mask.contains(NSEventMask::Pressure));
        assert!(mask.contains(NSEventMask::DirectTouch));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_view_click_gesture_masks_match_mouse_buttons() {
        assert_eq!(primary_click_button_mask(), 1 << 0);
        assert_eq!(secondary_click_button_mask(), 1 << 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_view_click_gestures_include_two_touch_secondary_click() {
        let specs = status_view_context_click_gesture_specs();

        assert!(specs.contains(&StatusViewClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 1,
            delay_primary: false,
            delay_secondary: true,
        }));
        assert!(specs.contains(&StatusViewClickGestureSpec {
            button_mask: primary_click_button_mask(),
            touch_count: 2,
            delay_primary: true,
            delay_secondary: false,
        }));
        assert!(specs.contains(&StatusViewClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 2,
            delay_primary: false,
            delay_secondary: true,
        }));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_button_action_opens_for_secondary_left_mouse_shape() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert!(
            should_open_tray_menu_from_status_button_action_event_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::empty(),
                1
            )
        );
        assert!(
            should_open_tray_menu_from_status_button_action_event_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::Control,
                0
            )
        );
        assert!(
            !should_open_tray_menu_from_status_button_action_event_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::empty(),
                0
            )
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn primary_status_item_click_intercept_keeps_context_clicks_available() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert!(should_handle_primary_status_item_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            0
        ));
        assert!(should_handle_primary_status_item_click_details(
            NSEventType::LeftMouseUp,
            NSEventModifierFlags::empty(),
            0
        ));
        assert!(!should_handle_primary_status_item_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::Control,
            0
        ));
        assert!(!should_handle_primary_status_item_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_item_system_menu_cancel_only_for_plain_primary_clicks() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert!(
            should_cancel_status_item_system_menu_for_primary_click_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::empty(),
                0
            )
        );
        assert!(
            should_cancel_status_item_system_menu_for_primary_click_details(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0
            )
        );
        assert!(
            !should_cancel_status_item_system_menu_for_primary_click_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::Control,
                0
            )
        );
        assert!(
            !should_cancel_status_item_system_menu_for_primary_click_details(
                NSEventType::RightMouseDown,
                NSEventModifierFlags::empty(),
                1
            )
        );
        assert!(
            !should_cancel_status_item_system_menu_for_primary_click_details(
                NSEventType::OtherMouseDown,
                NSEventModifierFlags::empty(),
                2
            )
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn primary_status_item_click_action_suppresses_mouse_up_after_menu() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert_eq!(
            primary_status_item_click_action_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::empty(),
                0,
                false
            ),
            PrimaryStatusItemClickAction::Ignore
        );
        assert_eq!(
            primary_status_item_click_action_details(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0,
                false
            ),
            PrimaryStatusItemClickAction::TogglePanel
        );
        assert_eq!(
            primary_status_item_click_action_details(
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0,
                true
            ),
            PrimaryStatusItemClickAction::Ignore
        );
        assert_eq!(
            primary_status_item_click_action_details(
                NSEventType::LeftMouseDown,
                NSEventModifierFlags::Control,
                0,
                false
            ),
            PrimaryStatusItemClickAction::PassThrough
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn local_monitor_checks_two_touch_before_primary_click() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert!(should_open_tray_menu_from_touch_count(false, 2));
        assert!(should_handle_primary_status_item_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            0
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
    fn tray_input_overlay_touch_gate_opens_once_per_two_touch_sequence() {
        assert!(should_open_tray_menu_from_touch_count(false, 2));
        assert!(should_open_tray_menu_from_touch_count(false, 3));
        assert!(!should_open_tray_menu_from_touch_count(false, 1));
        assert!(!should_open_tray_menu_from_touch_count(true, 2));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_touch_gate_resets_below_two_touches() {
        assert!(should_reset_touch_menu_gate(0));
        assert!(should_reset_touch_menu_gate(1));
        assert!(!should_reset_touch_menu_gate(2));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_mouse_up_routes_context_click_shapes() {
        use objc2_app_kit::{NSEventModifierFlags, NSEventType};

        assert_eq!(
            overlay_mouse_up_action_details(
                false,
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0,
                false
            ),
            TrayOverlayMouseUpAction::TogglePanel
        );
        assert_eq!(
            overlay_mouse_up_action_details(
                false,
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0,
                true
            ),
            TrayOverlayMouseUpAction::Ignore
        );
        assert_eq!(
            overlay_mouse_up_action_details(
                false,
                NSEventType::RightMouseUp,
                NSEventModifierFlags::empty(),
                1,
                false
            ),
            TrayOverlayMouseUpAction::OpenMenu
        );
        assert_eq!(
            overlay_mouse_up_action_details(
                false,
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                1,
                false
            ),
            TrayOverlayMouseUpAction::OpenMenu
        );
        assert_eq!(
            overlay_mouse_up_action_details(
                true,
                NSEventType::LeftMouseUp,
                NSEventModifierFlags::empty(),
                0,
                false
            ),
            TrayOverlayMouseUpAction::Ignore
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_content_matches_status_item_size() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let status_frame = NSRect::new(NSPoint::new(100.0, 978.0), NSSize::new(24.0, 22.0));
        let window_frame = tray_input_overlay_window_frame(status_frame, 1000.0);
        let content_frame = tray_input_overlay_content_frame(window_frame);

        assert_eq!(content_frame.origin.x, 0.0);
        assert_eq!(content_frame.origin.y, 0.0);
        assert_eq!(content_frame.size.width, window_frame.size.width);
        assert_eq!(content_frame.size.height, window_frame.size.height);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_window_expands_to_tahoe_menu_bar_top_edge() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let status_frame = NSRect::new(NSPoint::new(100.0, 972.0), NSSize::new(24.0, 22.0));
        let overlay_frame = tray_input_overlay_window_frame(status_frame, 1000.0);

        assert_eq!(overlay_frame.origin.x, 97.0);
        assert_eq!(overlay_frame.origin.y, 970.0);
        assert_eq!(overlay_frame.size.width, 30.0);
        assert_eq!(overlay_frame.size.height, 30.0);
        assert_eq!(overlay_frame.origin.y + overlay_frame.size.height, 1000.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_window_sits_above_status_item_level() {
        assert!(tray_input_overlay_window_level() > objc2_app_kit::NSStatusWindowLevel);
        assert!(tray_input_overlay_window_level() > objc2_app_kit::NSPopUpMenuWindowLevel);
        assert_eq!(
            tray_input_overlay_window_level(),
            objc2_app_kit::NSScreenSaverWindowLevel + 1
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn tray_input_overlay_window_is_used_for_custom_status_view() {
        assert!(should_install_tray_input_overlay_window(true));
        assert!(should_install_tray_input_overlay_window(false));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn status_frame_matches_screen_that_contains_status_center() {
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        let primary_screen = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0));
        let status_screen = NSRect::new(NSPoint::new(1440.0, 200.0), NSSize::new(1280.0, 800.0));
        let status_frame = NSRect::new(NSPoint::new(1600.0, 972.0), NSSize::new(24.0, 22.0));

        assert!(!screen_frame_matches_status_frame(
            primary_screen,
            status_frame
        ));
        assert!(screen_frame_matches_status_frame(
            status_screen,
            status_frame
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn recent_native_menu_open_window_expires() {
        let now = std::time::Instant::now();

        assert!(instant_is_within_window(
            Some(now - std::time::Duration::from_millis(100)),
            now,
            std::time::Duration::from_millis(700)
        ));
        assert!(!instant_is_within_window(
            Some(now - std::time::Duration::from_millis(700)),
            now,
            std::time::Duration::from_millis(700)
        ));
        assert!(!instant_is_within_window(
            None,
            now,
            std::time::Duration::from_millis(700)
        ));
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
    fn core_graphics_status_item_taps_use_active_session_fallback() {
        use objc2_core_graphics::{CGEventTapLocation, CGEventTapOptions};

        assert_eq!(
            event_tap_options_for_mode(TrayEventTapMode::Active),
            CGEventTapOptions::Default
        );
        assert_eq!(
            event_tap_options_for_mode(TrayEventTapMode::ListenOnly),
            CGEventTapOptions::ListenOnly
        );

        let specs = status_item_event_tap_specs();
        assert!(specs.contains(&(CGEventTapLocation::HIDEventTap, TrayEventTapMode::Active)));
        assert!(specs.contains(&(
            CGEventTapLocation::HIDEventTap,
            TrayEventTapMode::ListenOnly
        )));
        assert!(specs.contains(&(
            CGEventTapLocation::SessionEventTap,
            TrayEventTapMode::Active
        )));
        assert!(specs.contains(&(
            CGEventTapLocation::SessionEventTap,
            TrayEventTapMode::ListenOnly
        )));
        assert!(specs.contains(&(
            CGEventTapLocation::AnnotatedSessionEventTap,
            TrayEventTapMode::ListenOnly
        )));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn active_core_graphics_tap_failure_requests_input_monitoring() {
        assert!(should_request_input_monitoring_for_event_tap_failure(
            TrayEventTapMode::Active
        ));
        assert!(!should_request_input_monitoring_for_event_tap_failure(
            TrayEventTapMode::ListenOnly
        ));
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
        assert!(should_open_tray_menu_from_cg_event_details(
            CGEventType::LeftMouseDown,
            CGEventFlags::empty(),
            1
        ));
        assert!(should_open_tray_menu_from_cg_event_details(
            CGEventType::LeftMouseUp,
            CGEventFlags::empty(),
            1
        ));
        assert!(!should_open_tray_menu_from_cg_event_type(
            CGEventType::LeftMouseUp,
            CGEventFlags::empty()
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn listen_only_core_graphics_taps_open_on_mouse_up_edge() {
        use objc2_core_graphics::{CGEventFlags, CGEventType};

        assert!(should_open_tray_menu_from_cg_event_details_for_tap_mode(
            TrayEventTapMode::Active,
            CGEventType::RightMouseDown,
            CGEventFlags::empty(),
            1
        ));
        assert!(!should_open_tray_menu_from_cg_event_details_for_tap_mode(
            TrayEventTapMode::ListenOnly,
            CGEventType::RightMouseDown,
            CGEventFlags::empty(),
            1
        ));
        assert!(should_open_tray_menu_from_cg_event_details_for_tap_mode(
            TrayEventTapMode::ListenOnly,
            CGEventType::RightMouseUp,
            CGEventFlags::empty(),
            1
        ));
        assert!(should_open_tray_menu_from_cg_event_details_for_tap_mode(
            TrayEventTapMode::ListenOnly,
            CGEventType::LeftMouseUp,
            CGEventFlags::empty(),
            1
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn active_core_graphics_tap_swallows_only_opened_context_events() {
        use objc2_core_graphics::{CGEventFlags, CGEventType};

        assert!(should_swallow_opened_cg_event_details(
            TrayEventTapMode::Active,
            CGEventType::RightMouseDown,
            CGEventFlags::empty(),
            1
        ));
        assert!(should_swallow_opened_cg_event_details(
            TrayEventTapMode::Active,
            CGEventType::LeftMouseDown,
            CGEventFlags::MaskControl,
            0
        ));
        assert!(!should_swallow_opened_cg_event_details(
            TrayEventTapMode::ListenOnly,
            CGEventType::RightMouseDown,
            CGEventFlags::empty(),
            1
        ));
        assert!(!should_swallow_opened_cg_event_details(
            TrayEventTapMode::Active,
            CGEventType::LeftMouseDown,
            CGEventFlags::empty(),
            0
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
}
