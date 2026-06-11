use objc2::{AnyThread as _, ClassType as _, Message};
use objc2_app_kit::{
    NSButton, NSImage, NSImageScaling, NSImageView, NSStatusBarButton, NSStatusItem, NSView,
};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

pub(crate) const STATUS_ITEM_WIDTH: f64 = 24.0;
pub(crate) const STATUS_ITEM_MENU_BAR_HIT_HEIGHT: f64 = 30.0;
const STATUS_ITEM_ICON_SIZE: f64 = 18.0;
const STATUS_ITEM_HORIZONTAL_PADDING: f64 = 6.0;

static CUSTOM_STATUS_VIEW_INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static CUSTOM_STATUS_IMAGE_VIEW: RefCell<Option<objc2::rc::Retained<NSImageView>>> =
        RefCell::new(None);
    static CUSTOM_STATUS_ITEM: RefCell<Option<objc2::rc::Retained<NSStatusItem>>> =
        RefCell::new(None);
    static CUSTOM_STATUS_VIEW: RefCell<Option<objc2::rc::Retained<NSView>>> =
        RefCell::new(None);
    static NATIVE_STATUS_BUTTON: RefCell<Option<objc2::rc::Retained<NSStatusBarButton>>> =
        RefCell::new(None);
}

#[allow(dead_code)]
pub(crate) fn install(status_item: &NSStatusItem, status_view: &NSView, image_view: &NSImageView) {
    CUSTOM_STATUS_VIEW_INSTALLED.store(true, Ordering::SeqCst);
    CUSTOM_STATUS_IMAGE_VIEW.with(|slot| {
        *slot.borrow_mut() = Some(image_view.retain());
    });
    CUSTOM_STATUS_ITEM.with(|slot| {
        *slot.borrow_mut() = Some(status_item.retain());
    });
    CUSTOM_STATUS_VIEW.with(|slot| {
        *slot.borrow_mut() = Some(status_view.retain());
    });
    NATIVE_STATUS_BUTTON.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub(crate) fn install_native_button(status_item: &NSStatusItem, button: &NSStatusBarButton) {
    CUSTOM_STATUS_VIEW_INSTALLED.store(true, Ordering::SeqCst);
    CUSTOM_STATUS_IMAGE_VIEW.with(|slot| {
        *slot.borrow_mut() = None;
    });
    CUSTOM_STATUS_ITEM.with(|slot| {
        *slot.borrow_mut() = Some(status_item.retain());
    });
    CUSTOM_STATUS_VIEW.with(|slot| {
        *slot.borrow_mut() = Some(button.as_super().as_super().as_super().retain());
    });
    NATIVE_STATUS_BUTTON.with(|slot| {
        *slot.borrow_mut() = Some(button.retain());
    });
    log::warn!("tray context menu: using native status bar button");
}

#[allow(dead_code)]
pub(crate) fn initial_image_frame(size: NSSize) -> NSRect {
    status_item_image_frame_for_icon_size(
        size,
        NSSize::new(
            STATUS_ITEM_ICON_SIZE.min(size.width),
            STATUS_ITEM_ICON_SIZE.min(size.height),
        ),
    )
}

pub(crate) fn set_icon_rgba(
    app_handle: &AppHandle,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    is_template: bool,
) -> Result<bool, String> {
    if !CUSTOM_STATUS_VIEW_INSTALLED.load(Ordering::SeqCst) {
        return Ok(false);
    }
    validate_rgba_image(&rgba, width, height)?;

    app_handle
        .run_on_main_thread(move || {
            if let Err(error) = set_icon_rgba_on_main(rgba, width, height, is_template) {
                log::error!("failed to update custom macOS tray icon: {error}");
            }
        })
        .map_err(|error| format!("failed to schedule tray icon update: {error}"))?;

    Ok(true)
}

fn set_icon_rgba_on_main(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    is_template: bool,
) -> Result<(), String> {
    let png = encode_png_rgba(&rgba, width, height)?;
    let data = NSData::from_vec(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "failed to create NSImage from tray icon data".to_string())?;
    let image_size = status_item_image_display_size(width, height);
    image.setSize(image_size);
    image.setTemplate(is_template);

    CUSTOM_STATUS_IMAGE_VIEW.with(|image_view_slot| {
        NATIVE_STATUS_BUTTON.with(|button_slot| {
            CUSTOM_STATUS_ITEM.with(|status_item_slot| {
                CUSTOM_STATUS_VIEW.with(|status_view_slot| {
                    let image_view = image_view_slot.borrow();
                    let button = button_slot.borrow();
                    let status_item = status_item_slot.borrow();
                    let status_view = status_view_slot.borrow();
                    let Some(status_item) = status_item.as_ref() else {
                        return Err("custom status item missing".to_string());
                    };
                    let Some(status_view) = status_view.as_ref() else {
                        return Err("custom status view missing".to_string());
                    };

                    let item_size = status_item_size_for_icon_size(image_size);
                    status_item.setLength(item_size.width);
                    if let Some(image_view) = image_view.as_ref() {
                        status_view.setFrameSize(item_size);
                        status_view.setBoundsSize(item_size);
                        image_view
                            .as_super()
                            .as_super()
                            .setFrame(status_item_image_frame_for_icon_size(item_size, image_size));
                        image_view.setImage(Some(&image));
                    } else if let Some(button) = button.as_ref() {
                        let button: &NSButton = button.as_super();
                        button.setImageScaling(NSImageScaling::ScaleProportionallyDown);
                        button.setImage(Some(&image));
                    } else {
                        return Err("status icon target missing".to_string());
                    }
                    crate::tray::update_native_tray_rect_from_view(status_view);
                    Ok(())
                })
            })
        })
    })
}

fn status_item_image_frame_for_icon_size(size: NSSize, icon_size: NSSize) -> NSRect {
    NSRect::new(
        NSPoint::new(
            (size.width - icon_size.width) / 2.0,
            (size.height - icon_size.height) / 2.0,
        ),
        icon_size,
    )
}

fn status_item_image_display_size(width: u32, height: u32) -> NSSize {
    let width = f64::from(width);
    let height = f64::from(height);
    if width <= 0.0 || height <= 0.0 {
        return NSSize::new(STATUS_ITEM_ICON_SIZE, STATUS_ITEM_ICON_SIZE);
    }

    NSSize::new(
        width / height * STATUS_ITEM_ICON_SIZE,
        STATUS_ITEM_ICON_SIZE,
    )
}

fn status_item_size_for_icon_size(icon_size: NSSize) -> NSSize {
    NSSize::new(
        (icon_size.width + STATUS_ITEM_HORIZONTAL_PADDING).max(STATUS_ITEM_WIDTH),
        STATUS_ITEM_MENU_BAR_HIT_HEIGHT.max(icon_size.height),
    )
}

fn validate_rgba_image(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("tray icon size must be non-zero".to_string());
    }

    let expected_len = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "tray icon size is too large".to_string())?;
    if rgba.len() != expected_len {
        return Err(format!(
            "tray icon rgba length mismatch: got {}, expected {}",
            rgba.len(),
            expected_len
        ));
    }

    Ok(())
}

