//! The display subsystem including window management, font rasterization, and
//! GPU drawing.

use std::fmt::{self, Formatter};
use std::mem::{self, ManuallyDrop};
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::time::{Duration, Instant};

use glutin::context::{NotCurrentContext, PossiblyCurrentContext};
use glutin::prelude::*;
use glutin::surface::{Surface, SwapInterval, WindowSurface};

use log::{debug, info};
use raw_window_handle::RawWindowHandle;
use serde::{Deserialize, Serialize};
use winit::dpi::PhysicalSize;

use crossfont::{self, Rasterize, Rasterizer, Size as FontSize};

use crate::config::font::Font;
use crate::config::UiConfig;
use crate::display::color::{List, Rgb};
use crate::display::window::Window;
use crate::event::{Event, EventType};
use crate::renderer::rects::RenderRect;
use crate::renderer::{self, Renderer};
use crate::scheduler::{Scheduler, TimerId, Topic};

pub mod color;
pub mod window;

#[derive(Debug)]
pub enum Error {
    /// Error with window management.
    Window(window::Error),

    /// Error dealing with fonts.
    Font(crossfont::Error),

    /// Error in renderer.
    Render(renderer::Error),

    /// Error during context operations.
    Context(glutin::error::Error),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Window(err) => err.source(),
            Error::Font(err) => err.source(),
            Error::Render(err) => err.source(),
            Error::Context(err) => err.source(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::Window(err) => err.fmt(f),
            Error::Font(err) => err.fmt(f),
            Error::Render(err) => err.fmt(f),
            Error::Context(err) => err.fmt(f),
        }
    }
}

impl From<window::Error> for Error {
    fn from(val: window::Error) -> Self {
        Error::Window(val)
    }
}

impl From<crossfont::Error> for Error {
    fn from(val: crossfont::Error) -> Self {
        Error::Font(val)
    }
}

impl From<renderer::Error> for Error {
    fn from(val: renderer::Error) -> Self {
        Error::Render(val)
    }
}

impl From<glutin::error::Error> for Error {
    fn from(val: glutin::error::Error) -> Self {
        Error::Context(val)
    }
}

/// Terminal size info.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct SizeInfo<T = f32> {
    /// Terminal window width.
    width: T,

    /// Terminal window height.
    height: T,
}

impl From<SizeInfo<f32>> for SizeInfo<u32> {
    fn from(size_info: SizeInfo<f32>) -> Self {
        Self { width: size_info.width as u32, height: size_info.height as u32 }
    }
}

impl<T: Clone + Copy> SizeInfo<T> {
    #[inline]
    pub fn width(&self) -> T {
        self.width
    }

    #[inline]
    pub fn height(&self) -> T {
        self.height
    }
}

impl SizeInfo<f32> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(width: f32, height: f32) -> SizeInfo {
        SizeInfo { width, height }
    }
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct DisplayUpdate {
    pub dirty: bool,

    dimensions: Option<PhysicalSize<u32>>,
    cursor_dirty: bool,
    font: Option<Font>,
}

impl DisplayUpdate {
    pub fn dimensions(&self) -> Option<PhysicalSize<u32>> {
        self.dimensions
    }

    pub fn set_dimensions(&mut self, dimensions: PhysicalSize<u32>) {
        self.dimensions = Some(dimensions);
        self.dirty = true;
    }
}

/// The display wraps a window, font rasterizer, and GPU renderer.
pub struct Display {
    pub window: Window,

    pub size_info: SizeInfo,

    pub raw_window_handle: RawWindowHandle,

    /// UI cursor visibility for blinking.
    pub cursor_hidden: bool,

    /// Mapped RGB values for each terminal color.
    pub colors: List,

    /// Unprocessed display updates.
    pub pending_update: DisplayUpdate,

    /// The renderer update that takes place only once before the actual rendering.
    pub pending_renderer_update: Option<RendererUpdate>,

    /// The state of the timer for frame scheduling.
    pub frame_timer: FrameTimer,

    renderer: ManuallyDrop<Renderer>,

    surface: ManuallyDrop<Surface<WindowSurface>>,

    context: ManuallyDrop<Replaceable<PossiblyCurrentContext>>,
}

