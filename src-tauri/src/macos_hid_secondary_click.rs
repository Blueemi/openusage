use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use objc2::Message;
use objc2_app_kit::{NSMenu, NSView};

#[path = "macos_hid_secondary_click_logic.rs"]
mod logic;
use logic::*;

type CFAllocatorRef = *const libc::c_void;
type CFArrayRef = *const libc::c_void;
type CFMutableDictionaryRef = *mut libc::c_void;
type CFNumberRef = *const libc::c_void;
type CFRunLoopRef = *const libc::c_void;
type CFStringRef = *const libc::c_void;
type IOHIDElementRef = *mut libc::c_void;
type IOHIDManagerRef = *mut libc::c_void;
type IOHIDValueRef = *mut libc::c_void;
type IOReturn = libc::c_int;
type IOOptionBits = u32;

const DEVICE_USAGE_KEY: &[u8] = b"DeviceUsage\0";
const DEVICE_USAGE_PAGE_KEY: &[u8] = b"DeviceUsagePage\0";
const HID_USAGE_PAGE_DESKTOP: i32 = 0x01;
const HID_USAGE_PAGE_DIGITIZER: i32 = 0x0d;
const HID_USAGE_MOUSE: i32 = 0x02;
const HID_USAGE_POINTER: i32 = 0x01;
const HID_USAGE_TOUCHPAD: i32 = 0x05;
const K_CF_NUMBER_SINT32_TYPE: libc::c_int = 3;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;

    fn CFArrayCreate(
        allocator: CFAllocatorRef,
        values: *const *const libc::c_void,
        num_values: isize,
        callbacks: *const libc::c_void,
    ) -> CFArrayRef;
    fn CFDictionaryCreateMutable(
        allocator: CFAllocatorRef,
        capacity: isize,
        key_callbacks: *const libc::c_void,
        value_callbacks: *const libc::c_void,
    ) -> CFMutableDictionaryRef;
    fn CFDictionarySetValue(
        dictionary: CFMutableDictionaryRef,
        key: *const libc::c_void,
        value: *const libc::c_void,
    );
    fn CFNumberCreate(
        allocator: CFAllocatorRef,
        the_type: libc::c_int,
        value_ptr: *const libc::c_void,
    ) -> CFNumberRef;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFStringCreateWithCString(
        allocator: CFAllocatorRef,
        c_str: *const libc::c_char,
        encoding: u32,
    ) -> CFStringRef;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
    fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    fn IOHIDManagerCreate(allocator: CFAllocatorRef, options: IOOptionBits) -> IOHIDManagerRef;
    fn IOHIDManagerOpen(manager: IOHIDManagerRef, options: IOOptionBits) -> IOReturn;
    fn IOHIDManagerRegisterInputValueCallback(
        manager: IOHIDManagerRef,
        callback: Option<IOHIDValueCallback>,
        context: *mut libc::c_void,
    );
    fn IOHIDManagerScheduleWithRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: CFStringRef,
    );
    fn IOHIDManagerSetDeviceMatching(manager: IOHIDManagerRef, matching: *const libc::c_void);
    fn IOHIDManagerSetDeviceMatchingMultiple(manager: IOHIDManagerRef, multiple: CFArrayRef);
    fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
    fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> libc::c_long;
}

type IOHIDValueCallback = unsafe extern "C" fn(
    context: *mut libc::c_void,
    result: IOReturn,
    sender: *mut libc::c_void,
    value: IOHIDValueRef,
);

#[derive(Debug)]
struct HidSecondaryClickTimerState {
    menu: objc2::rc::Retained<NSMenu>,
    status_view: objc2::rc::Retained<NSView>,
    click_state: Arc<HidSecondaryClickState>,
    primary_button_was_down: std::cell::Cell<bool>,
    secondary_button_was_down: std::cell::Cell<bool>,
}

struct HidSecondaryClickRuntime {
    _manager: IOHIDManagerRef,
    _state: Arc<HidSecondaryClickState>,
}

static HID_SECONDARY_CLICK_STATE: OnceLock<Arc<HidSecondaryClickState>> = OnceLock::new();

