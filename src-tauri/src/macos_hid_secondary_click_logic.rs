use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub(super) const HID_USAGE_PAGE_BUTTON: u32 = 0x09;
pub(super) const HID_USAGE_BUTTON_PRIMARY: u32 = 0x01;
pub(super) const HID_USAGE_BUTTON_SECONDARY: u32 = 0x02;
pub(super) const HID_USAGE_PAGE_DIGITIZER_U32: u32 = 0x0d;
pub(super) const HID_USAGE_DIGITIZER_CONTACT_COUNT: u32 = 0x54;
pub(super) const HID_RECENT_TWO_CONTACT_MILLIS: u64 = 240;

#[derive(Debug)]
pub(super) struct HidSecondaryClickState {
    pub(super) primary_button_is_down: AtomicBool,
    pub(super) secondary_button_is_down: AtomicBool,
    pub(super) contact_count: AtomicUsize,
    pub(super) last_two_contact_millis: AtomicU64,
    value_frames: AtomicU64,
    pub(super) runtime_started: AtomicBool,
}

impl HidSecondaryClickState {
    pub(super) fn new() -> Self {
        Self {
            primary_button_is_down: AtomicBool::new(false),
            secondary_button_is_down: AtomicBool::new(false),
            contact_count: AtomicUsize::new(0),
            last_two_contact_millis: AtomicU64::new(0),
            value_frames: AtomicU64::new(0),
            runtime_started: AtomicBool::new(false),
        }
    }
}

pub(super) fn update_hid_button_state(
    state: &HidSecondaryClickState,
    button_state: &AtomicBool,
    integer_value: libc::c_long,
    label: &str,
) {
    let is_down = integer_value != 0;
    let previous = button_state.swap(is_down, Ordering::SeqCst);
    let frames = state.value_frames.fetch_add(1, Ordering::SeqCst) + 1;
    if frames == 1 || previous != is_down {
        log::debug!("tray context menu: IOHID {label} button is_down={is_down}");
    }
}

pub(super) fn update_hid_contact_count(
    state: &HidSecondaryClickState,
    integer_value: libc::c_long,
) {
    let contact_count = normalize_hid_contact_count(integer_value);
    let previous = state.contact_count.swap(contact_count, Ordering::SeqCst);
    if contact_count >= 2 {
        state
            .last_two_contact_millis
            .store(hid_elapsed_millis(), Ordering::SeqCst);
    }
    if previous != contact_count {
        log::debug!("tray context menu: IOHID contact_count={contact_count}");
    }
}

pub(super) fn is_button_hid_usage(usage_page: u32, usage: u32, expected_usage: u32) -> bool {
    usage_page == HID_USAGE_PAGE_BUTTON && usage == expected_usage
}

pub(super) fn is_secondary_button_hid_usage(usage_page: u32, usage: u32) -> bool {
    is_button_hid_usage(usage_page, usage, HID_USAGE_BUTTON_SECONDARY)
}

pub(super) fn is_contact_count_hid_usage(usage_page: u32, usage: u32) -> bool {
    usage_page == HID_USAGE_PAGE_DIGITIZER_U32 && usage == HID_USAGE_DIGITIZER_CONTACT_COUNT
}

pub(super) fn normalize_hid_contact_count(value: libc::c_long) -> usize {
    usize::try_from(value.max(0)).unwrap_or(0)
}

pub(super) fn hid_two_contact_signal_is_active(
    state: &HidSecondaryClickState,
    now_millis: u64,
    max_age_millis: u64,
) -> bool {
    state.contact_count.load(Ordering::SeqCst) >= 2
        || recent_hid_two_contact_seen(state, now_millis, max_age_millis)
}

pub(super) fn recent_hid_two_contact_seen(
    state: &HidSecondaryClickState,
    now_millis: u64,
    max_age_millis: u64,
) -> bool {
    let last_two_contact_millis = state.last_two_contact_millis.load(Ordering::SeqCst);
    last_two_contact_millis > 0
        && now_millis.saturating_sub(last_two_contact_millis) < max_age_millis
}

pub(super) fn should_open_menu_from_hid_button_state(
    secondary_button_was_down: bool,
    secondary_button_is_down: bool,
    cursor_inside_status_view: bool,
) -> bool {
    !secondary_button_was_down && secondary_button_is_down && cursor_inside_status_view
}

pub(super) fn should_open_menu_from_hid_primary_two_contact_state(
    primary_button_was_down: bool,
    primary_button_is_down: bool,
    two_contact_signal_is_active: bool,
    cursor_inside_status_view: bool,
) -> bool {
    !primary_button_was_down
        && primary_button_is_down
        && two_contact_signal_is_active
        && cursor_inside_status_view
}

pub(super) fn hid_elapsed_millis() -> u64 {
    static STARTED_AT: OnceLock<std::time::Instant> = OnceLock::new();
    STARTED_AT
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
