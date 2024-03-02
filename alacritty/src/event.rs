//! Process window events.

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::fmt::Debug;
#[cfg(not(windows))]
use std::os::unix::io::RawFd;
use std::rc::Rc;
use std::time::Instant;
use std::{f32, mem};

use ahash::RandomState;
use glutin::display::{Display as GlutinDisplay, GetGlDisplay};
use log::info;
use winit::event::{
    ElementState, Event as WinitEvent, Modifiers, MouseButton, StartCause, WindowEvent,
};
use winit::event_loop::{
    ControlFlow, DeviceEvents, EventLoop, EventLoopProxy, EventLoopWindowTarget,
};
use winit::window::WindowId;

use alacritty_terminal::event::{Event as TerminalEvent, EventListener, Notify};
use alacritty_terminal::event_loop::Notifier;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Direction, Point, Side};
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::Term;

use crate::cli::WindowOptions;
use crate::config::UiConfig;
use crate::display::window::Window;
use crate::display::Display;
use crate::input::{self, ActionContext as _};
use crate::scheduler::Scheduler;
use crate::window_context::WindowContext;

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
    Terminal(TerminalEvent),
    Scroll(Scroll),
    CreateWindow(WindowOptions),
    SearchNext,
    Frame,
}

impl From<TerminalEvent> for EventType {
    fn from(event: TerminalEvent) -> Self {
        Self::Terminal(event)
    }
}

pub struct ActionContext<'a, N, T> {
    pub notifier: &'a mut N,
    pub terminal: &'a mut Term<T>,
    pub display: &'a mut Display,
    pub config: &'a UiConfig,
    pub event_loop: &'a EventLoopWindowTarget<Event>,
    pub event_proxy: &'a EventLoopProxy<Event>,
    pub scheduler: &'a mut Scheduler,
    pub dirty: &'a mut bool,
    pub occluded: &'a mut bool,
    #[cfg(not(windows))]
    pub master_fd: RawFd,
    #[cfg(not(windows))]
    pub shell_pid: u32,
}

impl<'a, N: Notify + 'a, T: EventListener> input::ActionContext<T> for ActionContext<'a, N, T> {
    #[inline]
    fn window(&mut self) -> &mut Window {
        &mut self.display.window
    }
}

/// Identified purpose of the touch input.
#[derive(Debug)]
pub enum TouchPurpose {
    None,
}

impl Default for TouchPurpose {
    fn default() -> Self {
        Self::None
    }
}

/// State of the mouse.
#[derive(Debug)]
pub struct Mouse {
    pub left_button_state: ElementState,
    pub middle_button_state: ElementState,
    pub right_button_state: ElementState,
    pub last_click_timestamp: Instant,
    pub last_click_button: MouseButton,
    pub click_state: ClickState,
    pub cell_side: Side,
    pub lines_scrolled: f32,
    pub block_hint_launcher: bool,
    pub hint_highlight_dirty: bool,
    pub inside_text_area: bool,
    pub x: usize,
    pub y: usize,
}

