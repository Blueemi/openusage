use std::ffi::CString;

pub(crate) type MTDeviceRef = *mut libc::c_void;
type CFArrayRef = *const libc::c_void;
type CFIndex = isize;
pub(crate) type MTContactCallback =
    unsafe extern "C" fn(MTDeviceRef, *mut libc::c_void, libc::c_int, libc::c_double, libc::c_int);
type MTDeviceCreateDefault = unsafe extern "C" fn() -> MTDeviceRef;
type MTDeviceCreateList = unsafe extern "C" fn() -> CFArrayRef;
type MTDeviceIsAvailable = unsafe extern "C" fn() -> bool;
type MTRegisterContactFrameCallback = unsafe extern "C" fn(MTDeviceRef, MTContactCallback);
type MTDeviceStart = unsafe extern "C" fn(MTDeviceRef, libc::c_int) -> libc::c_int;

const MULTITOUCH_FRAMEWORK_PATH: &str =
    "/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> *const libc::c_void;
}

pub(crate) struct MultitouchSupportRuntime {
    _handle: *mut libc::c_void,
    _device_list: Option<CFArrayRef>,
    _devices: Vec<MTDeviceRef>,
}

impl MultitouchSupportRuntime {
    pub(crate) unsafe fn start(callback: MTContactCallback) -> Result<Self, String> {
        let path = CString::new(MULTITOUCH_FRAMEWORK_PATH).expect("valid framework path");
        let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW) };
        if handle.is_null() {
            return Err(dl_error());
        }

        let create_list: Option<MTDeviceCreateList> =
            unsafe { load_optional_symbol(handle, b"MTDeviceCreateList\0") };
        let create_default: Option<MTDeviceCreateDefault> =
            unsafe { load_optional_symbol(handle, b"MTDeviceCreateDefault\0") };
        let device_is_available: Option<MTDeviceIsAvailable> =
            unsafe { load_optional_symbol(handle, b"MTDeviceIsAvailable\0") };
        let register_callback: MTRegisterContactFrameCallback =
            unsafe { load_symbol(handle, b"MTRegisterContactFrameCallback\0")? };
        let device_start: MTDeviceStart = unsafe { load_symbol(handle, b"MTDeviceStart\0")? };

        let (devices, device_list) =
            unsafe { create_multitouch_devices(create_list, create_default, device_is_available)? };
        let mut started_device_count = 0usize;

        for &device in &devices {
            unsafe {
                register_callback(device, callback);
                let start_status = device_start(device, 0);
                if mt_status_is_success(start_status) {
                    started_device_count += 1;
                } else {
                    log::warn!(
                        "tray context menu: raw trackpad device start failed status={start_status}"
                    );
                }
            }
        }

        if started_device_count == 0 {
            return Err("no MultitouchSupport devices started".to_string());
        }

        Ok(Self {
            _handle: handle,
            _device_list: device_list,
            _devices: devices,
        })
    }

    pub(crate) fn device_count(&self) -> usize {
        self._devices.len()
    }
}

unsafe fn create_multitouch_devices(
    create_list: Option<MTDeviceCreateList>,
    create_default: Option<MTDeviceCreateDefault>,
    device_is_available: Option<MTDeviceIsAvailable>,
) -> Result<(Vec<MTDeviceRef>, Option<CFArrayRef>), String> {
    let mut devices = Vec::new();
    let mut device_list = None;

    if multitouch_default_device_is_available(device_is_available) {
        if let Some(create_default) = create_default {
            let device = unsafe { create_default() };
            push_unique_device(&mut devices, device);
        }
    }

    if let Some(create_list) = create_list {
        let list = unsafe { create_list() };
        if !list.is_null() {
            let device_count = unsafe { CFArrayGetCount(list) }.max(0);
            for index in 0..device_count {
                let device = unsafe { CFArrayGetValueAtIndex(list, index) as MTDeviceRef };
                push_unique_device(&mut devices, device);
            }
            device_list = Some(list);
        }
    }

    if devices.is_empty() {
        return Err("no MultitouchSupport devices available".to_string());
    }

    Ok((devices, device_list))
}

fn multitouch_default_device_is_available(
    device_is_available: Option<MTDeviceIsAvailable>,
) -> bool {
    device_is_available
        .map(|is_available| unsafe { is_available() })
        .unwrap_or(true)
}

fn push_unique_device(devices: &mut Vec<MTDeviceRef>, device: MTDeviceRef) {
    if device.is_null() || devices.contains(&device) {
        return;
    }

    devices.push(device);
}

fn mt_status_is_success(status: libc::c_int) -> bool {
    status == 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mt_status_zero_is_success() {
        assert!(mt_status_is_success(0));
        assert!(!mt_status_is_success(-1));
        assert!(!mt_status_is_success(1));
    }

    unsafe extern "C" fn unavailable_default_device() -> bool {
        false
    }

    #[test]
    fn default_device_availability_is_optimistic_without_symbol() {
        assert!(multitouch_default_device_is_available(None));
        assert!(!multitouch_default_device_is_available(Some(
            unavailable_default_device
        )));
    }

    #[test]
    fn push_unique_device_skips_null_and_duplicates() {
        let mut devices = Vec::new();
        let device = 1usize as MTDeviceRef;

        push_unique_device(&mut devices, std::ptr::null_mut());
        push_unique_device(&mut devices, device);
        push_unique_device(&mut devices, device);

        assert_eq!(devices, vec![device]);
    }
}