impl Display {
    pub fn new(
        window: Window,
        gl_context: NotCurrentContext,
        config: &UiConfig,
        _tabbed: bool,
    ) -> Result<Display, Error> {
        let raw_window_handle = window.raw_window_handle();

        let scale_factor = window.scale_factor as f32;
        let rasterizer = Rasterizer::new()?;

        // Create the GL surface to draw into.
        let surface = renderer::platform::create_gl_surface(
            &gl_context,
            window.inner_size(),
            window.raw_window_handle(),
        )?;

        // Make the context current.
        let context = gl_context.make_current(&surface)?;

        // Create renderer.
        let mut renderer = Renderer::new(&context)?;

        let viewport_size = window.inner_size();

        // Create new size with at least one column and row.
        let size_info = SizeInfo::new(viewport_size.width as f32, viewport_size.height as f32);

        // Clear screen.
        let background_color = config.colors.primary.background;
        renderer.clear(background_color, 1.0);

        // On Wayland we can safely ignore this call, since the window isn't visible until you
        // actually draw something into it and commit those changes.
        let is_wayland = matches!(raw_window_handle, RawWindowHandle::Wayland(_));
        if !is_wayland {
            surface.swap_buffers(&context).expect("failed to swap buffers.");
            renderer.finish();
        }

        window.set_visible(true);

        // Disable vsync.
        if let Err(err) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
            info!("Failed to disable vsync: {}", err);
        }

        Ok(Self {
            context: ManuallyDrop::new(Replaceable::new(context)),
            renderer: ManuallyDrop::new(renderer),
            surface: ManuallyDrop::new(surface),
            colors: List::from(&config.colors),
            frame_timer: FrameTimer::new(),
            raw_window_handle,
            size_info,
            window,
            pending_renderer_update: Default::default(),
            pending_update: Default::default(),
            cursor_hidden: Default::default(),
        })
    }

    #[inline]
    pub fn gl_context(&self) -> &PossiblyCurrentContext {
        self.context.get()
    }

    pub fn make_current(&self) {
        if !self.context.get().is_current() {
            self.context.make_current(&self.surface).expect("failed to make context current")
        }
    }

    fn swap_buffers(&self) {
        #[allow(clippy::single_match)]
        let res = match (self.surface.deref(), &self.context.get()) {
            (surface, context) => surface.swap_buffers(context),
        };
        if let Err(err) = res {
            debug!("error calling swap_buffers: {}", err);
        }
    }

    // XXX: this function must not call to any `OpenGL` related tasks. Renderer updates are
    // performed in [`Self::process_renderer_update`] right before drawing.
    //
    /// Process update events.
    pub fn handle_update(&mut self, config: &UiConfig) {
        let pending_update = mem::take(&mut self.pending_update);

        let (mut width, mut height) = (self.size_info.width(), self.size_info.height());
        if let Some(dimensions) = pending_update.dimensions() {
            width = dimensions.width as f32;
            height = dimensions.height as f32;
        }

        let new_size = SizeInfo::new(width, height);

        // Check if dimensions have changed.
        if new_size != self.size_info {
            // Queue renderer update.
            let renderer_update = self.pending_renderer_update.get_or_insert(Default::default());
            renderer_update.resize = true;
        }
        self.size_info = new_size;
    }

    // NOTE: Renderer updates are split off, since platforms like Wayland require resize and other
    // OpenGL operations to be performed right before rendering. Otherwise they could lock the
    // back buffer and render with the previous state. This also solves flickering during resizes.
    //
    /// Update the state of the renderer.
    pub fn process_renderer_update(&mut self) {
        let renderer_update = match self.pending_renderer_update.take() {
            Some(renderer_update) => renderer_update,
            _ => return,
        };

        // Resize renderer.
        if renderer_update.resize {
            let width = NonZeroU32::new(self.size_info.width() as u32).unwrap();
            let height = NonZeroU32::new(self.size_info.height() as u32).unwrap();
            self.surface.resize(&self.context, width, height);
        }

        // Ensure we're modifying the correct OpenGL context.
        self.make_current();
    }

    /// Draw the screen.
    ///
    /// A reference to Term whose state is being drawn must be provided.
    ///
    /// This call may block if vsync is enabled.
    pub fn draw(&mut self, scheduler: &mut Scheduler, config: &UiConfig) {
        let size_info = self.size_info;

        // Make sure this window's OpenGL context is active.
        self.make_current();

        self.renderer.clear(Rgb::new(24, 24, 24), 1.0);

        // Ensure macOS hasn't reset our viewport.
        #[cfg(target_os = "macos")]
        self.renderer.set_viewport(&size_info);

        let mut rects: Vec<RenderRect> = Vec::new();
        rects.push(RenderRect::new(10., 10., 100., 50., Rgb::new(255, 0, 0), 1.));
        rects.push(RenderRect::new(500., 200., 100., 50., Rgb::new(255, 255, 0), 1.));
        self.renderer.draw_rects(&size_info, rects);

        // Notify winit that we're about to present.
        self.window.pre_present_notify();

        // Clearing debug highlights from the previous frame requires full redraw.
        self.swap_buffers();

        if matches!(self.raw_window_handle, RawWindowHandle::Xcb(_) | RawWindowHandle::Xlib(_)) {
            // On X11 `swap_buffers` does not block for vsync. However the next OpenGl command
            // will block to synchronize (this is `glClear` in Alacritty), which causes a
            // permanent one frame delay.
            self.renderer.finish();
        }

        // XXX: Request the new frame after swapping buffers, so the
        // time to finish OpenGL operations is accounted for in the timeout.
        if !matches!(self.raw_window_handle, RawWindowHandle::Wayland(_)) {
            self.request_frame(scheduler);
        }
    }

    /// Request a new frame for a window on Wayland.
    fn request_frame(&mut self, scheduler: &mut Scheduler) {
        // Mark that we've used a frame.
        self.window.has_frame = false;

        // Get the display vblank interval.
        let monitor_vblank_interval = 1_000_000.
            / self
                .window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .unwrap_or(60_000) as f64;

        // Now convert it to micro seconds.
        let monitor_vblank_interval =
            Duration::from_micros((1000. * monitor_vblank_interval) as u64);

        let swap_timeout = self.frame_timer.compute_timeout(monitor_vblank_interval);

        let window_id = self.window.id();
        let timer_id = TimerId::new(Topic::Frame, window_id);
        let event = Event::new(EventType::Frame, window_id);

        scheduler.schedule(event, swap_timeout, false, timer_id);
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        // Switch OpenGL context before dropping, otherwise objects (like programs) from other
        // contexts might be deleted when dropping renderer.
        self.make_current();
        unsafe {
            ManuallyDrop::drop(&mut self.renderer);
            ManuallyDrop::drop(&mut self.context);
            ManuallyDrop::drop(&mut self.surface);
        }
    }
}

