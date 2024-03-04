//! Process window events.

use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;

use ahash::RandomState;
use glutin::display::{Display as GlutinDisplay, GetGlDisplay};
use log::info;
use winit::event::{Event as WinitEvent, StartCause, WindowEvent};
use winit::event_loop::{
    ControlFlow, DeviceEvents, EventLoop, EventLoopProxy, EventLoopWindowTarget,
};
use winit::window::WindowId;

use crate::display::window::Window;
use crate::display::Display;
use crate::scheduler::Scheduler;
use crate::window_context::WindowContext;

pub struct InputProcessor<A: InputActionContext> {
    pub ctx: A,
}

pub trait InputActionContext {
    fn window(&mut self) -> &mut Window;
}

impl<A: InputActionContext> InputProcessor<A> {
    pub fn new(ctx: A) -> Self {
        Self { ctx }
    }
}

/// Alacritty events.
#[derive(Debug, Clone)]
pub struct Event {
    /// Limit event to a specific window.
    window_id: Option<WindowId>,

    /// Event payload.
    payload: EventType,
}

impl Event {
    pub fn new<I: Into<Option<WindowId>>>(payload: EventType, window_id: I) -> Self {
        Self { window_id: window_id.into(), payload }
    }
}

impl From<Event> for WinitEvent<Event> {
    fn from(event: Event) -> Self {
        WinitEvent::UserEvent(event)
    }
}

/// Alacritty events.
#[derive(Debug, Clone)]
pub enum EventType {
    SearchNext,
    Frame,
}

pub struct ActionContext<'a> {
    pub display: &'a mut Display,
    pub event_loop: &'a EventLoopWindowTarget<Event>,
    pub event_proxy: &'a EventLoopProxy<Event>,
    pub scheduler: &'a mut Scheduler,
    pub dirty: &'a mut bool,
    pub occluded: &'a mut bool,
}

impl<'a> InputActionContext for ActionContext<'a> {
    #[inline]
    fn window(&mut self) -> &mut Window {
        &mut self.display.window
    }
}

impl InputProcessor<ActionContext<'_>> {
    /// Handle events from winit.
    pub fn handle_event(&mut self, event: WinitEvent<Event>) {
        match event {
            WinitEvent::UserEvent(Event { payload: _, .. }) => (),
            WinitEvent::WindowEvent { event, .. } => {
                match event {
                    WindowEvent::Resized(size) => {
                        // Ignore resize events to zero in any dimension, to avoid issues with Winit
                        // and the ConPTY. A 0x0 resize will also occur when the window is minimized
                        // on Windows.
                        if size.width == 0 || size.height == 0 {
                            return;
                        }

                        self.ctx.display.pending_update.set_dimensions(size);
                    },
                    WindowEvent::ScaleFactorChanged { scale_factor: _, .. } => {},
                    WindowEvent::ActivationTokenDone { .. }
                    | WindowEvent::HoveredFileCancelled
                    | WindowEvent::Destroyed
                    | WindowEvent::ThemeChanged(_)
                    | WindowEvent::HoveredFile(_)
                    | WindowEvent::RedrawRequested
                    | WindowEvent::CloseRequested
                    | WindowEvent::Moved(_)
                    | WindowEvent::Focused(_)
                    | WindowEvent::Occluded(_)
                    | WindowEvent::DroppedFile(_) => (),
                }
            },
            WinitEvent::Suspended { .. }
            | WinitEvent::NewEvents { .. }
            | WinitEvent::LoopExiting
            | WinitEvent::Resumed
            | WinitEvent::MemoryWarning
            | WinitEvent::AboutToWait => (),
        }
    }
}

/// The event processor.
///
/// Stores some state from received events and dispatches actions when they are
/// triggered.
pub struct Processor {
    windows: HashMap<WindowId, WindowContext, RandomState>,
    gl_display: Option<GlutinDisplay>,
}

impl Processor {
    /// Create a new event processor.
    ///
    /// Takes a writer which is expected to be hooked up to the write end of a PTY.
    pub fn new(_event_loop: &EventLoop<Event>) -> Processor {
        Processor { gl_display: None, windows: Default::default() }
    }

