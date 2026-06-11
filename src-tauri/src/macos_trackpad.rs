use std::ffi::CString;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use objc2::Message;
use objc2_app_kit::{NSMenu, NSView};

type MTDeviceRef = *mut libc::c_void;
type CFArrayRef = *const libc::c_void;
type CFIndex = isize;
type MTContactCallback = unsafe extern "C" fn(
    libc::c_int,
    *mut libc::c_void,
    libc::c_int,
    libc::c_double,
    libc::c_int,
) -> libc::c_int;
type MTDeviceCreateDefault = unsafe extern "C" fn() -> MTDeviceRef;
type MTDeviceCreateList = unsafe extern "C" fn() -> CFArrayRef;
type MTRegisterContactFrameCallback = unsafe extern "C" fn(MTDeviceRef, MTContactCallback);
type MTDeviceStart = unsafe extern "C" fn(MTDeviceRef, libc::c_int);

const MULTITOUCH_FRAMEWORK_PATH: &str =
    "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";
const RAW_TRACKPAD_IDLE_END_MILLIS: u64 = 160;
const RAW_TRACKPAD_RECENT_TWO_FINGER_MILLIS: u64 = 300;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const libc::c_void;
}

#[derive(Debug)]
struct RawTrackpadTouchState {
    active_fingers: AtomicUsize,
    last_update_millis: AtomicU64,
    last_two_finger_millis: AtomicU64,
    runtime_started: AtomicBool,
}

impl RawTrackpadTouchState {
    fn new() -> Self {
        Self {
            active_fingers: AtomicUsize::new(0),
            last_update_millis: AtomicU64::new(0),
            last_two_finger_millis: AtomicU64::new(0),
            runtime_started: AtomicBool::new(false),
        }
    }
}

#[derive(Debug)]
struct RawTrackpadMenuTimerState {
    menu: objc2::rc::Retained<NSMenu>,
    status_view: objc2::rc::Retained<NSView>,
    touch_state: Arc<RawTrackpadTouchState>,
    two_finger_was_active: std::cell::Cell<bool>,
    two_finger_sequence_inside_status_view: std::cell::Cell<bool>,
    menu_opened_for_sequence: std::cell::Cell<bool>,
}

struct MultitouchSupportRuntime {
    _handle: *mut libc::c_void,
    _device_list: Option<CFArrayRef>,
    _devices: Vec<MTDeviceRef>,
}

static RAW_TRACKPAD_TOUCH_STATE: OnceLock<Arc<RawTrackpadTouchState>> = OnceLock::new();

pub(crate) fn install_context_click_fallback(menu: &NSMenu, status_view: &NSView) {
    let touch_state = RAW_TRACKPAD_TOUCH_STATE
        .get_or_init(|| Arc::new(RawTrackpadTouchState::new()))
        .clone();

    if !start_raw_trackpad_listener(&touch_state) {
        return;
    }

    install_raw_trackpad_menu_timer(menu, status_view, touch_state);
}

