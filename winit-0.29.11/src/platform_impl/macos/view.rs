#![allow(clippy::unnecessary_cast)]
use std::boxed::Box;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ptr::NonNull;

use icrate::Foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSCopying, NSMutableAttributedString,
    NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString, NSUInteger,
};
use objc2::declare::{Ivar, IvarDrop};
use objc2::rc::{Id, WeakId};
use objc2::runtime::{AnyObject, Sel};
use objc2::{class, declare_class, msg_send, msg_send_id, mutability, sel, ClassType};

use super::appkit::{
    NSApp, NSCursor, NSEvent, NSEventPhase, NSResponder, NSTextInputClient, NSTrackingRectTag,
    NSView,
};
use crate::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    platform::macos::{OptionAsAlt, WindowExtMacOS},
    platform_impl::platform::{app_state::AppState, util, window::WinitWindow},
    window::WindowId,
};

#[derive(Debug, Default)]
pub struct ViewState {
    tracking_rect: Cell<Option<NSTrackingRectTag>>,

    /// True if the current key event should be forwarded
    /// to the application, even during IME
    forward_key_to_app: Cell<bool>,

    marked_text: RefCell<Id<NSMutableAttributedString>>,
    accepts_first_mouse: bool,
}

declare_class!(
    #[derive(Debug)]
    #[allow(non_snake_case)]
    pub(super) struct WinitView {
        // Weak reference because the window keeps a strong reference to the view
        _ns_window: IvarDrop<Box<WeakId<WinitWindow>>, "__ns_window">,
        state: IvarDrop<Box<ViewState>, "_state">,
    }

    mod ivars;

    unsafe impl ClassType for WinitView {
        #[inherits(NSResponder, NSObject)]
        type Super = NSView;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "WinitView";
    }

    unsafe impl WinitView {
        #[method(initWithId:acceptsFirstMouse:)]
        unsafe fn init_with_id(
            this: *mut Self,
            window: &WinitWindow,
            accepts_first_mouse: bool,
        ) -> Option<NonNull<Self>> {
            let this: Option<&mut Self> = unsafe { msg_send![super(this), init] };
            this.map(|this| {
                let state = ViewState {
                    accepts_first_mouse,
                    ..Default::default()
                };

                Ivar::write(
                    &mut this._ns_window,
                    Box::new(WeakId::new(&window.retain())),
                );
                Ivar::write(&mut this.state, Box::new(state));

                this.setPostsFrameChangedNotifications(true);

                let notification_center: &AnyObject =
                    unsafe { msg_send![class!(NSNotificationCenter), defaultCenter] };
                // About frame change
                let frame_did_change_notification_name =
                    NSString::from_str("NSViewFrameDidChangeNotification");
                #[allow(clippy::let_unit_value)]
                unsafe {
                    let _: () = msg_send![
                        notification_center,
                        addObserver: &*this,
                        selector: sel!(frameDidChange:),
                        name: &*frame_did_change_notification_name,
                        object: &*this,
                    ];
                }

                NonNull::from(this)
            })
        }
    }

    unsafe impl WinitView {
        #[method(viewDidMoveToWindow)]
        fn view_did_move_to_window(&self) {
            trace_scope!("viewDidMoveToWindow");
            if let Some(tracking_rect) = self.state.tracking_rect.take() {
                self.removeTrackingRect(tracking_rect);
            }

            let rect = self.frame();
            let tracking_rect = self.add_tracking_rect(rect, false);
            self.state.tracking_rect.set(Some(tracking_rect));
        }

        #[method(frameDidChange:)]
        fn frame_did_change(&self, _event: &NSEvent) {
            trace_scope!("frameDidChange:");
            if let Some(tracking_rect) = self.state.tracking_rect.take() {
                self.removeTrackingRect(tracking_rect);
            }

            let rect = self.frame();
            let tracking_rect = self.add_tracking_rect(rect, false);
            self.state.tracking_rect.set(Some(tracking_rect));

            // Emit resize event here rather than from windowDidResize because:
            // 1. When a new window is created as a tab, the frame size may change without a window resize occurring.
            // 2. Even when a window resize does occur on a new tabbed window, it contains the wrong size (includes tab height).
            let logical_size = LogicalSize::new(rect.size.width as f64, rect.size.height as f64);
            let size = logical_size.to_physical::<u32>(self.scale_factor());
            self.queue_event(WindowEvent::Resized(size));
        }

        #[method(drawRect:)]
        fn draw_rect(&self, rect: NSRect) {
            trace_scope!("drawRect:");

            // It's a workaround for https://github.com/rust-windowing/winit/issues/2640, don't replace with `self.window_id()`.
            if let Some(window) = self._ns_window.load() {
                AppState::handle_redraw(WindowId(window.id()));
            }

            #[allow(clippy::let_unit_value)]
            unsafe {
                let _: () = msg_send![super(self), drawRect: rect];
            }
        }

        #[method(acceptsFirstResponder)]
        fn accepts_first_responder(&self) -> bool {
            trace_scope!("acceptsFirstResponder");
            true
        }
    }

    unsafe impl NSTextInputClient for WinitView {
        #[method(hasMarkedText)]
        fn has_marked_text(&self) -> bool {
            trace_scope!("hasMarkedText");
            self.state.marked_text.borrow().length() > 0
        }

        #[method(markedRange)]
        fn marked_range(&self) -> NSRange {
            trace_scope!("markedRange");
            let length = self.state.marked_text.borrow().length();
            if length > 0 {
                NSRange::new(0, length)
            } else {
                util::EMPTY_RANGE
            }
        }

        #[method(selectedRange)]
        fn selected_range(&self) -> NSRange {
            trace_scope!("selectedRange");
            util::EMPTY_RANGE
        }

        #[method_id(attributedSubstringForProposedRange:actualRange:)]
        fn attributed_substring_for_proposed_range(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> Option<Id<NSAttributedString>> {
            trace_scope!("attributedSubstringForProposedRange:actualRange:");
            None
        }

        #[method(characterIndexForPoint:)]
        fn character_index_for_point(&self, _point: NSPoint) -> NSUInteger {
            trace_scope!("characterIndexForPoint:");
            0
        }
    }
);

impl WinitView {
    pub(super) fn new(window: &WinitWindow, accepts_first_mouse: bool) -> Id<Self> {
        unsafe {
            msg_send_id![
                Self::alloc(),
                initWithId: window,
                acceptsFirstMouse: accepts_first_mouse,
            ]
        }
    }

    fn window(&self) -> Id<WinitWindow> {
        // TODO: Simply use `window` property on `NSView`.
        // That only returns a window _after_ the view has been attached though!
        // (which is incompatible with `frameDidChange:`)
        //
        // unsafe { msg_send_id![self, window] }
        self._ns_window.load().expect("view to have a window")
    }

    fn window_id(&self) -> WindowId {
        WindowId(self.window().id())
    }

    fn queue_event(&self, event: WindowEvent) {
        let event = Event::WindowEvent {
            window_id: self.window_id(),
            event,
        };
        AppState::queue_event(event);
    }

    fn scale_factor(&self) -> f64 {
        self.window().backingScaleFactor() as f64
    }
}