fn encode_png_rgba(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut png_bytes), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("failed to write tray icon png header: {error}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|error| format!("failed to write tray icon png data: {error}"))?;
    }
    Ok(png_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_status_view_uses_stable_icon_frame() {
        let frame = initial_image_frame(NSSize::new(24.0, 30.0));
        assert_eq!(frame.origin.x, 3.0);
        assert_eq!(frame.origin.y, 6.0);
        assert_eq!(frame.size.width, 18.0);
        assert_eq!(frame.size.height, 18.0);
    }

    #[test]
    fn custom_status_view_resizes_wide_icons() {
        let image_size = status_item_image_display_size(84, 36);
        let item_size = status_item_size_for_icon_size(image_size);
        let frame = status_item_image_frame_for_icon_size(item_size, image_size);

        assert_eq!(image_size.height, 18.0);
        assert_eq!(image_size.width, 42.0);
        assert_eq!(item_size.width, 48.0);
        assert_eq!(item_size.height, 30.0);
        assert_eq!(frame.origin.x, 3.0);
        assert_eq!(frame.origin.y, 6.0);
    }

    #[test]
    fn custom_status_view_rejects_bad_rgba_size() {
        assert!(validate_rgba_image(&[0, 0, 0], 1, 1).is_err());
        assert!(validate_rgba_image(&[0, 0, 0, 0], 1, 1).is_ok());
    }
}
