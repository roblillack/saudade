//! macOS: a menu or dropdown belongs to the window it drops from, and a dialog
//! to the window it opens over.
//!
//! saudade hosts every popup in a window of its own, and to AppKit a winit
//! window is one like any other: unrelated to the window it drops from, and
//! taking the keyboard when it is shown. Three things followed from that:
//!
//! * Opening a menu took the keyboard from the window it belongs to, whose
//!   title bar went inactive for as long as the menu was up.
//! * A menu stayed where it was when that window moved, until `windowDidMove`
//!   reached the runtime and it was put back in place.
//! * Sharing that window in a video call left the menu out. ScreenCaptureKit's
//!   single-window capture, which Chrome shares a window with, takes in the
//!   window's child windows, even where they hang off its edge, but no other
//!   window of the app lying on top of it.
//!
//! [`attach_popup`] makes the popup a child window of the one it drops from,
//! which the window server then carries along with it, and which capture counts
//! as part of it. And the popup leaves the keyboard with that window, as a
//! native menu does. winit's window class says yes to any window asking to
//! become key or main, and a winit window has had its class swapped by
//! key-value observing before we ever see it, so it can't be given a subclass
//! of its own: the answer is changed on winit's class instead, to no for a
//! popup. The keyboard stays with the window beneath, and reaches the widget
//! tree from there, as it does on X11 and Windows.
//!
//! A window that isn't key can't count on mouse-moved events: AppKit sends them
//! to a window of the app that need not be the one under the pointer. So the
//! popup gets a tracking area of its own that asks for them.
//!
//! That leaves the runtime to learn some other way what the popup losing the
//! keyboard used to tell it — that the user is done with the menu. Another app
//! or another window of this one taking over still shows, as the window beneath
//! losing the keyboard instead. A click on that window's own title bar doesn't
//! move the keyboard anywhere, so an event monitor catches it there.
//!
//! A dialog was a window of its own as well: left out when the window it opens
//! over was shared, put behind that window when that one was clicked, and left
//! behind when it moved. [`attach_dialog`] makes it a child window of that
//! window too, which keeps it above it and has it move along and be shared
//! with it. Unlike a popup it takes the keyboard, as any window with a title
//! bar does. winit centers a window it is given no position for on its
//! display, so [`center_over`] puts a dialog over its window instead.

use std::cell::OnceCell;
use std::mem;
use std::ptr::NonNull;
use std::sync::OnceLock;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, NSObjectProtocol, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, ffi, sel};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow,
    NSWindowDelegate, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGRect};
use winit::window::Window;

use crate::app::ns_window;

/// What winit's window class answered `canBecomeKeyWindow` and
/// `canBecomeMainWindow` with, in that order, before [`leave_keyboard_with_parent`]
/// took the two over. Set once, whether that worked or not.
static ORIGINAL: OnceLock<[Option<Imp>; 2]> = OnceLock::new();

thread_local! {
    /// The event monitor [`dismiss_popups_on_frame_click`] installs, for as
    /// long as the app runs.
    static FRAME_CLICK_MONITOR: OnceCell<Option<Retained<AnyObject>>> = const { OnceCell::new() };
}

/// Make the popup `win` a child window of `parent`, the window it drops from,
/// that leaves the keyboard with it. Adding a hidden window as a child shows
/// it, so this is called once, when the popup is ready to be seen — and before
/// winit's `set_visible`, which shows a window by making it key.
///
/// AppKit gives a child its parent's window level, which is fine: it keeps it
/// ordered above the parent. Closing the popup takes it off the parent again.
/// A window that can't be reached is left as it was.
pub(crate) fn attach_popup(win: &Window, parent: &Window) {
    let (Some(window), Some(parent)) = (ns_window(win), ns_window(parent)) else {
        return;
    };
    let mtm = window.mtm();
    leave_keyboard_with_parent();
    dismiss_popups_on_frame_click(mtm);
    if let Some(view) = window.contentView() {
        track_mouse_moves(&view);
    }
    add_child(&parent, &window);
}

/// Make the dialog `win` a child window of `parent`, the window it opens over.
/// As with [`attach_popup`], this shows it, and is called once, when the dialog
/// is ready to be seen.
pub(crate) fn attach_dialog(win: &Window, parent: &Window) {
    if let (Some(window), Some(parent)) = (ns_window(win), ns_window(parent)) {
        add_child(&parent, &window);
    }
}

fn add_child(parent: &NSWindow, child: &NSWindow) {
    // SAFETY: two live windows of this thread, and an ordering mode AppKit
    // defines.
    unsafe { parent.addChildWindow_ordered(child, NSWindowOrderingMode::Above) };
}

/// Move the dialog `win` to the middle of `parent`, kept within the part of the
/// display `parent` is on that the menu bar and the Dock leave free, so its
/// title bar can't land under either or off the screen.
pub(crate) fn center_over(win: &Window, parent: &Window) {
    let (Some(window), Some(parent)) = (ns_window(win), ns_window(parent)) else {
        return;
    };
    let over = parent.frame();
    let size = window.frame().size;
    let mut x = over.origin.x + (over.size.width - size.width) / 2.0;
    let mut y = over.origin.y + (over.size.height - size.height) / 2.0;
    if let Some(screen) = parent.screen() {
        // In points, with y growing upwards: the top edge, the title bar, is
        // the one kept on screen where both can't be.
        let free = screen.visibleFrame();
        x = x
            .min(free.origin.x + free.size.width - size.width)
            .max(free.origin.x);
        y = y
            .max(free.origin.y)
            .min(free.origin.y + free.size.height - size.height);
    }
    window.setFrameOrigin(CGPoint::new(x.round(), y.round()));
}