pub(crate) fn install(menu: &NSMenu, status_view: &NSView) {
    let click_state = HID_SECONDARY_CLICK_STATE
        .get_or_init(|| Arc::new(HidSecondaryClickState::new()))
        .clone();

    if !start_hid_secondary_click_listener(&click_state) {
        return;
    }

    install_hid_secondary_click_timer(menu, status_view, click_state);
}

fn start_hid_secondary_click_listener(click_state: &Arc<HidSecondaryClickState>) -> bool {
    if click_state.runtime_started.swap(true, Ordering::SeqCst) {
        return true;
    }

    match unsafe { HidSecondaryClickRuntime::start(click_state.clone()) } {
        Ok(runtime) => {
            std::mem::forget(runtime);
            log::warn!("tray context menu: installed IOHID secondary-click fallback");
            true
        }
        Err(error) => {
            click_state.runtime_started.store(false, Ordering::SeqCst);
            log::warn!("tray context menu: IOHID secondary-click fallback unavailable: {error}");
            false
        }
    }
}

fn install_hid_secondary_click_timer(
    menu: &NSMenu,
    status_view: &NSView,
    click_state: Arc<HidSecondaryClickState>,
) {
    use objc2_foundation::NSTimer;
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(HidSecondaryClickTimerState {
        menu: menu.retain(),
        status_view: status_view.retain(),
        click_state,
        primary_button_was_down: std::cell::Cell::new(false),
        secondary_button_was_down: std::cell::Cell::new(false),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let now_millis = hid_elapsed_millis();
        let primary_button_is_down = block_state
            .click_state
            .primary_button_is_down
            .load(Ordering::SeqCst);
        let secondary_button_is_down = block_state
            .click_state
            .secondary_button_is_down
            .load(Ordering::SeqCst);
        let primary_button_was_down = block_state
            .primary_button_was_down
            .replace(primary_button_is_down);
        let secondary_button_was_down = block_state
            .secondary_button_was_down
            .replace(secondary_button_is_down);
        let two_contact_signal_is_active = hid_two_contact_signal_is_active(
            &block_state.click_state,
            now_millis,
            HID_RECENT_TWO_CONTACT_MILLIS,
        );
        let cursor_inside_status_view =
            crate::tray::is_mouse_inside_status_view(&block_state.status_view);

        if should_open_menu_from_hid_button_state(
            secondary_button_was_down,
            secondary_button_is_down,
            cursor_inside_status_view,
        ) {
            log::debug!("tray context menu: IOHID secondary button down");
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
        } else if should_open_menu_from_hid_primary_two_contact_state(
            primary_button_was_down,
            primary_button_is_down,
            two_contact_signal_is_active,
            cursor_inside_status_view,
        ) {
            log::warn!("tray context menu: IOHID primary button with two contacts");
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
        }
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSTimer>)> = &block;
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.025, true, block_ref) };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(timer);
}

impl HidSecondaryClickRuntime {
    unsafe fn start(state: Arc<HidSecondaryClickState>) -> Result<Self, String> {
        let manager = unsafe { IOHIDManagerCreate(std::ptr::null(), 0) };
        if manager.is_null() {
            return Err("IOHIDManagerCreate returned null".to_string());
        }

        unsafe {
            set_pointer_device_matching(manager);
            let open_status = IOHIDManagerOpen(manager, 0);
            if !iohid_status_is_success(open_status) {
                IOHIDManagerSetDeviceMatching(manager, std::ptr::null());
                let broad_open_status = IOHIDManagerOpen(manager, 0);
                if !iohid_status_is_success(broad_open_status) {
                    return Err(format!(
                        "IOHIDManagerOpen failed status={open_status}, broad_status={broad_open_status}"
                    ));
                }
                log::warn!("tray context menu: IOHID opened with broad device matching");
            }

            IOHIDManagerRegisterInputValueCallback(
                manager,
                Some(hid_secondary_click_value_callback),
                Arc::as_ptr(&state) as *mut libc::c_void,
            );
        }

        let run_loop = unsafe { CFRunLoopGetMain() };
        if run_loop.is_null() {
            return Err("main run loop unavailable".to_string());
        }

        unsafe { IOHIDManagerScheduleWithRunLoop(manager, run_loop, kCFRunLoopCommonModes) };

        Ok(Self {
            _manager: manager,
            _state: state,
        })
    }
}

