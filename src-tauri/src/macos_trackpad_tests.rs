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
    assert_eq!(normalize_raw_active_fingers(-1), 0);
    assert!(should_log_raw_callback_frame(1, 0, 0));
    assert!(should_log_raw_callback_frame(2, 1, 2));
    assert!(!should_log_raw_callback_frame(2, 2, 2));
}

#[test]
fn raw_trackpad_path_states_track_active_fingers() {
    assert!(raw_trackpad_path_state_is_active(
        RAW_TRACKPAD_PATH_STATE_START_IN_RANGE
    ));
    assert!(raw_trackpad_path_state_is_active(
        RAW_TRACKPAD_PATH_STATE_TOUCHING
    ));
    assert!(!raw_trackpad_path_state_is_active(5));
    assert_eq!(raw_trackpad_path_bit(0), Some(1));
    assert_eq!(raw_trackpad_path_bit(2), Some(4));
    assert_eq!(raw_trackpad_path_bit(-1), None);
    assert_eq!(raw_trackpad_path_bit(64), None);
}

#[test]
fn raw_trackpad_path_callback_accepts_current_multitouch_abi() {
    unsafe {
        raw_trackpad_path_callback(
            std::ptr::null_mut(),
            0,
            RAW_TRACKPAD_PATH_STATE_TOUCHING,
            std::ptr::null_mut(),
        );
    }
}

#[test]
fn raw_trackpad_callback_accepts_current_multitouch_abi() {
    unsafe {
        raw_trackpad_contact_callback(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0.0, 0);
    }
}
