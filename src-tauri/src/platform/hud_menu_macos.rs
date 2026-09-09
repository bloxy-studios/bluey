//! AppKit-only implementation. See docs/NATIVE_HUD_MENUS.md for the source/order audit.

use std::cell::Cell;

use bluey_core::{BlueyError, BlueyResult};
use bluey_protocols::hud_menu::{HudMenuAlign, HudMenuIcon, HudMenuItem, HudMenuRequest};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AllocAnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSColor, NSControlStateValueOff, NSControlStateValueOn, NSForegroundColorAttributeName,
    NSImage, NSMenu, NSMenuItem, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSAttributedString, NSDictionary, NSObject, NSPoint, NSString,
};
use tauri::WebviewWindow;

define_class!(
    // A single registered class; each popup owns its own short-lived instance.
    #[unsafe(super(NSObject))]
    #[name = "BlueyHudMenuTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<Option<usize>>]
    struct HudMenuTarget;

    impl HudMenuTarget {
        #[unsafe(method(chooseHudMenuItem:))]
        fn choose(&self, sender: &NSMenuItem) {
            // Only our NSMenuItems use this selector. No callbacks, IPC, domain actions or
            // borrows across reentrant AppKit tracking. First selection wins; no panic path.
            if self.ivars().get().is_none() && sender.isEnabled() {
                self.ivars().set(usize::try_from(sender.tag()).ok());
            }
        }
    }
);

impl HudMenuTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(None));
        // SAFETY: NSObject's designated init, with initialized Rust ivars.
        unsafe { msg_send![super(this), init] }
    }
}

struct Popup {
    menu: Retained<NSMenu>,
    items: Vec<Retained<NSMenuItem>>,
    target: Retained<HudMenuTarget>,
}

impl Drop for Popup {
    fn drop(&mut self) {
        // Targets are weak. Clear both action and target before releasing our strong owner,
        // including on partial construction/unwind. All drops happen on the main thread.
        for item in &self.items {
            // SAFETY: removing our valid target-action pair; no subsequent dispatch possible.
            unsafe {
                item.setAction(None);
                item.setTarget(None);
            }
        }
        self.menu.removeAllItems();
    }
}

pub(super) fn popup(
    window: &WebviewWindow,
    request: &HudMenuRequest,
) -> BlueyResult<Option<String>> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| BlueyError::internal("HUD menu must run on the main thread"))?;
    let raw_view = window
        .ns_view()
        .map_err(|_| BlueyError::internal("HUD content view is unavailable"))?;
    // SAFETY: Tauri's ns_view is the invoking live window's NSView. Access and retain occur
    // on the main thread before tracking can reenter the event loop. No raw handle escapes.
    let view = unsafe { Retained::retain(raw_view.cast::<NSView>()) }
        .ok_or_else(|| BlueyError::internal("HUD content view is unavailable"))?;
    let owner = view
        .window()
        .ok_or_else(|| BlueyError::internal("HUD content view is detached"))?;
    // A non-activating NSPanel intentionally remains non-key when its toolbar is
    // clicked. Requiring isKeyWindow here would silently discard that first click.
    // Only reject a hidden/stale owner; AppKit can track a menu in a visible
    // non-key panel without us activating the app or making the panel key.
    if !owner.isVisible() {
        return Ok(None);
    }
    let bounds = view.bounds();
    request.validate_view_size(bounds.size.width, bounds.size.height)?;

    let mut popup = Popup {
        menu: NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("")),
        items: Vec::with_capacity(request.items.len()),
        target: HudMenuTarget::new(mtm),
    };
    popup.menu.setAutoenablesItems(false);
    for (index, entry) in request.items.iter().enumerate() {
        let item = match entry {
            HudMenuItem::Separator { .. } => NSMenuItem::separatorItem(mtm),
            // Supported by the app's existing macOS 14 minimum. Native, noninteractive header.
            HudMenuItem::Label { label, .. } => {
                NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(label), mtm)
            }
            HudMenuItem::Item {
                label,
                enabled,
                checked,
                icon,
                destructive,
                ..
            } => {
                let title = NSString::from_str(label);
                // SAFETY: selector signature matches HudMenuTarget::choose. Target is set below
                // before the item is exposed to AppKit and lives until tracking/cleanup finish.
                let item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &title,
                        Some(sel!(chooseHudMenuItem:)),
                        &NSString::from_str(""),
                    )
                };
                item.setTag(index as isize); // validated <=128 items
                item.setEnabled(*enabled);
                if let Some(checked) = checked {
                    item.setState(if *checked {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
                if let Some(icon) = icon {
                    let name = match icon {
                        HudMenuIcon::Manage => "square.grid.2x2",
                        HudMenuIcon::Play => "play",
                        HudMenuIcon::Pause => "pause",
                        HudMenuIcon::Stop => "stop",
                        HudMenuIcon::History => "clock.arrow.circlepath",
                    };
                    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str(name),
                        None,
                    );
                    item.setImage(image.as_deref());
                }
                if *destructive {
                    let color = NSColor::systemRedColor();
                    // SAFETY: AppKit constant is valid on all supported macOS versions; the
                    // foreground-color attribute requires NSColor, which is exactly our value.
                    let title = unsafe {
                        let attributes = NSDictionary::<NSString, AnyObject>::from_slices(
                            &[NSForegroundColorAttributeName],
                            &[&color],
                        );
                        NSAttributedString::initWithString_attributes(
                            NSAttributedString::alloc(),
                            &title,
                            Some(&attributes),
                        )
                    };
                    item.setAttributedTitle(Some(&title));
                }
                // SAFETY: correctly typed target; Popup retains it longer than every item.
                unsafe { item.setTarget(Some(&popup.target)) };
                item
            }
        };
        // Own every item before exposing it to the menu, including partial construction.
        popup.items.push(item.clone());
        popup.menu.addItem(&item);
    }
    let x = bounds.origin.x + request.position.x
        - if request.align == HudMenuAlign::End {
            popup.menu.size().width
        } else {
            0.0
        };
    // Client coordinates are top-left logical points; NSView may be flipped. No DPR/screen math.
    let y = bounds.origin.y
        + if view.isFlipped() {
            request.position.y
        } else {
            bounds.size.height - request.position.y
        };
    // AppKit sends target-action synchronously during this call, unlike Tauri's event-loop
    // proxy. The method returns after tracking finishes, including cancel/outside click.
    let selected = popup.menu.popUpMenuPositioningItem_atLocation_inView(
        None,
        NSPoint::new(x, y),
        Some(&view),
    );
    let id = if selected {
        request.selected_id(popup.target.ivars().get())
    } else {
        None
    };
    // Explicit boundary: disconnect/drop native objects before the oneshot reply or gate release.
    drop(popup);
    drop(owner);
    Ok(id)
}