unsafe fn set_pointer_device_matching(manager: IOHIDManagerRef) {
    let matchers = [
        unsafe { create_device_matching_dictionary(HID_USAGE_PAGE_DESKTOP, HID_USAGE_MOUSE) },
        unsafe { create_device_matching_dictionary(HID_USAGE_PAGE_DESKTOP, HID_USAGE_POINTER) },
        unsafe { create_device_matching_dictionary(HID_USAGE_PAGE_DIGITIZER, HID_USAGE_TOUCHPAD) },
    ];
    let values = [
        matchers[0] as *const libc::c_void,
        matchers[1] as *const libc::c_void,
        matchers[2] as *const libc::c_void,
    ];
    let array = unsafe {
        CFArrayCreate(
            std::ptr::null(),
            values.as_ptr(),
            values.len() as isize,
            std::ptr::null(),
        )
    };

    if array.is_null() {
        unsafe { IOHIDManagerSetDeviceMatching(manager, std::ptr::null()) };
        return;
    }

    unsafe { IOHIDManagerSetDeviceMatchingMultiple(manager, array) };
}

unsafe fn create_device_matching_dictionary(usage_page: i32, usage: i32) -> CFMutableDictionaryRef {
    let dictionary = unsafe {
        CFDictionaryCreateMutable(std::ptr::null(), 0, std::ptr::null(), std::ptr::null())
    };
    if dictionary.is_null() {
        return dictionary;
    }

    let usage_page_number = unsafe { create_i32_cf_number(usage_page) };
    let usage_number = unsafe { create_i32_cf_number(usage) };
    let usage_page_key = unsafe { create_cf_string(DEVICE_USAGE_PAGE_KEY) };
    let usage_key = unsafe { create_cf_string(DEVICE_USAGE_KEY) };
    if usage_page_number.is_null()
        || usage_number.is_null()
        || usage_page_key.is_null()
        || usage_key.is_null()
    {
        return dictionary;
    }

    unsafe {
        CFDictionarySetValue(dictionary, usage_page_key.cast(), usage_page_number.cast());
        CFDictionarySetValue(dictionary, usage_key.cast(), usage_number.cast());
    }

    dictionary
}

unsafe fn create_i32_cf_number(value: i32) -> CFNumberRef {
    unsafe {
        CFNumberCreate(
            std::ptr::null(),
            K_CF_NUMBER_SINT32_TYPE,
            (&value as *const i32).cast(),
        )
    }
}

unsafe fn create_cf_string(value: &[u8]) -> CFStringRef {
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            value.as_ptr().cast(),
            K_CF_STRING_ENCODING_UTF8,
        )
    }
}

unsafe extern "C" fn hid_secondary_click_value_callback(
    context: *mut libc::c_void,
    result: IOReturn,
    _sender: *mut libc::c_void,
    value: IOHIDValueRef,
) {
    if context.is_null() || !iohid_status_is_success(result) || value.is_null() {
        return;
    }

    let element = unsafe { IOHIDValueGetElement(value) };
    if element.is_null() {
        return;
    }

    let usage_page = unsafe { IOHIDElementGetUsagePage(element) };
    let usage = unsafe { IOHIDElementGetUsage(element) };
    let state = unsafe { &*(context.cast::<HidSecondaryClickState>()) };
    let integer_value = unsafe { IOHIDValueGetIntegerValue(value) };
    if is_button_hid_usage(usage_page, usage, HID_USAGE_BUTTON_PRIMARY) {
        update_hid_button_state(
            state,
            &state.primary_button_is_down,
            integer_value,
            "primary",
        );
    } else if is_secondary_button_hid_usage(usage_page, usage) {
        update_hid_button_state(
            state,
            &state.secondary_button_is_down,
            integer_value,
            "secondary",
        );
    } else if is_contact_count_hid_usage(usage_page, usage) {
        update_hid_contact_count(state, integer_value);
    }
}

fn iohid_status_is_success(status: IOReturn) -> bool {
    status == 0
}

#[cfg(test)]
#[path = "macos_hid_secondary_click_tests.rs"]
mod tests;
