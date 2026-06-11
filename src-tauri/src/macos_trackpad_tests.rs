use super::*;
use std::sync::atomic::Ordering;

#[test]
fn raw_trackpad_menu_opens_when_two_finger_sequence_ends_inside_status_view() {
    assert!(should_open_raw_trackpad_menu_on_sequence_end(
        true, false, true, false
    ));
    assert!(!should_open_raw_trackpad_menu_on_sequence_end(
        false, true, true, false
    ));
    assert!(!should_open_raw_trackpad_menu_on_sequence_end(
        true, true, true, false
    ));
    assert!(!should_open_raw_trackpad_menu_on_sequence_end(
        true, false, false, false
    ));
    assert!(!should_open_raw_trackpad_menu_on_sequence_end(
        true, false, true, true
    ));
}

#[test]
fn raw_trackpad_menu_opens_while_two_fingers_are_active_inside_status_view() {
    assert!(should_open_raw_trackpad_menu_while_sequence_active(
        true, true, false
    ));
    assert!(!should_open_raw_trackpad_menu_while_sequence_active(
        true, true, true
    ));
    assert!(!should_open_raw_trackpad_menu_while_sequence_active(
        true, false, false
    ));
    assert!(!should_open_raw_trackpad_menu_while_sequence_active(
        false, true, false
    ));
}

#[test]
fn stale_raw_trackpad_update_ends_two_finger_sequence() {
    assert!(!raw_trackpad_update_is_stale(100, 0, 160));
    assert!(!raw_trackpad_update_is_stale(259, 100, 160));
    assert!(raw_trackpad_update_is_stale(260, 100, 160));
}

#[test]
fn recent_raw_two_finger_seen_covers_fast_taps() {
    let state = RawTrackpadTouchState::new();
    assert!(!recent_raw_two_finger_seen(&state, 100));

    state.last_two_finger_millis.store(100, Ordering::SeqCst);
    assert!(recent_raw_two_finger_seen(&state, 399));
    assert!(!recent_raw_two_finger_seen(&state, 400));
}

#[test]
fn raw_trackpad_callback_logs_first_frame_and_finger_changes() {
    assert_eq!(normalize_raw_active_fingers(2), 2);
    assert!(should_log_raw_callback_frame(1, 0, 0));
    assert!(should_log_raw_callback_frame(2, 1, 2));
    assert!(!should_log_raw_callback_frame(2, 2, 2));
}

#[test]
fn raw_trackpad_callback_returns_success_for_multitouch_runtime() {
    let status = unsafe {
        raw_trackpad_contact_callback(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0.0, 0)
    };

    assert_eq!(status, 0);
}
