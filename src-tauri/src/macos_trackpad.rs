use std::ffi::CString;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use objc2::Message;
use objc2_app_kit::{NSMenu, NSStatusItem, NSView};

type MTDeviceRef = *mut libc::c_void;
type CFArrayRef = *const libc::c_void;
type CFIndex = isize;
#[repr(C)]
struct MTPoint {
    _x: f32,
    _y: f32,
}

#[repr(C)]
struct MTVector {
    _position: MTPoint,
    _velocity: MTPoint,
}

#[repr(C)]
struct MTTouch {
    _frame: i32,
    _timestamp: f64,
    _path_index: i32,
    state: i32,
    _finger_id: i32,
    _hand_id: i32,
    _normalized: MTVector,
    z_total: f32,
    _field_9: i32,
    _angle: f32,
    _major_axis: f32,
    _minor_axis: f32,
    _absolute: MTVector,
    _field_14: i32,
    _field_15: i32,
    z_density: f32,
}

type MTContactCallback =
    unsafe extern "C" fn(MTDeviceRef, *mut MTTouch, usize, libc::c_double, usize);
type MTDeviceCreateDefault = unsafe extern "C" fn() -> MTDeviceRef;
type MTDeviceCreateList = unsafe extern "C" fn() -> CFArrayRef;
type MTRegisterContactFrameCallback = unsafe extern "C" fn(MTDeviceRef, MTContactCallback);
type MTDeviceStart = unsafe extern "C" fn(MTDeviceRef, libc::c_int);

const MULTITOUCH_FRAMEWORK_PATH: &str =
    "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const libc::c_void;
}

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
    two_finger_sequence_inside_status_view: std::cell::Cell<bool>,
}

struct MultitouchSupportRuntime {
    _handle: *mut libc::c_void,
    _device_list: Option<CFArrayRef>,
    _devices: Vec<MTDeviceRef>,
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
        two_finger_sequence_inside_status_view: std::cell::Cell::new(false),
    });
    let block_state = state.clone();
    let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let active_fingers = block_state
            .touch_state
            .active_fingers
            .load(Ordering::SeqCst);
        let two_finger_is_active = active_fingers >= 2;
        let cursor_inside_status_view =
            crate::tray::is_mouse_inside_status_view(&block_state.status_view);
        let two_finger_was_active = block_state
            .two_finger_was_active
            .replace(two_finger_is_active);
        let sequence_inside_status_view = block_state.two_finger_sequence_inside_status_view.get()
            || (two_finger_is_active && cursor_inside_status_view);
        block_state
            .two_finger_sequence_inside_status_view
            .set(sequence_inside_status_view);

        if should_open_raw_trackpad_menu_on_sequence_end(
            two_finger_was_active,
            two_finger_is_active,
            sequence_inside_status_view,
        ) {
            block_state
                .two_finger_sequence_inside_status_view
                .set(false);
            log::debug!(
                "tray context menu: raw trackpad two-finger sequence ended active_fingers={active_fingers}"
            );
            crate::tray::show_native_tray_menu(&block_state.status_item, &block_state.menu);
        } else if !two_finger_is_active {
            block_state
                .two_finger_sequence_inside_status_view
                .set(false);
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
    _device: MTDeviceRef,
    touches: *mut MTTouch,
    touch_count: usize,
    _timestamp: libc::c_double,
    _frame: usize,
) {
    if let Some(state) = RAW_TRACKPAD_TOUCH_STATE.get() {
        let active_fingers = raw_active_touch_count(touches, touch_count);
        let previous = state.active_fingers.swap(active_fingers, Ordering::SeqCst);
        if previous != active_fingers {
            log::debug!(
                "tray context menu: raw trackpad active_fingers={active_fingers} touch_count={touch_count}"
            );
        }
    }
}

fn raw_active_touch_count(touches: *const MTTouch, touch_count: usize) -> usize {
    if touches.is_null() {
        return touch_count;
    }

    let touching_count = (0..touch_count)
        .filter(|index| {
            let touch = unsafe { &*touches.add(*index) };
            raw_touch_is_active(touch.state, touch.z_total, touch.z_density)
        })
        .count();

    if touching_count > 0 {
        touching_count
    } else {
        touch_count
    }
}

fn raw_touch_is_active(state: i32, z_total: f32, z_density: f32) -> bool {
    matches!(state, 3 | 4) || z_total > 0.0 || z_density > 0.0
}

fn should_open_raw_trackpad_menu_on_sequence_end(
    two_finger_was_active: bool,
    two_finger_is_active: bool,
    two_finger_sequence_inside_status_view: bool,
) -> bool {
    two_finger_was_active && !two_finger_is_active && two_finger_sequence_inside_status_view
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_trackpad_menu_opens_when_two_finger_sequence_ends_inside_status_view() {
        assert!(should_open_raw_trackpad_menu_on_sequence_end(
            true, false, true
        ));
        assert!(!should_open_raw_trackpad_menu_on_sequence_end(
            false, true, true
        ));
        assert!(!should_open_raw_trackpad_menu_on_sequence_end(
            true, true, true
        ));
        assert!(!should_open_raw_trackpad_menu_on_sequence_end(
            true, false, false
        ));
    }

    #[test]
    fn raw_touch_count_prefers_active_touch_states() {
        let touches = [
            MTTouch {
                _frame: 0,
                _timestamp: 0.0,
                _path_index: 0,
                state: 2,
                _finger_id: 0,
                _hand_id: 0,
                _normalized: MTVector {
                    _position: MTPoint { _x: 0.0, _y: 0.0 },
                    _velocity: MTPoint { _x: 0.0, _y: 0.0 },
                },
                z_total: 0.0,
                _field_9: 0,
                _angle: 0.0,
                _major_axis: 0.0,
                _minor_axis: 0.0,
                _absolute: MTVector {
                    _position: MTPoint { _x: 0.0, _y: 0.0 },
                    _velocity: MTPoint { _x: 0.0, _y: 0.0 },
                },
                _field_14: 0,
                _field_15: 0,
                z_density: 0.0,
            },
            MTTouch {
                _frame: 0,
                _timestamp: 0.0,
                _path_index: 0,
                state: 4,
                _finger_id: 1,
                _hand_id: 0,
                _normalized: MTVector {
                    _position: MTPoint { _x: 0.0, _y: 0.0 },
                    _velocity: MTPoint { _x: 0.0, _y: 0.0 },
                },
                z_total: 0.0,
                _field_9: 0,
                _angle: 0.0,
                _major_axis: 0.0,
                _minor_axis: 0.0,
                _absolute: MTVector {
                    _position: MTPoint { _x: 0.0, _y: 0.0 },
                    _velocity: MTPoint { _x: 0.0, _y: 0.0 },
                },
                _field_14: 0,
                _field_15: 0,
                z_density: 0.0,
            },
        ];

        assert_eq!(raw_active_touch_count(touches.as_ptr(), touches.len()), 1);
        assert_eq!(raw_active_touch_count(std::ptr::null(), 2), 2);
    }
}
