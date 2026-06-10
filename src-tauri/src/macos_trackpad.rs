use std::ffi::CString;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use objc2::Message;
use objc2_app_kit::{NSMenu, NSStatusItem, NSView};

type MTDeviceRef = *mut libc::c_void;
type MTContactCallback = unsafe extern "C" fn(
    MTDeviceRef,
    *mut libc::c_void,
    libc::c_int,
    libc::c_double,
    libc::c_int,
) -> libc::c_int;
type MTDeviceCreateDefault = unsafe extern "C" fn() -> MTDeviceRef;
type MTRegisterContactFrameCallback = unsafe extern "C" fn(MTDeviceRef, MTContactCallback);
type MTDeviceStart = unsafe extern "C" fn(MTDeviceRef, libc::c_int);

const MULTITOUCH_FRAMEWORK_PATH: &str =
    "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";

#[derive(Debug)]
struct RawTrackpadTouchState {
    active_fingers: AtomicUsize,
    runtime_started: AtomicBool,
}

impl RawTrackpadTouchState {
    fn new() -> Self {
        Self {
            active_fingers: AtomicUsize::new(0),
            runtime_started: AtomicBool::new(false),
        }
    }
}

#[derive(Debug)]
struct RawTrackpadMenuTimerState {
    menu: objc2::rc::Retained<NSMenu>,
    status_item: objc2::rc::Retained<NSStatusItem>,
    status_view: objc2::rc::Retained<NSView>,
    touch_state: Arc<RawTrackpadTouchState>,
    two_finger_was_active: std::cell::Cell<bool>,
}

struct MultitouchSupportRuntime {
    _handle: *mut libc::c_void,
    _device: MTDeviceRef,
}

static RAW_TRACKPAD_TOUCH_STATE: OnceLock<Arc<RawTrackpadTouchState>> = OnceLock::new();

pub(crate) fn install_context_click_fallback(
    menu: &NSMenu,
    status_item: &NSStatusItem,
    status_view: &NSView,
) {
    let touch_state = RAW_TRACKPAD_TOUCH_STATE
        .get_or_init(|| Arc::new(RawTrackpadTouchState::new()))
        .clone();

    if !start_raw_trackpad_listener(&touch_state) {
        return;
    }

    install_raw_trackpad_menu_timer(menu, status_item, status_view, touch_state);
}

fn start_raw_trackpad_listener(touch_state: &RawTrackpadTouchState) -> bool {
    if touch_state.runtime_started.swap(true, Ordering::SeqCst) {
        return true;
    }

    match unsafe { MultitouchSupportRuntime::start() } {
        Ok(runtime) => {
            std::mem::forget(runtime);
            log::warn!("tray context menu: installed raw trackpad fallback");
            true
        }
        Err(error) => {
            touch_state.runtime_started.store(false, Ordering::SeqCst);
            log::warn!("tray context menu: raw trackpad fallback unavailable: {error}");
            false
        }
    }
}

fn install_raw_trackpad_menu_timer(
    menu: &NSMenu,
    status_item: &NSStatusItem,
    status_view: &NSView,
    touch_state: Arc<RawTrackpadTouchState>,
) {
    use objc2_foundation::NSTimer;
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(RawTrackpadMenuTimerState {
        menu: menu.retain(),
        status_item: status_item.retain(),
        status_view: status_view.retain(),
        touch_state,
        two_finger_was_active: std::cell::Cell::new(false),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let active_fingers = block_state
            .touch_state
            .active_fingers
            .load(Ordering::SeqCst);
        let two_finger_is_active = active_fingers >= 2;
        let two_finger_was_active = block_state
            .two_finger_was_active
            .replace(two_finger_is_active);

        if should_open_raw_trackpad_menu(
            two_finger_was_active,
            two_finger_is_active,
            crate::tray::is_mouse_inside_status_view(&block_state.status_view),
        ) {
            log::debug!(
                "tray context menu: raw trackpad two-finger fallback active_fingers={active_fingers}"
            );
            crate::tray::show_native_tray_menu(&block_state.status_item, &block_state.menu);
        }
    });
    let block_ref: &block2::DynBlock<dyn Fn(NonNull<NSTimer>)> = &block;
    let timer =
        unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.025, true, block_ref) };

    std::mem::forget(state);
    std::mem::forget(block);
    std::mem::forget(timer);
}

impl MultitouchSupportRuntime {
    unsafe fn start() -> Result<Self, String> {
        let path = CString::new(MULTITOUCH_FRAMEWORK_PATH).expect("valid framework path");
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW) };
        if handle.is_null() {
            return Err(dl_error());
        }

        let create_default: MTDeviceCreateDefault =
            unsafe { load_symbol(handle, b"MTDeviceCreateDefault\0")? };
        let register_callback: MTRegisterContactFrameCallback =
            unsafe { load_symbol(handle, b"MTRegisterContactFrameCallback\0")? };
        let device_start: MTDeviceStart = unsafe { load_symbol(handle, b"MTDeviceStart\0")? };

        let device = unsafe { create_default() };
        if device.is_null() {
            return Err("MTDeviceCreateDefault returned null".to_string());
        }

        unsafe {
            register_callback(device, raw_trackpad_contact_callback);
            device_start(device, 0);
        }

        Ok(Self {
            _handle: handle,
            _device: device,
        })
    }
}

unsafe fn load_symbol<T: Copy>(handle: *mut libc::c_void, name: &[u8]) -> Result<T, String> {
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
    if symbol.is_null() {
        return Err(format!(
            "{} unavailable",
            String::from_utf8_lossy(&name[..name.len().saturating_sub(1)])
        ));
    }

    Ok(unsafe { std::mem::transmute_copy(&symbol) })
}

fn dl_error() -> String {
    let error = unsafe { libc::dlerror() };
    if error.is_null() {
        return "unknown dlopen error".to_string();
    }

    unsafe { std::ffi::CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn raw_trackpad_contact_callback(
    _device: MTDeviceRef,
    _touches: *mut libc::c_void,
    active_fingers: libc::c_int,
    _timestamp: libc::c_double,
    _frame: libc::c_int,
) -> libc::c_int {
    if let Some(state) = RAW_TRACKPAD_TOUCH_STATE.get() {
        state
            .active_fingers
            .store(active_fingers.max(0) as usize, Ordering::SeqCst);
    }

    0
}

fn should_open_raw_trackpad_menu(
    two_finger_was_active: bool,
    two_finger_is_active: bool,
    cursor_inside_status_view: bool,
) -> bool {
    !two_finger_was_active && two_finger_is_active && cursor_inside_status_view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_trackpad_menu_opens_on_two_finger_enter_inside_status_view() {
        assert!(should_open_raw_trackpad_menu(false, true, true));
        assert!(!should_open_raw_trackpad_menu(true, true, true));
        assert!(!should_open_raw_trackpad_menu(false, false, true));
        assert!(!should_open_raw_trackpad_menu(false, true, false));
    }
}
