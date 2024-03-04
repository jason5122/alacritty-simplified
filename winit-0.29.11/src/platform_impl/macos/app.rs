#![allow(clippy::unnecessary_cast)]

use icrate::Foundation::NSObject;
use objc2::{declare_class, msg_send, mutability, ClassType};

use super::app_state::AppState;
use super::appkit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSResponder};
use crate::event::Event;

declare_class!(
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub(super) struct WinitApplication;

    unsafe impl ClassType for WinitApplication {
        #[inherits(NSResponder, NSObject)]
        type Super = NSApplication;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "WinitApplication";
    }

    unsafe impl WinitApplication {
        // Normally, holding Cmd + any key never sends us a `keyUp` event for that key.
        // Overriding `sendEvent:` like this fixes that. (https://stackoverflow.com/a/15294196)
        // Fun fact: Firefox still has this bug! (https://bugzilla.mozilla.org/show_bug.cgi?id=1299553)
        #[method(sendEvent:)]
        fn send_event(&self, event: &NSEvent) {
            // For posterity, there are some undocumented event types
            // (https://github.com/servo/cocoa-rs/issues/155)
            // but that doesn't really matter here.
            let event_type = event.type_();
            let modifier_flags = event.modifierFlags();
            if event_type == NSEventType::NSKeyUp
                && modifier_flags.contains(NSEventModifierFlags::NSCommandKeyMask)
            {
                if let Some(key_window) = self.keyWindow() {
                    unsafe { key_window.sendEvent(event) };
                }
            }
        }
    }
);