fn start_raw_trackpad_listener(touch_state: &RawTrackpadTouchState) -> bool {
    if touch_state.runtime_started.swap(true, Ordering::SeqCst) {
        return true;
    }

    match unsafe { MultitouchSupportRuntime::start() } {
        Ok(runtime) => {
            let device_count = runtime.device_count();
            std::mem::forget(runtime);
            log::warn!(
                "tray context menu: installed raw trackpad fallback for {device_count} device(s)"
            );
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
    status_view: &NSView,
    touch_state: Arc<RawTrackpadTouchState>,
) {
    use objc2_foundation::NSTimer;
    use std::ptr::NonNull;

    let state = std::rc::Rc::new(RawTrackpadMenuTimerState {
        menu: menu.retain(),
        status_view: status_view.retain(),
        touch_state,
        two_finger_was_active: std::cell::Cell::new(false),
        two_finger_sequence_inside_status_view: std::cell::Cell::new(false),
        menu_opened_for_sequence: std::cell::Cell::new(false),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let now_millis = raw_trackpad_elapsed_millis();
        let active_fingers = current_raw_active_fingers(&block_state.touch_state, now_millis);
        let two_finger_is_active = active_fingers >= 2;
        let recent_two_finger_seen =
            recent_raw_two_finger_seen(&block_state.touch_state, now_millis);
        let two_finger_signal_is_active = two_finger_is_active || recent_two_finger_seen;
        let cursor_inside_status_view =
            crate::tray::is_mouse_inside_status_view(&block_state.status_view);
        let two_finger_was_active = block_state
            .two_finger_was_active
            .replace(two_finger_signal_is_active);
        let sequence_inside_status_view = block_state.two_finger_sequence_inside_status_view.get()
            || (two_finger_signal_is_active && cursor_inside_status_view);
        block_state
            .two_finger_sequence_inside_status_view
            .set(sequence_inside_status_view);

        if should_open_raw_trackpad_menu_while_sequence_active(
            two_finger_signal_is_active,
            cursor_inside_status_view,
            block_state.menu_opened_for_sequence.get(),
        ) {
            block_state.menu_opened_for_sequence.set(true);
            log::debug!(
                "tray context menu: raw trackpad two-finger sequence active inside status view"
            );
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
            return;
        }

        if should_open_raw_trackpad_menu_on_sequence_end(
            two_finger_was_active,
            two_finger_signal_is_active,
            sequence_inside_status_view,
            block_state.menu_opened_for_sequence.get(),
        ) {
            block_state
                .two_finger_sequence_inside_status_view
                .set(false);
            block_state.menu_opened_for_sequence.set(false);
            log::debug!(
                "tray context menu: raw trackpad two-finger sequence ended active_fingers={active_fingers}"
            );
            crate::tray::show_native_tray_menu_at_view(&block_state.menu, &block_state.status_view);
        } else if !two_finger_signal_is_active {
            block_state
                .two_finger_sequence_inside_status_view
                .set(false);
            block_state.menu_opened_for_sequence.set(false);
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

        let create_list: Option<MTDeviceCreateList> =
            unsafe { load_optional_symbol(handle, b"MTDeviceCreateList\0") };
        let create_default: Option<MTDeviceCreateDefault> =
            unsafe { load_optional_symbol(handle, b"MTDeviceCreateDefault\0") };
        let register_callback: MTRegisterContactFrameCallback =
            unsafe { load_symbol(handle, b"MTRegisterContactFrameCallback\0")? };
        let device_start: MTDeviceStart = unsafe { load_symbol(handle, b"MTDeviceStart\0")? };

        let mut devices = Vec::new();
        let mut device_list = None;

        if let Some(create_list) = create_list {
            let list = unsafe { create_list() };
            if !list.is_null() {
                let device_count = unsafe { CFArrayGetCount(list) }.max(0);
                for index in 0..device_count {
                    let device = unsafe { CFArrayGetValueAtIndex(list, index) as MTDeviceRef };
                    if !device.is_null() {
                        devices.push(device);
                    }
                }
                device_list = Some(list);
            }
        }

        if devices.is_empty() {
            let Some(create_default) = create_default else {
                return Err("MTDeviceCreateList and MTDeviceCreateDefault unavailable".to_string());
            };

            let device = unsafe { create_default() };
            if !device.is_null() {
                devices.push(device);
            }
        }

        if devices.is_empty() {
            return Err("no MultitouchSupport devices available".to_string());
        }

        for &device in &devices {
            unsafe {
                register_callback(device, raw_trackpad_contact_callback);
                device_start(device, 0);
            }
        }

        Ok(Self {
            _handle: handle,
            _device_list: device_list,
            _devices: devices,
        })
    }

    fn device_count(&self) -> usize {
        self._devices.len()
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

unsafe fn load_optional_symbol<T: Copy>(handle: *mut libc::c_void, name: &[u8]) -> Option<T> {
    let symbol = unsafe { libc::dlsym(handle, name.as_ptr().cast()) };
    if symbol.is_null() {
        return None;
    }

    Some(unsafe { std::mem::transmute_copy(&symbol) })
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
    _device: libc::c_int,
    _touches: *mut libc::c_void,
    active_fingers: libc::c_int,
    _timestamp: libc::c_double,
    _frame: libc::c_int,
) -> libc::c_int {
    if let Some(state) = RAW_TRACKPAD_TOUCH_STATE.get() {
        let active_fingers = active_fingers.max(0) as usize;
        let now_millis = raw_trackpad_elapsed_millis();
        let previous = state.active_fingers.swap(active_fingers, Ordering::SeqCst);
        state.last_update_millis.store(now_millis, Ordering::SeqCst);
        if active_fingers >= 2 {
            state
                .last_two_finger_millis
                .store(now_millis, Ordering::SeqCst);
        }
        if previous != active_fingers {
            log::debug!("tray context menu: raw trackpad active_fingers={active_fingers}");
        }
    }

    0
}

fn current_raw_active_fingers(state: &RawTrackpadTouchState, now_millis: u64) -> usize {
    let active_fingers = state.active_fingers.load(Ordering::SeqCst);
    if active_fingers < 2 {
        return active_fingers;
    }

    if raw_trackpad_update_is_stale(
        now_millis,
        state.last_update_millis.load(Ordering::SeqCst),
        RAW_TRACKPAD_IDLE_END_MILLIS,
    ) {
        state.active_fingers.store(0, Ordering::SeqCst);
        return 0;
    }

    active_fingers
}

fn recent_raw_two_finger_seen(state: &RawTrackpadTouchState, now_millis: u64) -> bool {
    let last_two_finger_millis = state.last_two_finger_millis.load(Ordering::SeqCst);
    if last_two_finger_millis == 0 {
        return false;
    }

    !raw_trackpad_update_is_stale(
        now_millis,
        last_two_finger_millis,
        RAW_TRACKPAD_RECENT_TWO_FINGER_MILLIS,
    )
}

fn raw_trackpad_update_is_stale(
    now_millis: u64,
    last_update_millis: u64,
    max_age_millis: u64,
) -> bool {
    last_update_millis > 0 && now_millis.saturating_sub(last_update_millis) >= max_age_millis
}

fn raw_trackpad_elapsed_millis() -> u64 {
    static STARTED_AT: OnceLock<std::time::Instant> = OnceLock::new();
    STARTED_AT
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn should_open_raw_trackpad_menu_on_sequence_end(
    two_finger_was_active: bool,
    two_finger_is_active: bool,
    two_finger_sequence_inside_status_view: bool,
    menu_opened_for_sequence: bool,
) -> bool {
    two_finger_was_active
        && !two_finger_is_active
        && two_finger_sequence_inside_status_view
        && !menu_opened_for_sequence
}

fn should_open_raw_trackpad_menu_while_sequence_active(
    two_finger_is_active: bool,
    cursor_inside_status_view: bool,
    menu_opened_for_sequence: bool,
) -> bool {
    two_finger_is_active && cursor_inside_status_view && !menu_opened_for_sequence
}

#[cfg(test)]
#[path = "macos_trackpad_tests.rs"]
mod macos_trackpad_tests;