impl Default for Mouse {
    fn default() -> Mouse {
        Mouse {
            last_click_timestamp: Instant::now(),
            last_click_button: MouseButton::Left,
            left_button_state: ElementState::Released,
            middle_button_state: ElementState::Released,
            right_button_state: ElementState::Released,
            click_state: ClickState::None,
            cell_side: Side::Left,
            hint_highlight_dirty: Default::default(),
            block_hint_launcher: Default::default(),
            inside_text_area: Default::default(),
            lines_scrolled: Default::default(),
            x: Default::default(),
            y: Default::default(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ClickState {
    None,
}

impl input::Processor<EventProxy, ActionContext<'_, Notifier, EventProxy>> {
    /// Handle events from winit.
    pub fn handle_event(&mut self, event: WinitEvent<Event>) {
        match event {
            WinitEvent::UserEvent(Event { payload: _, .. }) => (),
            WinitEvent::WindowEvent { event, .. } => {
                match event {
                    WindowEvent::CloseRequested => self.ctx.terminal.exit(),
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        let old_scale_factor =
                            mem::replace(&mut self.ctx.window().scale_factor, scale_factor);

                        let display_update_pending = &mut self.ctx.display.pending_update;

                        // Rescale font size for the new factor.
                        let font_scale = scale_factor as f32 / old_scale_factor as f32;
                        self.ctx.display.font_size = self.ctx.display.font_size.scale(font_scale);

                        let font = self.ctx.config.font.clone();
                        display_update_pending.set_font(font.with_size(self.ctx.display.font_size));
                    },
                    WindowEvent::Resized(size) => {
                        // Ignore resize events to zero in any dimension, to avoid issues with Winit
                        // and the ConPTY. A 0x0 resize will also occur when the window is minimized
                        // on Windows.
                        if size.width == 0 || size.height == 0 {
                            return;
                        }

                        self.ctx.display.pending_update.set_dimensions(size);
                    },
                    WindowEvent::KeyboardInput { event: _, is_synthetic: false, .. } => (),
                    WindowEvent::ModifiersChanged(_) => (),
                    WindowEvent::MouseInput { state: _, button: _, .. } => (),
                    WindowEvent::CursorMoved { position: _, .. } => (),
                    WindowEvent::MouseWheel { delta: _, phase: _, .. } => (),
                    WindowEvent::KeyboardInput { is_synthetic: true, .. }
                    | WindowEvent::ActivationTokenDone { .. }
                    | WindowEvent::TouchpadPressure { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::TouchpadMagnify { .. }
                    | WindowEvent::TouchpadRotate { .. }
                    | WindowEvent::SmartMagnify { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::AxisMotion { .. }
                    | WindowEvent::HoveredFileCancelled
                    | WindowEvent::Destroyed
                    | WindowEvent::ThemeChanged(_)
                    | WindowEvent::HoveredFile(_)
                    | WindowEvent::RedrawRequested
                    | WindowEvent::Moved(_)
                    | WindowEvent::Touch(_)
                    | WindowEvent::Focused(_)
                    | WindowEvent::Occluded(_)
                    | WindowEvent::DroppedFile(_)
                    | WindowEvent::Ime(_) => (),
                }
            },
            WinitEvent::Suspended { .. }
            | WinitEvent::NewEvents { .. }
            | WinitEvent::DeviceEvent { .. }
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
    config: Rc<UiConfig>,
}

impl Processor {
    /// Create a new event processor.
    ///
    /// Takes a writer which is expected to be hooked up to the write end of a PTY.
    pub fn new(_event_loop: &EventLoop<Event>) -> Processor {
        Processor {
            gl_display: None,
            config: Rc::new(UiConfig::default()),
            windows: Default::default(),
        }
    }

    /// Create initial window and load GL platform.
    ///
    /// This will initialize the OpenGL Api and pick a config that
    /// will be used for the rest of the windows.
    pub fn create_initial_window(
        &mut self,
        event_loop: &EventLoopWindowTarget<Event>,
        proxy: EventLoopProxy<Event>,
    ) -> Result<(), Box<dyn Error>> {
        let window_context = WindowContext::initial(
            event_loop,
            proxy,
            Rc::new(UiConfig::default()),
            WindowOptions::default(),
        )?;

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
                    if let Err(err) = self.create_initial_window(event_loop, proxy.clone()) {
                        *initial_window_error_loop = Err(err);
                        event_loop.exit();
                        return;
                    }

                    info!("Initialisation complete");
                },
                // NOTE: This event bypasses batching to minimize input latency.
                WinitEvent::UserEvent(Event {
                    window_id: Some(window_id),
                    payload: EventType::Terminal(TerminalEvent::Wakeup),
                }) => {
                    if let Some(window_context) = self.windows.get_mut(&window_id) {
                        window_context.dirty = true;
                        if window_context.display.window.has_frame {
                            window_context.display.window.request_redraw();
                        }
                    }
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
                // Check for shutdown.
                WinitEvent::UserEvent(Event {
                    window_id: Some(window_id),
                    payload: EventType::Terminal(TerminalEvent::Exit),
                }) => {
                    // Remove the closed terminal.
                    let window_context = match self.windows.remove(&window_id) {
                        Some(window_context) => window_context,
                        None => return,
                    };

                    // Unschedule pending events.
                    scheduler.unschedule_window(window_context.id());

                    // Shutdown if no more terminals are open.
                    if self.windows.is_empty() {
                        event_loop.exit();
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
                WindowEvent::KeyboardInput { is_synthetic: true, .. }
                    | WindowEvent::TouchpadPressure { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::AxisMotion { .. }
                    | WindowEvent::HoveredFileCancelled
                    | WindowEvent::Destroyed
                    | WindowEvent::HoveredFile(_)
                    | WindowEvent::Moved(_)
            ),
            WinitEvent::Suspended { .. } | WinitEvent::NewEvents { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventProxy {
    proxy: EventLoopProxy<Event>,
    window_id: WindowId,
}

impl EventProxy {
    pub fn new(proxy: EventLoopProxy<Event>, window_id: WindowId) -> Self {
        Self { proxy, window_id }
    }

    /// Send an event to the event loop.
    pub fn send_event(&self, event: EventType) {
        let _ = self.proxy.send_event(Event::new(event, self.window_id));
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: TerminalEvent) {
        let _ = self.proxy.send_event(Event::new(event.into(), self.window_id));
    }
}
