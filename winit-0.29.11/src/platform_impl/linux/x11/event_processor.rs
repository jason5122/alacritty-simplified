use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::os::raw::{c_char, c_int, c_long, c_ulong};
use std::slice;
use std::sync::{Arc, Mutex};

use x11_dl::xinput2::{
    self, XIDeviceEvent, XIEnterEvent, XIFocusInEvent, XIFocusOutEvent, XIHierarchyEvent,
    XILeaveEvent, XIModifierState, XIRawEvent,
};
use x11_dl::xlib::{
    self, Display as XDisplay, Window as XWindow, XAnyEvent, XClientMessageEvent, XConfigureEvent,
    XDestroyWindowEvent, XEvent, XExposeEvent, XKeyEvent, XMapEvent, XPropertyEvent,
    XReparentEvent, XSelectionEvent, XVisibilityEvent, XkbAnyEvent, XkbStateRec,
};
use x11rb::protocol::xinput;
use x11rb::protocol::xkb::ID as XkbId;
use x11rb::protocol::xproto::{self, ConnectionExt as _, ModMask};
use x11rb::x11_utils::ExtensionInformation;
use x11rb::x11_utils::Serialize;
use xkbcommon_dl::xkb_mod_mask_t;

use crate::dpi::{PhysicalPosition, PhysicalSize};
use crate::event::InnerSizeWriter;
use crate::event::{Event, WindowEvent};
use crate::event_loop::EventLoopWindowTarget as RootELW;
use crate::platform_impl::platform::x11::EventLoopWindowTarget;
use crate::platform_impl::platform::EventLoopWindowTarget as PlatformEventLoopWindowTarget;
use crate::platform_impl::x11::{
    atoms::*, mkwid, util, CookieResultExt, Device, DeviceId, DeviceInfo, Dnd, DndState,
    GenericEventCookie, ScrollOrientation, UnownedWindow, WindowId,
};

/// The maximum amount of X modifiers to replay.
pub const MAX_MOD_REPLAY_LEN: usize = 32;

/// The X11 documentation states: "Keycodes lie in the inclusive range `[8, 255]`".
const KEYCODE_OFFSET: u8 = 8;

pub struct EventProcessor<T: 'static> {
    pub dnd: Dnd,
    pub randr_event_offset: u8,
    pub devices: RefCell<HashMap<DeviceId, Device>>,
    pub xi2ext: ExtensionInformation,
    pub xkbext: ExtensionInformation,
    pub target: RootELW<T>,
    // Currently focused window belonging to this process
    pub active_window: Option<xproto::Window>,
    pub is_composing: bool,
}

