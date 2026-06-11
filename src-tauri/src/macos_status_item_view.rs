use crate::macos_status_item_icon::{STATUS_ITEM_HEIGHT, STATUS_ITEM_WIDTH, initial_image_frame};
use objc2::{ClassType, DeclaredClass, Message, msg_send};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSEventType, NSImage, NSImageScaling, NSImageView, NSMenu,
    NSStatusItem, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use tauri::AppHandle;

#[derive(Debug)]
struct OpenUsageStatusItemViewIvars {
    app_handle: AppHandle,
    menu: objc2::rc::Retained<NSMenu>,
    suppress_next_mouse_up: std::cell::Cell<bool>,
    two_touch_menu_open: std::cell::Cell<bool>,
}

objc2::define_class!(
    #[derive(Debug)]
    #[unsafe(super(NSView))]
    #[name = "OpenUsageStatusItemView"]
    #[ivars = OpenUsageStatusItemViewIvars]
    struct OpenUsageStatusItemView;

    impl OpenUsageStatusItemView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<&'static NSView> {
            point_is_inside_size(point, self.as_view().bounds().size)
                .then(|| unsafe { &*(self.as_view() as *const NSView) })
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.update_tray_rect();
            if self.should_open_context_menu_from_event(event) {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.update_tray_rect();
            if self.ivars().suppress_next_mouse_up.replace(false) {
                return;
            }

            if self.should_open_context_menu_from_event(event) {
                self.open_context_menu_for_event(event);
                return;
            }

            crate::panel::toggle_panel(&self.ivars().app_handle);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.ivars().suppress_next_mouse_up.set(true);
            self.open_context_menu_for_event(event);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.ivars().suppress_next_mouse_up.set(true);
            self.open_context_menu_for_event(event);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            if event.buttonNumber() > 0 {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            if event.buttonNumber() > 0 {
                self.ivars().suppress_next_mouse_up.set(true);
                self.open_context_menu_for_event(event);
            }
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(swipeWithEvent:))]
        fn swipe_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(rotateWithEvent:))]
        fn rotate_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(smartMagnifyWithEvent:))]
        fn smart_magnify_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(pressureChangeWithEvent:))]
        fn pressure_change_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(quickLookWithEvent:))]
        fn quick_look_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesBeganWithEvent:))]
        fn touches_began_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesMovedWithEvent:))]
        fn touches_moved_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesEndedWithEvent:))]
        fn touches_ended_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(touchesCancelledWithEvent:))]
        fn touches_cancelled_with_event(&self, event: &NSEvent) {
            self.handle_touch_event(event);
        }

        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(&self, event: &NSEvent) -> Option<&'static NSMenu> {
            if self.should_open_context_menu_from_event(event) {
                self.update_tray_rect();
                Some(unsafe {
                    &*(objc2::rc::Retained::as_ptr(&self.ivars().menu) as *const NSMenu)
                })
            } else {
                None
            }
        }
    }
);

impl OpenUsageStatusItemView {
    fn as_view(&self) -> &NSView {
        self.as_super()
    }

    fn update_tray_rect(&self) {
        crate::tray::update_native_tray_rect_from_view(self.as_view());
    }

    fn open_context_menu_for_event(&self, event: &NSEvent) {
        self.update_tray_rect();
        crate::tray::show_native_tray_menu_for_event(&self.ivars().menu, event, self.as_view());
    }

    fn handle_touch_event(&self, event: &NSEvent) {
        let touch_count = active_touch_count_for_event(event, self.as_view());
        if should_open_from_touch_count(self.ivars().two_touch_menu_open.get(), touch_count) {
            self.ivars().two_touch_menu_open.set(true);
            self.ivars().suppress_next_mouse_up.set(true);
            log::debug!("tray context menu: custom status view two-touch event");
            self.update_tray_rect();
            crate::tray::show_native_tray_menu_at_view(&self.ivars().menu, self.as_view());
            return;
        }

        if should_reset_touch_gate(touch_count) {
            self.ivars().two_touch_menu_open.set(false);
        }
    }

    fn should_open_context_menu_from_event(&self, event: &NSEvent) -> bool {
        event_is_context_click(event)
            || should_open_from_touch_count(
                self.ivars().two_touch_menu_open.get(),
                active_touch_count_for_event(event, self.as_view()),
            )
    }
}

