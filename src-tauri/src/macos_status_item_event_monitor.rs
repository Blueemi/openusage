use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use objc2::Message;
use objc2_app_kit::{NSEvent, NSEventMask, NSMenu, NSView};
use objc2_foundation::MainThreadMarker;

const MAX_EVENT_MONITOR_LOGS: usize = 16;
static EVENT_MONITOR_LOGS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct StatusItemEventMonitorState {
    menu: objc2::rc::Retained<NSMenu>,
    status_view: objc2::rc::Retained<NSView>,
}

pub(crate) fn install(menu: &NSMenu, status_view: &NSView) {
    if MainThreadMarker::new().is_none() {
        log::warn!("tray context menu: cannot install AppKit event monitors off main thread");
        return;
    }

    install_local_monitor(menu, status_view);
    install_global_monitor(menu, status_view);
}

fn install_local_monitor(menu: &NSMenu, status_view: &NSView) {
    let state = std::rc::Rc::new(StatusItemEventMonitorState {
        menu: menu.retain(),
        status_view: status_view.retain(),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| -> *mut NSEvent {
        let event = unsafe { event_ptr.as_ref() };
        if should_open_from_monitor_event(event, &block_state.status_view) {
            log_monitor_event("local", event);
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
            return std::ptr::null_mut();
        }

        event_ptr.as_ptr()
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent> = &block;
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            status_item_event_monitor_mask(),
            block_ref,
        )
    };
    let Some(monitor) = monitor else {
        log::warn!("tray context menu: AppKit local event monitor unavailable");
        return;
    };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(monitor);
    log::debug!("tray context menu: installed AppKit local event monitor");
}

fn install_global_monitor(menu: &NSMenu, status_view: &NSView) {
    let state = std::rc::Rc::new(StatusItemEventMonitorState {
        menu: menu.retain(),
        status_view: status_view.retain(),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |event_ptr: NonNull<NSEvent>| {
        let event = unsafe { event_ptr.as_ref() };
        if should_open_from_monitor_event(event, &block_state.status_view) {
            log_monitor_event("global", event);
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
        }
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSEvent>)> = &block;
    let Some(monitor) = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
        status_item_event_monitor_mask(),
        block_ref,
    ) else {
        log::warn!("tray context menu: AppKit global event monitor unavailable");
        return;
    };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(monitor);
    log::debug!("tray context menu: installed AppKit global event monitor");
}

fn should_open_from_monitor_event(event: &NSEvent, status_view: &NSView) -> bool {
    let mouse_inside = crate::tray::is_mouse_inside_status_view(status_view);
    let native_open = crate::tray::should_open_tray_menu_from_native_event(event);
    let touch_count = touch_count_for_event(event, status_view);
    let should_open =
        should_open_from_monitor_event_details(native_open, touch_count, mouse_inside);

    log_monitor_candidate(event, touch_count, mouse_inside, native_open, should_open);

    should_open
}

fn should_open_from_monitor_event_details(
    native_open: bool,
    touch_count: usize,
    mouse_inside: bool,
) -> bool {
    mouse_inside && (native_open || touch_count >= 2)
}

fn touch_count_for_event(event: &NSEvent, status_view: &NSView) -> usize {
    if !event_type_can_report_touches(event.r#type()) {
        return 0;
    }

    let touches_in_view = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, Some(status_view))
        .count();
    let touches_in_event = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, None)
        .count();
    let any_touches_in_view = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Any, Some(status_view))
        .count();
    let any_touches_in_event = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Any, None)
        .count();

    touches_in_view
        .max(touches_in_event)
        .max(any_touches_in_view)
        .max(any_touches_in_event)
}

fn event_type_can_report_touches(event_type: objc2_app_kit::NSEventType) -> bool {
    use objc2_app_kit::NSEventType;

    event_type == NSEventType::Gesture
        || event_type == NSEventType::Magnify
        || event_type == NSEventType::Swipe
        || event_type == NSEventType::Rotate
        || event_type == NSEventType::BeginGesture
        || event_type == NSEventType::EndGesture
        || event_type == NSEventType::SmartMagnify
        || event_type == NSEventType::Pressure
        || event_type == NSEventType::DirectTouch
        || event_type == NSEventType::ScrollWheel
}

fn log_monitor_candidate(
    event: &NSEvent,
    touch_count: usize,
    mouse_inside: bool,
    native_open: bool,
    should_open: bool,
) {
    if !mouse_inside && touch_count < 2 {
        return;
    }

    let count = EVENT_MONITOR_LOGS.fetch_add(1, Ordering::Relaxed);
    if count >= MAX_EVENT_MONITOR_LOGS {
        return;
    }

    log::warn!(
        "tray context menu: AppKit monitor candidate event_type={:?} subtype={:?} button={} pressed_buttons={} touches={} inside={} native_open={} should_open={}",
        event.r#type(),
        event.subtype(),
        event.buttonNumber(),
        NSEvent::pressedMouseButtons(),
        touch_count,
        mouse_inside,
        native_open,
        should_open
    );
}

fn log_monitor_event(kind: &str, event: &NSEvent) {
    let count = EVENT_MONITOR_LOGS.fetch_add(1, Ordering::Relaxed);
    if count >= MAX_EVENT_MONITOR_LOGS {
        return;
    }

    log::warn!(
        "tray context menu: AppKit {kind} monitor open event_type={:?} subtype={:?} button={} pressed_buttons={}",
        event.r#type(),
        event.subtype(),
        event.buttonNumber(),
        NSEvent::pressedMouseButtons()
    );
}

fn status_item_event_monitor_mask() -> NSEventMask {
    NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseUp
        | NSEventMask::OtherMouseDown
        | NSEventMask::OtherMouseUp
        | NSEventMask::SystemDefined
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appkit_monitor_mask_includes_secondary_click_events() {
        let mask = status_item_event_monitor_mask();

        assert!(mask.contains(NSEventMask::RightMouseDown));
        assert!(mask.contains(NSEventMask::RightMouseUp));
        assert!(mask.contains(NSEventMask::OtherMouseDown));
        assert!(mask.contains(NSEventMask::OtherMouseUp));
        assert!(mask.contains(NSEventMask::LeftMouseDown));
        assert!(mask.contains(NSEventMask::LeftMouseUp));
    }

    #[test]
    fn appkit_monitor_mask_includes_trackpad_events() {
        let mask = status_item_event_monitor_mask();

        assert!(mask.contains(NSEventMask::SystemDefined));
        assert!(mask.contains(NSEventMask::Gesture));
        assert!(mask.contains(NSEventMask::BeginGesture));
        assert!(mask.contains(NSEventMask::EndGesture));
        assert!(mask.contains(NSEventMask::Pressure));
        assert!(mask.contains(NSEventMask::DirectTouch));
    }

    #[test]
    fn appkit_monitor_opens_for_two_touch_inside_event() {
        assert!(should_open_from_monitor_event_details(false, 2, true));
        assert!(!should_open_from_monitor_event_details(false, 2, false));
        assert!(!should_open_from_monitor_event_details(false, 1, true));
        assert!(should_open_from_monitor_event_details(true, 0, true));
    }

    #[test]
    fn touch_count_queries_only_run_for_touch_capable_events() {
        use objc2_app_kit::NSEventType;

        assert!(!event_type_can_report_touches(NSEventType::LeftMouseDown));
        assert!(!event_type_can_report_touches(NSEventType::RightMouseDown));
        assert!(event_type_can_report_touches(NSEventType::Gesture));
        assert!(event_type_can_report_touches(NSEventType::DirectTouch));
    }
}