/// Pending renderer updates.
///
/// All renderer updates are cached to be applied just before rendering, to avoid platform-specific
/// rendering issues.
#[derive(Debug, Default, Copy, Clone)]
pub struct RendererUpdate {
    /// Should resize the window.
    resize: bool,
}

/// Struct for safe in-place replacement.
///
/// This struct allows easily replacing struct fields that provide `self -> Self` methods in-place,
/// without having to deal with constantly unwrapping the underlying [`Option`].
struct Replaceable<T>(Option<T>);

impl<T> Replaceable<T> {
    pub fn new(inner: T) -> Self {
        Self(Some(inner))
    }

    /// Get immutable access to the wrapped value.
    pub fn get(&self) -> &T {
        self.0.as_ref().unwrap()
    }

    /// Get mutable access to the wrapped value.
    pub fn get_mut(&mut self) -> &mut T {
        self.0.as_mut().unwrap()
    }
}

impl<T> Deref for Replaceable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for Replaceable<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

/// The frame timer state.
pub struct FrameTimer {
    /// Base timestamp used to compute sync points.
    base: Instant,

    /// The last timestamp we synced to.
    last_synced_timestamp: Instant,

    /// The refresh rate we've used to compute sync timestamps.
    refresh_interval: Duration,
}

impl FrameTimer {
    pub fn new() -> Self {
        let now = Instant::now();
        Self { base: now, last_synced_timestamp: now, refresh_interval: Duration::ZERO }
    }

    /// Compute the delay that we should use to achieve the target frame
    /// rate.
    pub fn compute_timeout(&mut self, refresh_interval: Duration) -> Duration {
        let now = Instant::now();

        // Handle refresh rate change.
        if self.refresh_interval != refresh_interval {
            self.base = now;
            self.last_synced_timestamp = now;
            self.refresh_interval = refresh_interval;
            return refresh_interval;
        }

        let next_frame = self.last_synced_timestamp + self.refresh_interval;

        if next_frame < now {
            // Redraw immediately if we haven't drawn in over `refresh_interval` microseconds.
            let elapsed_micros = (now - self.base).as_micros() as u64;
            let refresh_micros = self.refresh_interval.as_micros() as u64;
            self.last_synced_timestamp =
                now - Duration::from_micros(elapsed_micros % refresh_micros);
            Duration::ZERO
        } else {
            // Redraw on the next `refresh_interval` clock tick.
            self.last_synced_timestamp = next_frame;
            next_frame - now
        }
    }
}