#[allow(deprecated)]
pub(crate) fn install(
    app_handle: &AppHandle,
    menu: &NSMenu,
    status_item: &NSStatusItem,
    image: Option<&NSImage>,
    status_size: NSSize,
) -> Option<objc2::rc::Retained<NSView>> {
    let mtm = MainThreadMarker::new()?;
    let size = normalized_status_item_size(status_size);
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);

    let status_view = unsafe {
        let view = mtm.alloc().set_ivars(OpenUsageStatusItemViewIvars {
            app_handle: app_handle.clone(),
            menu: menu.retain(),
            suppress_next_mouse_up: std::cell::Cell::new(false),
            two_touch_menu_open: std::cell::Cell::new(false),
        });
        let view: objc2::rc::Retained<OpenUsageStatusItemView> =
            msg_send![super(view), initWithFrame: frame];
        view
    };
    let status_ns_view: &NSView = status_view.as_super();
    accept_indirect_touch_events(status_ns_view);

    let image_view = match image {
        Some(image) => NSImageView::imageViewWithImage(image, mtm),
        None => NSImageView::initWithFrame(mtm.alloc(), initial_image_frame(size)),
    };
    image_view.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    let image_ns_view: &NSView = image_view.as_super().as_super();
    image_ns_view.setFrame(initial_image_frame(size));
    status_ns_view.addSubview(image_ns_view);

    status_item.setLength(size.width);
    status_item.setMenu(None);
    status_item.setView(Some(status_ns_view));
    crate::macos_status_item_icon::install(status_item, status_ns_view, &image_view);
    log::warn!("tray context menu: installed custom status item view");

    Some(objc2::rc::Retained::into_super(status_view))
}

#[allow(deprecated)]
fn accept_indirect_touch_events(view: &NSView) {
    view.setAcceptsTouchEvents(true);
    view.setWantsRestingTouches(true);
    view.setAllowedTouchTypes(objc2_app_kit::NSTouchTypeMask::Indirect);
}

fn active_touch_count_for_event(event: &NSEvent, view: &NSView) -> usize {
    let touches_in_view = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, Some(view))
        .count();
    let touches_in_event = event
        .touchesMatchingPhase_inView(objc2_app_kit::NSTouchPhase::Touching, None)
        .count();

    touches_in_view.max(touches_in_event)
}

fn event_is_context_click(event: &NSEvent) -> bool {
    event_is_context_click_details(event.r#type(), event.modifierFlags(), event.buttonNumber())
}

fn event_is_context_click_details(
    event_type: NSEventType,
    modifier_flags: NSEventModifierFlags,
    button_number: isize,
) -> bool {
    event_type == NSEventType::RightMouseDown
        || event_type == NSEventType::RightMouseUp
        || event_type == NSEventType::OtherMouseDown
        || event_type == NSEventType::OtherMouseUp
        || ((event_type == NSEventType::LeftMouseDown || event_type == NSEventType::LeftMouseUp)
            && (modifier_flags.contains(NSEventModifierFlags::Control) || button_number == 1))
}

fn should_open_from_touch_count(menu_open_for_current_touch: bool, touch_count: usize) -> bool {
    !menu_open_for_current_touch && touch_count >= 2
}

fn should_reset_touch_gate(touch_count: usize) -> bool {
    touch_count < 2
}

fn normalized_status_item_size(size: NSSize) -> NSSize {
    NSSize::new(
        size.width.max(STATUS_ITEM_WIDTH),
        size.height.max(STATUS_ITEM_HEIGHT),
    )
}

fn point_is_inside_size(point: NSPoint, size: NSSize) -> bool {
    point.x >= 0.0 && point.x <= size.width && point.y >= 0.0 && point.y <= size.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_status_view_opens_for_secondary_click_shapes() {
        assert!(event_is_context_click_details(
            NSEventType::RightMouseDown,
            NSEventModifierFlags::empty(),
            0,
        ));
        assert!(event_is_context_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::Control,
            0,
        ));
        assert!(event_is_context_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            1,
        ));
        assert!(!event_is_context_click_details(
            NSEventType::LeftMouseDown,
            NSEventModifierFlags::empty(),
            0,
        ));
    }

    #[test]
    fn custom_status_view_touch_gate_opens_once_per_two_touch_sequence() {
        assert!(should_open_from_touch_count(false, 2));
        assert!(!should_open_from_touch_count(true, 2));
        assert!(!should_open_from_touch_count(false, 1));
        assert!(should_reset_touch_gate(1));
        assert!(!should_reset_touch_gate(2));
    }
}