/// Whether `window` is a popup: a child window without a title bar. A dialog is
/// a child window too, but has one, and takes the keyboard as any window with
/// a title bar does.
fn is_popup(window: &NSWindow) -> bool {
    window.parentWindow().is_some() && !window.styleMask().contains(NSWindowStyleMask::Titled)
}

/// Have winit's windows refuse to become key or main while they are popups
/// [`attach_popup`]ed to the window they drop from, and answer as before
/// otherwise. Done once; if winit's window class can't be found, a popup takes
/// the keyboard as it would without.
fn leave_keyboard_with_parent() {
    ORIGINAL.get_or_init(|| {
        let Some(class) = AnyClass::get(c"WinitWindow") else {
            return [None, None];
        };
        let replace = |sel: Sel, imp: Imp| {
            let method = class.instance_method(sel)?;
            let inherited = method.implementation();
            // SAFETY: `imp` takes and returns what the method it replaces
            // does, with the same type encoding, and answers the same for
            // every window but a popup. Replacing it on winit's class leaves
            // `NSWindow`'s own alone. `None` back means winit's class only
            // inherited the method, whose implementation then is the original.
            let previous = unsafe {
                ffi::class_replaceMethod(
                    (class as *const AnyClass).cast_mut(),
                    sel,
                    imp,
                    ffi::method_getTypeEncoding(method),
                )
            };
            Some(previous.unwrap_or(inherited))
        };
        type Answer = unsafe extern "C-unwind" fn(&NSWindow, Sel) -> Bool;
        // SAFETY: function pointers of one ABI, as the runtime stores them.
        let (key, main) = unsafe {
            (
                mem::transmute::<Answer, Imp>(can_become_key_window),
                mem::transmute::<Answer, Imp>(can_become_main_window),
            )
        };
        [
            replace(sel!(canBecomeKeyWindow), key),
            replace(sel!(canBecomeMainWindow), main),
        ]
    });
}

unsafe extern "C-unwind" fn can_become_key_window(this: &NSWindow, cmd: Sel) -> Bool {
    // SAFETY: called by the runtime as the method it replaced.
    unsafe { answer(this, cmd, 0) }
}

unsafe extern "C-unwind" fn can_become_main_window(this: &NSWindow, cmd: Sel) -> Bool {
    // SAFETY: as above.
    unsafe { answer(this, cmd, 1) }
}

/// No for a popup, and the original answer — the one at `idx` in [`ORIGINAL`] —
/// for any other window.
///
/// # Safety
///
/// Called with the arguments the method being answered for was sent.
unsafe fn answer(this: &NSWindow, cmd: Sel, idx: usize) -> Bool {
    if is_popup(this) {
        return Bool::NO;
    }
    match ORIGINAL.get().and_then(|original| original[idx]) {
        Some(imp) => {
            // SAFETY: the implementation this one replaced, called as it was
            // called.
            unsafe {
                let imp =
                    mem::transmute::<Imp, unsafe extern "C-unwind" fn(&NSWindow, Sel) -> Bool>(imp);
                imp(this, cmd)
            }
        }
        // winit's own answer.
        None => Bool::YES,
    }
}

/// Have `view` sent mouse-moved events while the pointer is over it and the app
/// is active, which AppKit can't be counted on to do for a window that isn't
/// key.
fn track_mouse_moves(view: &NSView) {
    let options = NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveInActiveApp
        | NSTrackingAreaOptions::InVisibleRect;
    // SAFETY: the owner is the view the area is added to, which the area holds
    // weakly. The rect is ignored for `InVisibleRect`, which follows the view
    // as it is resized.
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            CGRect::ZERO,
            options,
            Some(view),
            None,
        )
    };
    view.addTrackingArea(&area);
}

/// Have a click on a window's title bar or frame close the popups it carries —
/// not its dialogs, which stay open as they would over any other click.
/// They are asked to close, as by their close button, which the runtime answers
/// for a popup as it does Escape. Installed once, for every window of the app.
fn dismiss_popups_on_frame_click(mtm: MainThreadMarker) {
    FRAME_CLICK_MONITOR.with(|monitor| {
        monitor.get_or_init(|| {
            let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
                // SAFETY: AppKit hands the handler a live event.
                close_popups_on_frame_click(unsafe { event.as_ref() }, mtm);
                event.as_ptr()
            });
            let mask = NSEventMask::LeftMouseDown
                | NSEventMask::RightMouseDown
                | NSEventMask::OtherMouseDown;
            // SAFETY: the handler returns the event it was handed, unchanged.
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &handler) }
        });
    });
}

fn close_popups_on_frame_click(event: &NSEvent, mtm: MainThreadMarker) {
    let Some(window) = event.window(mtm) else {
        return;
    };
    let Some(popups) = window.childWindows() else {
        return;
    };
    // A click in the content area reaches the widget tree, which tells for
    // itself whether it landed outside the menu.
    let at = event.locationInWindow();
    let content = window.contentLayoutRect();
    if at.x >= content.origin.x
        && at.y >= content.origin.y
        && at.x < content.origin.x + content.size.width
        && at.y < content.origin.y + content.size.height
    {
        return;
    }
    // Before the press is handled: on the title bar that is the start of a
    // window move, which the popup is not to come along on.
    for popup in popups.to_vec().into_iter().filter(|child| is_popup(child)) {
        if let Some(delegate) = popup.delegate()
            && delegate.respondsToSelector(sel!(windowShouldClose:))
        {
            delegate.windowShouldClose(&popup);
        }
    }
}
