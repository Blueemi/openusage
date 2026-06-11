use super::*;
use std::sync::atomic::Ordering;

#[test]
fn hid_usage_helpers_match_mouse_buttons_and_contact_count() {
    assert!(is_secondary_button_hid_usage(0x09, 0x02));
    assert!(!is_secondary_button_hid_usage(0x09, 0x01));
    assert!(!is_secondary_button_hid_usage(0x01, 0x02));
    assert!(is_contact_count_hid_usage(0x0d, 0x54));
    assert!(!is_contact_count_hid_usage(0x0d, 0x42));
}

#[test]
fn hid_button_down_edge_opens_only_inside_status_view() {
    assert!(should_open_menu_from_hid_button_state(false, true, true));
    assert!(!should_open_menu_from_hid_button_state(true, true, true));
    assert!(!should_open_menu_from_hid_button_state(false, false, true));
    assert!(!should_open_menu_from_hid_button_state(false, true, false));
}

#[test]
fn hid_primary_button_with_two_contacts_opens_only_on_inside_down_edge() {
    assert!(should_open_menu_from_hid_primary_two_contact_state(
        false, true, true, true
    ));
    assert!(!should_open_menu_from_hid_primary_two_contact_state(
        true, true, true, true
    ));
    assert!(!should_open_menu_from_hid_primary_two_contact_state(
        false, false, true, true
    ));
    assert!(!should_open_menu_from_hid_primary_two_contact_state(
        false, true, false, true
    ));
    assert!(!should_open_menu_from_hid_primary_two_contact_state(
        false, true, true, false
    ));
}

#[test]
fn recent_hid_two_contact_signal_covers_button_event_ordering() {
    let state = HidSecondaryClickState::new();
    assert!(!hid_two_contact_signal_is_active(&state, 100, 240));

    state.contact_count.store(2, Ordering::SeqCst);
    assert!(hid_two_contact_signal_is_active(&state, 100, 240));

    state.contact_count.store(0, Ordering::SeqCst);
    state.last_two_contact_millis.store(100, Ordering::SeqCst);
    assert!(hid_two_contact_signal_is_active(&state, 339, 240));
    assert!(!hid_two_contact_signal_is_active(&state, 340, 240));
}

#[test]
fn hid_contact_count_normalizes_negative_values() {
    assert_eq!(normalize_hid_contact_count(2), 2);
    assert_eq!(normalize_hid_contact_count(-1), 0);
}

#[test]
fn iohid_zero_status_is_success() {
    assert!(iohid_status_is_success(0));
    assert!(!iohid_status_is_success(1));
}
