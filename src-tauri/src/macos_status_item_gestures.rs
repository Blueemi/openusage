use objc2::{ClassType, runtime::AnyObject};
use objc2_app_kit::{NSClickGestureRecognizer, NSGestureRecognizer, NSView};
use objc2_foundation::{MainThreadMarker, NSInteger, NSUInteger};

pub(crate) fn install_context_click_gestures(view: &NSView, target: &AnyObject) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("tray context menu: cannot install status view gestures off main thread");
        return;
    };

    for spec in context_click_gesture_specs() {
        let click = NSClickGestureRecognizer::new(mtm);
        click.setButtonMask(spec.button_mask);
        click.setNumberOfClicksRequired(1);
        click.setNumberOfTouchesRequired(spec.touch_count);

        let gesture: &NSGestureRecognizer = click.as_super();
        unsafe {
            gesture.setTarget(Some(target));
            gesture.setAction(Some(objc2::sel!(openOpenUsageContextMenuFromGesture:)));
        }
        gesture.setDelaysPrimaryMouseButtonEvents(spec.delay_primary);
        gesture.setDelaysSecondaryMouseButtonEvents(spec.delay_secondary);
        gesture.setDelaysOtherMouseButtonEvents(false);
        view.addGestureRecognizer(gesture);
    }

    log::warn!("tray context menu: installed custom status view click gestures");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContextClickGestureSpec {
    button_mask: NSUInteger,
    touch_count: NSInteger,
    delay_primary: bool,
    delay_secondary: bool,
}

fn context_click_gesture_specs() -> [ContextClickGestureSpec; 3] {
    [
        ContextClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 1,
            delay_primary: false,
            delay_secondary: true,
        },
        ContextClickGestureSpec {
            button_mask: primary_click_button_mask(),
            touch_count: 2,
            delay_primary: true,
            delay_secondary: false,
        },
        ContextClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 2,
            delay_primary: false,
            delay_secondary: true,
        },
    ]
}

fn primary_click_button_mask() -> NSUInteger {
    1 << 0
}

fn secondary_click_button_mask() -> NSUInteger {
    1 << 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gestures_include_two_finger_clicks() {
        let specs = context_click_gesture_specs();

        assert!(specs.contains(&ContextClickGestureSpec {
            button_mask: primary_click_button_mask(),
            touch_count: 2,
            delay_primary: true,
            delay_secondary: false,
        }));
        assert!(specs.contains(&ContextClickGestureSpec {
            button_mask: secondary_click_button_mask(),
            touch_count: 2,
            delay_primary: false,
            delay_secondary: true,
        }));
    }
}