    /// Create initial window and load GL platform.
    ///
    /// This will initialize the OpenGL Api and pick a config that
    /// will be used for the rest of the windows.
    pub fn create_initial_window(
        &mut self,
        event_loop: &EventLoopWindowTarget<Event>,
    ) -> Result<(), Box<dyn Error>> {
        let window_context = WindowContext::initial(event_loop)?;

        self.gl_display = Some(window_context.display.gl_context().display());
        self.windows.insert(window_context.id(), window_context);

        Ok(())
    }

    /// Run the event loop.
    ///
    /// The result is exit code generate from the loop.
    pub fn run(&mut self, event_loop: EventLoop<Event>) -> Result<(), Box<dyn Error>> {
        let proxy = event_loop.create_proxy();
        let mut scheduler = Scheduler::new(proxy.clone());

        // Disable all device events, since we don't care about them.
        event_loop.listen_device_events(DeviceEvents::Never);

        let mut initial_window_error = Ok(());
        let initial_window_error_loop = &mut initial_window_error;
        // SAFETY: Since this takes a pointer to the winit event loop, it MUST be dropped first,
        // which is done by `move` into event loop.
        let result = event_loop.run(move |event, event_loop| {
            // Ignore all events we do not care about.
            if Self::skip_event(&event) {
                return;
            }

            match event {
                // The event loop just got initialized. Create a window.
                WinitEvent::Resumed => {
                    if let Err(err) = self.create_initial_window(event_loop) {
                        *initial_window_error_loop = Err(err);
                        event_loop.exit();
                        return;
                    }

                    info!("Initialisation complete");
                },
                // NOTE: This event bypasses batching to minimize input latency.
                WinitEvent::UserEvent(Event {
                    window_id: Some(window_id),
                    payload: EventType::Frame,
                }) => {
                    if let Some(window_context) = self.windows.get_mut(&window_id) {
                        window_context.display.window.has_frame = true;
                        if window_context.dirty {
                            window_context.display.window.request_redraw();
                        }
                    }
                },
                WinitEvent::WindowEvent { window_id, event: WindowEvent::RedrawRequested } => {
                    let window_context = match self.windows.get_mut(&window_id) {
                        Some(window_context) => window_context,
                        None => return,
                    };

                    window_context.handle_event(event_loop, &proxy, &mut scheduler, event);

                    window_context.draw(&mut scheduler);
                },
                // Process all pending events.
                WinitEvent::AboutToWait => {
                    // Dispatch event to all windows.
                    for window_context in self.windows.values_mut() {
                        window_context.handle_event(
                            event_loop,
                            &proxy,
                            &mut scheduler,
                            WinitEvent::AboutToWait,
                        );
                    }

                    // Update the scheduler after event processing to ensure
                    // the event loop deadline is as accurate as possible.
                    let control_flow = match scheduler.update() {
                        Some(instant) => ControlFlow::WaitUntil(instant),
                        None => ControlFlow::Wait,
                    };
                    event_loop.set_control_flow(control_flow);
                },
                // Process events affecting all windows.
                WinitEvent::UserEvent(event @ Event { window_id: None, .. }) => {
                    for window_context in self.windows.values_mut() {
                        window_context.handle_event(
                            event_loop,
                            &proxy,
                            &mut scheduler,
                            event.clone().into(),
                        );
                    }
                },
                // Process window-specific events.
                WinitEvent::WindowEvent { window_id, .. }
                | WinitEvent::UserEvent(Event { window_id: Some(window_id), .. }) => {
                    if let Some(window_context) = self.windows.get_mut(&window_id) {
                        window_context.handle_event(event_loop, &proxy, &mut scheduler, event);
                    }
                },
                _ => (),
            }
        });

        if initial_window_error.is_err() {
            initial_window_error
        } else {
            result.map_err(Into::into)
        }
    }

    /// Check if an event is irrelevant and can be skipped.
    fn skip_event(event: &WinitEvent<Event>) -> bool {
        match event {
            WinitEvent::NewEvents(StartCause::Init) => false,
            WinitEvent::WindowEvent { event, .. } => matches!(
                event,
                WindowEvent::HoveredFileCancelled
                    | WindowEvent::Destroyed
                    | WindowEvent::HoveredFile(_)
                    | WindowEvent::Moved(_)
            ),
            WinitEvent::Suspended { .. } | WinitEvent::NewEvents { .. } => true,
            _ => false,
        }
    }
}
