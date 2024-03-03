//! Terminal window context.

use std::error::Error;
use std::rc::Rc;

#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use glutin::platform::x11::X11GlConfigExt;
use raw_window_handle::HasRawDisplayHandle;
use winit::event::{Event as WinitEvent, WindowEvent};
use winit::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use winit::window::WindowId;

use crate::cli::WindowOptions;
use crate::config::UiConfig;
use crate::display::window::Window;
use crate::display::Display;
use crate::event::{ActionContext, Event};
use crate::scheduler::Scheduler;
use crate::{input, renderer};

/// Event context for one individual Alacritty window.
pub struct WindowContext {
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,
    occluded: bool,
    config: Rc<UiConfig>,
}

impl WindowContext {
    /// Create initial window context that does bootstrapping the graphics API we're going to use.
    pub fn initial(
        event_loop: &EventLoopWindowTarget<Event>,
        config: Rc<UiConfig>,
        options: WindowOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let raw_display_handle = event_loop.raw_display_handle();

        // Windows has different order of GL platform initialization compared to any other platform;
        // it requires the window first.
        #[cfg(windows)]
        let window = Window::new(event_loop, &config)?;
        #[cfg(windows)]
        let raw_window_handle = Some(window.raw_window_handle());

        #[cfg(not(windows))]
        let raw_window_handle = None;

        let gl_display =
            renderer::platform::create_gl_display(raw_display_handle, raw_window_handle, false)?;
        let gl_config = renderer::platform::pick_gl_config(&gl_display, raw_window_handle)?;

        #[cfg(not(windows))]
        let window = Window::new(
            event_loop,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
            #[cfg(target_os = "macos")]
            &options.window_tabbing_id,
        )?;

        // Create context.
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)?;

        let display = Display::new(window, gl_context, &config, false)?;

        Self::new(display, config)
    }

    /// Create a new terminal window context.
    fn new(display: Display, config: Rc<UiConfig>) -> Result<Self, Box<dyn Error>> {
        Ok(WindowContext {
            display,
            config,
            event_queue: Default::default(),
            occluded: Default::default(),
            dirty: Default::default(),
        })
    }

    /// Draw the window.
    pub fn draw(&mut self, scheduler: &mut Scheduler) {
        self.display.window.requested_redraw = false;

        if self.occluded {
            return;
        }

        self.dirty = false;

        // Force the display to process any pending display update.
        self.display.process_renderer_update();

        self.display.draw(scheduler);
    }

    /// Process events for this terminal window.
    pub fn handle_event(
        &mut self,
        event_loop: &EventLoopWindowTarget<Event>,
        event_proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        match event {
            WinitEvent::AboutToWait
            | WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                // Skip further event handling with no staged updates.
                if self.event_queue.is_empty() {
                    return;
                }

                // Continue to process all pending events.
            },
            event => {
                self.event_queue.push(event);
                return;
            },
        }

        let context = ActionContext {
            display: &mut self.display,
            dirty: &mut self.dirty,
            occluded: &mut self.occluded,
            config: &self.config,
            event_proxy,
            event_loop,
            scheduler,
        };
        let mut processor = input::Processor::new(context);

        for event in self.event_queue.drain(..) {
            processor.handle_event(event);
        }

        // Process DisplayUpdate events.
        if self.display.pending_update.dirty {
            Self::submit_display_update(&mut self.display);
            self.dirty = true;
        }

        // Don't call `request_redraw` when event is `RedrawRequested` since the `dirty` flag
        // represents the current frame, but redraw is for the next frame.
        if self.dirty
            && self.display.window.has_frame
            && !self.occluded
            && !matches!(event, WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. })
        {
            self.display.window.request_redraw();
        }
    }

    /// ID of this terminal context.
    pub fn id(&self) -> WindowId {
        self.display.window.id()
    }

    /// Submit the pending changes to the `Display`.
    fn submit_display_update(display: &mut Display) {
        display.handle_update();
    }
}