impl<T: 'static> EventProcessor<T> {
    pub fn process_event<F>(&mut self, xev: &mut XEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let window_target = Self::window_target_mut(&mut self.target);
    }

    /// XFilterEvent tells us when an event has been discarded by the input method.
    /// Specifically, this involves all of the KeyPress events in compose/pre-edit sequences,
    /// along with an extra copy of the KeyRelease events. This also prevents backspace and
    /// arrow keys from being detected twice.
    fn filter_event(&mut self, xev: &mut XEvent) -> bool {
        let wt = Self::window_target(&self.target);
        unsafe {
            (wt.xconn.xlib.XFilterEvent)(xev, {
                let xev: &XAnyEvent = xev.as_ref();
                xev.window
            }) == xlib::True
        }
    }

    pub fn poll(&self) -> bool {
        let window_target = Self::window_target(&self.target);
        let result = unsafe { (window_target.xconn.xlib.XPending)(window_target.xconn.display) };

        result != 0
    }

    pub unsafe fn poll_one_event(&mut self, event_ptr: *mut XEvent) -> bool {
        let window_target = Self::window_target(&self.target);
        // This function is used to poll and remove a single event
        // from the Xlib event queue in a non-blocking, atomic way.
        // XCheckIfEvent is non-blocking and removes events from queue.
        // XNextEvent can't be used because it blocks while holding the
        // global Xlib mutex.
        // XPeekEvent does not remove events from the queue.
        unsafe extern "C" fn predicate(
            _display: *mut XDisplay,
            _event: *mut XEvent,
            _arg: *mut c_char,
        ) -> c_int {
            // This predicate always returns "true" (1) to accept all events
            1
        }

        let result = unsafe {
            (window_target.xconn.xlib.XCheckIfEvent)(
                window_target.xconn.display,
                event_ptr,
                Some(predicate),
                std::ptr::null_mut(),
            )
        };

        result != 0
    }

    pub fn init_device(&self, device: xinput::DeviceId) {
        let window_target = Self::window_target(&self.target);
        let mut devices = self.devices.borrow_mut();
        if let Some(info) = DeviceInfo::get(&window_target.xconn, device as _) {
            for info in info.iter() {
                devices.insert(DeviceId(info.deviceid as _), Device::new(info));
            }
        }
    }

    pub fn with_window<F, Ret>(&self, window_id: xproto::Window, callback: F) -> Option<Ret>
    where
        F: Fn(&Arc<UnownedWindow>) -> Ret,
    {
        let mut deleted = false;
        let window_id = WindowId(window_id as _);
        let window_target = Self::window_target(&self.target);
        let result = window_target
            .windows
            .borrow()
            .get(&window_id)
            .and_then(|window| {
                let arc = window.upgrade();
                deleted = arc.is_none();
                arc
            })
            .map(|window| callback(&window));

        if deleted {
            // Garbage collection
            window_target.windows.borrow_mut().remove(&window_id);
        }

        result
    }

    // NOTE: we avoid `self` to not borrow the entire `self` as not mut.
    /// Get the platform window target.
    pub fn window_target(window_target: &RootELW<T>) -> &EventLoopWindowTarget<T> {
        match &window_target.p {
            PlatformEventLoopWindowTarget::X(target) => target,
            #[cfg(wayland_platform)]
            _ => unreachable!(),
        }
    }

    /// Get the platform window target.
    pub fn window_target_mut(window_target: &mut RootELW<T>) -> &mut EventLoopWindowTarget<T> {
        match &mut window_target.p {
            PlatformEventLoopWindowTarget::X(target) => target,
            #[cfg(wayland_platform)]
            _ => unreachable!(),
        }
    }

    fn client_message<F>(&mut self, xev: &XClientMessageEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);
        let atoms = wt.xconn.atoms();

        let window = xev.window as xproto::Window;
        let window_id = mkwid(window);

        if xev.data.get_long(0) as xproto::Atom == wt.wm_delete_window {
            let event = Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
            };
            callback(&self.target, event);
            return;
        }

        if xev.data.get_long(0) as xproto::Atom == wt.net_wm_ping {
            let client_msg = xproto::ClientMessageEvent {
                response_type: xproto::CLIENT_MESSAGE_EVENT,
                format: xev.format as _,
                sequence: xev.serial as _,
                window: wt.root,
                type_: xev.message_type as _,
                data: xproto::ClientMessageData::from({
                    let [a, b, c, d, e]: [c_long; 5] = xev.data.as_longs().try_into().unwrap();
                    [a as u32, b as u32, c as u32, d as u32, e as u32]
                }),
            };

            wt.xconn
                .xcb_connection()
                .send_event(
                    false,
                    wt.root,
                    xproto::EventMask::SUBSTRUCTURE_NOTIFY
                        | xproto::EventMask::SUBSTRUCTURE_REDIRECT,
                    client_msg.serialize(),
                )
                .expect_then_ignore_error("Failed to send `ClientMessage` event.");
            return;
        }

        if xev.message_type == atoms[XdndEnter] as c_ulong {
            let source_window = xev.data.get_long(0) as xproto::Window;
            let flags = xev.data.get_long(1);
            let version = flags >> 24;
            self.dnd.version = Some(version);
            let has_more_types = flags - (flags & (c_long::max_value() - 1)) == 1;
            if !has_more_types {
                let type_list = vec![
                    xev.data.get_long(2) as xproto::Atom,
                    xev.data.get_long(3) as xproto::Atom,
                    xev.data.get_long(4) as xproto::Atom,
                ];
                self.dnd.type_list = Some(type_list);
            } else if let Ok(more_types) = unsafe { self.dnd.get_type_list(source_window) } {
                self.dnd.type_list = Some(more_types);
            }
            return;
        }

        if xev.message_type == atoms[XdndPosition] as c_ulong {
            // This event occurs every time the mouse moves while a file's being dragged
            // over our window. We emit HoveredFile in response; while the macOS backend
            // does that upon a drag entering, XDND doesn't have access to the actual drop
            // data until this event. For parity with other platforms, we only emit
            // `HoveredFile` the first time, though if winit's API is later extended to
            // supply position updates with `HoveredFile` or another event, implementing
            // that here would be trivial.

            let source_window = xev.data.get_long(0) as xproto::Window;

            // Equivalent to `(x << shift) | y`
            // where `shift = mem::size_of::<c_short>() * 8`
            // Note that coordinates are in "desktop space", not "window space"
            // (in X11 parlance, they're root window coordinates)
            //let packed_coordinates = xev.data.get_long(2);
            //let shift = mem::size_of::<libc::c_short>() * 8;
            //let x = packed_coordinates >> shift;
            //let y = packed_coordinates & !(x << shift);

            // By our own state flow, `version` should never be `None` at this point.
            let version = self.dnd.version.unwrap_or(5);

            // Action is specified in versions 2 and up, though we don't need it anyway.
            //let action = xev.data.get_long(4);

            let accepted = if let Some(ref type_list) = self.dnd.type_list {
                type_list.contains(&atoms[TextUriList])
            } else {
                false
            };

            if !accepted {
                unsafe {
                    self.dnd
                        .send_status(window, source_window, DndState::Rejected)
                        .expect("Failed to send `XdndStatus` message.");
                }
                self.dnd.reset();
                return;
            }

            self.dnd.source_window = Some(source_window);
            if self.dnd.result.is_none() {
                let time = if version >= 1 {
                    xev.data.get_long(3) as xproto::Timestamp
                } else {
                    // In version 0, time isn't specified
                    x11rb::CURRENT_TIME
                };

                // Log this timestamp.
                wt.xconn.set_timestamp(time);

                // This results in the `SelectionNotify` event below
                unsafe {
                    self.dnd.convert_selection(window, time);
                }
            }

            unsafe {
                self.dnd
                    .send_status(window, source_window, DndState::Accepted)
                    .expect("Failed to send `XdndStatus` message.");
            }
            return;
        }

        if xev.message_type == atoms[XdndDrop] as c_ulong {
            let (source_window, state) = if let Some(source_window) = self.dnd.source_window {
                if let Some(Ok(ref path_list)) = self.dnd.result {
                    for path in path_list {
                        let event = Event::WindowEvent {
                            window_id,
                            event: WindowEvent::DroppedFile(path.clone()),
                        };
                        callback(&self.target, event);
                    }
                }
                (source_window, DndState::Accepted)
            } else {
                // `source_window` won't be part of our DND state if we already rejected the drop in our
                // `XdndPosition` handler.
                let source_window = xev.data.get_long(0) as xproto::Window;
                (source_window, DndState::Rejected)
            };

            unsafe {
                self.dnd
                    .send_finished(window, source_window, state)
                    .expect("Failed to send `XdndFinished` message.");
            }

            self.dnd.reset();
            return;
        }

        if xev.message_type == atoms[XdndLeave] as c_ulong {
            self.dnd.reset();
            let event = Event::WindowEvent {
                window_id,
                event: WindowEvent::HoveredFileCancelled,
            };
            callback(&self.target, event);
        }
    }

    fn selection_notify<F>(&mut self, xev: &XSelectionEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);
        let atoms = wt.xconn.atoms();

        let window = xev.requestor as xproto::Window;
        let window_id = mkwid(window);

        // Set the timestamp.
        wt.xconn.set_timestamp(xev.time as xproto::Timestamp);

        if xev.property != atoms[XdndSelection] as c_ulong {
            return;
        }

        // This is where we receive data from drag and drop
        self.dnd.result = None;
        if let Ok(mut data) = unsafe { self.dnd.read_data(window) } {
            let parse_result = self.dnd.parse_data(&mut data);
            if let Ok(ref path_list) = parse_result {
                for path in path_list {
                    let event = Event::WindowEvent {
                        window_id,
                        event: WindowEvent::HoveredFile(path.clone()),
                    };
                    callback(&self.target, event);
                }
            }
            self.dnd.result = Some(parse_result);
        }
    }

    fn configure_notify<F>(&self, xev: &XConfigureEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);

        let xwindow = xev.window as xproto::Window;
        let window_id = mkwid(xwindow);

        let window = match self.with_window(xwindow, Arc::clone) {
            Some(window) => window,
            None => return,
        };

        // So apparently...
        // `XSendEvent` (synthetic `ConfigureNotify`) -> position relative to root
        // `XConfigureNotify` (real `ConfigureNotify`) -> position relative to parent
        // https://tronche.com/gui/x/icccm/sec-4.html#s-4.1.5
        // We don't want to send `Moved` when this is false, since then every `Resized`
        // (whether the window moved or not) is accompanied by an extraneous `Moved` event
        // that has a position relative to the parent window.
        let is_synthetic = xev.send_event == xlib::True;

        // These are both in physical space.
        let new_inner_size = (xev.width as u32, xev.height as u32);
        let new_inner_position = (xev.x, xev.y);

        let (mut resized, moved) = {
            let mut shared_state_lock = window.shared_state_lock();

            let resized = util::maybe_change(&mut shared_state_lock.size, new_inner_size);
            let moved = if is_synthetic {
                util::maybe_change(&mut shared_state_lock.inner_position, new_inner_position)
            } else {
                // Detect when frame extents change.
                // Since this isn't synthetic, as per the notes above, this position is relative to the
                // parent window.
                let rel_parent = new_inner_position;
                if util::maybe_change(&mut shared_state_lock.inner_position_rel_parent, rel_parent)
                {
                    // This ensures we process the next `Moved`.
                    shared_state_lock.inner_position = None;
                    // Extra insurance against stale frame extents.
                    shared_state_lock.frame_extents = None;
                }
                false
            };
            (resized, moved)
        };

        let position = window.shared_state_lock().position;

        let new_outer_position = if let (Some(position), false) = (position, moved) {
            position
        } else {
            let mut shared_state_lock = window.shared_state_lock();

            // We need to convert client area position to window position.
            let frame_extents = shared_state_lock
                .frame_extents
                .as_ref()
                .cloned()
                .unwrap_or_else(|| {
                    let frame_extents = wt.xconn.get_frame_extents_heuristic(xwindow, wt.root);
                    shared_state_lock.frame_extents = Some(frame_extents.clone());
                    frame_extents
                });
            let outer =
                frame_extents.inner_pos_to_outer(new_inner_position.0, new_inner_position.1);
            shared_state_lock.position = Some(outer);

            // Unlock shared state to prevent deadlock in callback below
            drop(shared_state_lock);

            if moved {
                callback(
                    &self.target,
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::Moved(outer.into()),
                    },
                );
            }
            outer
        };

        if is_synthetic {
            let mut shared_state_lock = window.shared_state_lock();
            // If we don't use the existing adjusted value when available, then the user can screw up the
            // resizing by dragging across monitors *without* dropping the window.
            let (width, height) = shared_state_lock
                .dpi_adjusted
                .unwrap_or((xev.width as u32, xev.height as u32));

            let last_scale_factor = shared_state_lock.last_monitor.scale_factor;
            let new_scale_factor = {
                let window_rect = util::AaRect::new(new_outer_position, new_inner_size);
                let monitor = wt
                    .xconn
                    .get_monitor_for_window(Some(window_rect))
                    .expect("Failed to find monitor for window");

                if monitor.is_dummy() {
                    // Avoid updating monitor using a dummy monitor handle
                    last_scale_factor
                } else {
                    shared_state_lock.last_monitor = monitor.clone();
                    monitor.scale_factor
                }
            };
            if last_scale_factor != new_scale_factor {
                let (new_width, new_height) = window.adjust_for_dpi(
                    last_scale_factor,
                    new_scale_factor,
                    width,
                    height,
                    &shared_state_lock,
                );

                let old_inner_size = PhysicalSize::new(width, height);
                let new_inner_size = PhysicalSize::new(new_width, new_height);

                // Unlock shared state to prevent deadlock in callback below
                drop(shared_state_lock);

                let inner_size = Arc::new(Mutex::new(new_inner_size));
                callback(
                    &self.target,
                    Event::WindowEvent {
                        window_id,
                        event: WindowEvent::ScaleFactorChanged {
                            scale_factor: new_scale_factor,
                            inner_size_writer: InnerSizeWriter::new(Arc::downgrade(&inner_size)),
                        },
                    },
                );

                let new_inner_size = *inner_size.lock().unwrap();
                drop(inner_size);

                if new_inner_size != old_inner_size {
                    window.request_inner_size_physical(new_inner_size.width, new_inner_size.height);
                    window.shared_state_lock().dpi_adjusted = Some(new_inner_size.into());
                    // if the DPI factor changed, force a resize event to ensure the logical
                    // size is computed with the right DPI factor
                    resized = true;
                }
            }
        }

        // NOTE: Ensure that the lock is dropped before handling the resized and
        // sending the event back to user.
        let hittest = {
            let mut shared_state_lock = window.shared_state_lock();
            let hittest = shared_state_lock.cursor_hittest;

            // This is a hack to ensure that the DPI adjusted resize is actually
            // applied on all WMs. KWin doesn't need this, but Xfwm does. The hack
            // should not be run on other WMs, since tiling WMs constrain the window
            // size, making the resize fail. This would cause an endless stream of
            // XResizeWindow requests, making Xorg, the winit client, and the WM
            // consume 100% of CPU.
            if let Some(adjusted_size) = shared_state_lock.dpi_adjusted {
                if new_inner_size == adjusted_size || !util::wm_name_is_one_of(&["Xfwm4"]) {
                    // When this finally happens, the event will not be synthetic.
                    shared_state_lock.dpi_adjusted = None;
                } else {
                    // Unlock shared state to prevent deadlock in callback below
                    drop(shared_state_lock);
                    window.request_inner_size_physical(adjusted_size.0, adjusted_size.1);
                }
            }

            hittest
        };

        // Reload hittest.
        if hittest.unwrap_or(false) {
            let _ = window.set_cursor_hittest(true);
        }

        if resized {
            callback(
                &self.target,
                Event::WindowEvent {
                    window_id,
                    event: WindowEvent::Resized(new_inner_size.into()),
                },
            );
        }
    }

    /// This is generally a reliable way to detect when the window manager's been
    /// replaced, though this event is only fired by reparenting window managers
    /// (which is almost all of them). Failing to correctly update WM info doesn't
    /// really have much impact, since on the WMs affected (xmonad, dwm, etc.) the only
    /// effect is that we waste some time trying to query unsupported properties.
    fn reparent_notify(&self, xev: &XReparentEvent) {
        let wt = Self::window_target(&self.target);

        wt.xconn.update_cached_wm_info(wt.root);

        self.with_window(xev.window as xproto::Window, |window| {
            window.invalidate_cached_frame_extents();
        });
    }

    fn map_notify<F>(&self, xev: &XMapEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let window = xev.window as xproto::Window;
        let window_id = mkwid(window);

        // NOTE: Re-issue the focus state when mapping the window.
        //
        // The purpose of it is to deliver initial focused state of the newly created
        // window, given that we can't rely on `CreateNotify`, due to it being not
        // sent.
        let focus = self
            .with_window(window, |window| window.has_focus())
            .unwrap_or_default();
        let event = Event::WindowEvent {
            window_id,
            event: WindowEvent::Focused(focus),
        };

        callback(&self.target, event);
    }

    fn destroy_notify<F>(&self, xev: &XDestroyWindowEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);

        let window = xev.window as xproto::Window;
        let window_id = mkwid(window);

        // In the event that the window's been destroyed without being dropped first, we
        // cleanup again here.
        wt.windows.borrow_mut().remove(&WindowId(window as _));

        callback(
            &self.target,
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Destroyed,
            },
        );
    }

    fn property_notify<F>(&mut self, xev: &XPropertyEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);
        let atoms = wt.x_connection().atoms();
        let atom = xev.atom as xproto::Atom;

        if atom == xproto::Atom::from(xproto::AtomEnum::RESOURCE_MANAGER)
            || atom == atoms[_XSETTINGS_SETTINGS]
        {
            self.process_dpi_change(&mut callback);
        }
    }

    fn visibility_notify<F>(&self, xev: &XVisibilityEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let xwindow = xev.window as xproto::Window;

        let event = Event::WindowEvent {
            window_id: mkwid(xwindow),
            event: WindowEvent::Occluded(xev.state == xlib::VisibilityFullyObscured),
        };
        callback(&self.target, event);

        self.with_window(xwindow, |window| {
            window.visibility_notify();
        });
    }

    fn expose<F>(&self, xev: &XExposeEvent, mut callback: F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        // Multiple Expose events may be received for subareas of a window.
        // We issue `RedrawRequested` only for the last event of such a series.
        if xev.count == 0 {
            let window = xev.window as xproto::Window;
            let window_id = mkwid(window);

            let event = Event::WindowEvent {
                window_id,
                event: WindowEvent::RedrawRequested,
            };

            callback(&self.target, event);
        }
    }

    fn process_dpi_change<F>(&self, callback: &mut F)
    where
        F: FnMut(&RootELW<T>, Event<T>),
    {
        let wt = Self::window_target(&self.target);
        wt.xconn
            .reload_database()
            .expect("failed to reload Xft database");

        // In the future, it would be quite easy to emit monitor hotplug events.
        let prev_list = {
            let prev_list = wt.xconn.invalidate_cached_monitor_list();
            match prev_list {
                Some(prev_list) => prev_list,
                None => return,
            }
        };

        let new_list = wt
            .xconn
            .available_monitors()
            .expect("Failed to get monitor list");
        for new_monitor in new_list {
            // Previous list may be empty, in case of disconnecting and
            // reconnecting the only one monitor. We still need to emit events in
            // this case.
            let maybe_prev_scale_factor = prev_list
                .iter()
                .find(|prev_monitor| prev_monitor.name == new_monitor.name)
                .map(|prev_monitor| prev_monitor.scale_factor);
            if Some(new_monitor.scale_factor) != maybe_prev_scale_factor {
                for window in wt.windows.borrow().iter().filter_map(|(_, w)| w.upgrade()) {
                    window.refresh_dpi_for_monitor(&new_monitor, maybe_prev_scale_factor, |event| {
                        callback(&self.target, event);
                    })
                }
            }
        }
    }

    fn window_exists(&self, window_id: xproto::Window) -> bool {
        self.with_window(window_id, |_| ()).is_some()
    }
}
