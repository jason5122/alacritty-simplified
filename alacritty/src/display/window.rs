#[cfg(not(any(target_os = "macos", windows)))]
use winit::platform::startup_notify::{
    self, EventLoopExtStartupNotify, WindowBuilderExtStartupNotify,
};

#[cfg(all(not(feature = "x11"), not(any(target_os = "macos", windows))))]
use winit::platform::wayland::WindowBuilderExtWayland;

#[rustfmt::skip]
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use {
    std::io::Cursor,
    winit::platform::x11::{WindowBuilderExtX11, EventLoopWindowTargetExtX11},
    glutin::platform::x11::X11VisualInfo,
    winit::window::Icon,
    png::Decoder,
};

use std::fmt::{self, Display, Formatter};

#[cfg(target_os = "macos")]
use {
    cocoa::appkit::NSColorSpace,
    cocoa::base::{id, nil},
    objc::{msg_send, sel, sel_impl},
};

use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};
use winit::dpi::PhysicalSize;
use winit::event_loop::EventLoopWindowTarget;
use winit::monitor::MonitorHandle;
#[cfg(windows)]
use winit::platform::windows::IconExtWindows;
use winit::window::{
    CursorIcon, Theme as WinitTheme, Window as WinitWindow, WindowBuilder, WindowId,
};

/// Window icon for `_NET_WM_ICON` property.
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
static WINDOW_ICON: &[u8] = include_bytes!("../../extra/logo/compat/alacritty-term.png");

/// This should match the definition of IDI_ICON from `alacritty.rc`.
#[cfg(windows)]
const IDI_ICON: u16 = 0x101;

/// Window errors.
#[derive(Debug)]
pub enum Error {
    /// Error creating the window.
    WindowCreation(winit::error::OsError),

    /// Error dealing with fonts.
    Font(crossfont::Error),
}

/// Result of fallible operations concerning a Window.
type Result<T> = std::result::Result<T, Error>;

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::WindowCreation(err) => err.source(),
            Error::Font(err) => err.source(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::WindowCreation(err) => write!(f, "Error creating GL context; {}", err),
            Error::Font(err) => err.fmt(f),
        }
    }
}

impl From<winit::error::OsError> for Error {
    fn from(val: winit::error::OsError) -> Self {
        Error::WindowCreation(val)
    }
}

impl From<crossfont::Error> for Error {
    fn from(val: crossfont::Error) -> Self {
        Error::Font(val)
    }
}

/// A window which can be used for displaying the terminal.
///
/// Wraps the underlying windowing library to provide a stable API in Alacritty.
pub struct Window {
    /// Flag tracking that we have a frame we can draw.
    pub has_frame: bool,

    /// Cached scale factor for quickly scaling pixel sizes.
    pub scale_factor: f64,

    /// Flag indicating whether redraw was requested.
    pub requested_redraw: bool,

    window: WinitWindow,
}

impl Window {
    /// Create a new window.
    ///
    /// This creates a window and fully initializes a window.
    pub fn new<E>(
        event_loop: &EventLoopWindowTarget<E>,
        #[rustfmt::skip]
        #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
        x11_visual: Option<X11VisualInfo>,
    ) -> Result<Window> {
        #[allow(unused_mut)]
        let mut window_builder = Window::get_platform_window(
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            x11_visual,
        );

        #[cfg(not(any(target_os = "macos", windows)))]
        if let Some(token) = event_loop.read_token_from_env() {
            log::debug!("Activating window with token: {token:?}");
            window_builder = window_builder.with_activation_token(token);

            // Remove the token from the env.
            startup_notify::reset_activation_token_env();
        }

        let window = window_builder
            .with_title("Alacritty Simplified")
            .with_theme(Some(WinitTheme::Light))
            .with_visible(false)
            .with_transparent(true)
            .with_blur(false)
            .with_maximized(true)
            .with_fullscreen(None)
            .build(event_loop)?;

        #[cfg(target_os = "macos")]
        use_srgb_color_space(&window);

        let scale_factor = window.scale_factor();
        println!("Window scale factor: {}", scale_factor);

        Ok(Self { requested_redraw: false, has_frame: true, scale_factor, window })
    }

    #[inline]
    pub fn raw_window_handle(&self) -> RawWindowHandle {
        self.window.raw_window_handle()
    }

    #[inline]
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.window.inner_size()
    }

    #[inline]
    pub fn set_visible(&self, visibility: bool) {
        self.window.set_visible(visibility);
    }

    #[inline]
    pub fn request_redraw(&mut self) {
        if !self.requested_redraw {
            self.requested_redraw = true;
            self.window.request_redraw();
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn get_platform_window(
        #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))] x11_visual: Option<
            X11VisualInfo,
        >,
    ) -> WindowBuilder {
        #[cfg(feature = "x11")]
        let icon = {
            let mut decoder = Decoder::new(Cursor::new(WINDOW_ICON));
            decoder.set_transformations(png::Transformations::normalize_to_color8());
            let mut reader = decoder.read_info().expect("invalid embedded icon");
            let mut buf = vec![0; reader.output_buffer_size()];
            let _ = reader.next_frame(&mut buf);
            Icon::from_rgba(buf, reader.info().width, reader.info().height)
                .expect("invalid embedded icon format")
        };

        let builder = WindowBuilder::new().with_decorations(true);

        #[cfg(feature = "x11")]
        let builder = builder.with_window_icon(Some(icon));

        #[cfg(feature = "x11")]
        let builder = match x11_visual {
            Some(visual) => builder.with_x11_visual(visual.visual_id() as u32),
            None => builder,
        };

        builder
    }

    #[cfg(windows)]
    pub fn get_platform_window() -> WindowBuilder {
        let icon = winit::window::Icon::from_resource(IDI_ICON, None);

        WindowBuilder::new()
            .with_decorations(window_config.decorations != Decorations::None)
            .with_window_icon(icon.ok())
    }

    #[cfg(target_os = "macos")]
    pub fn get_platform_window() -> WindowBuilder {
        WindowBuilder::new()
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Inform windowing system about presenting to the window.
    ///
    /// Should be called right before presenting to the window with e.g. `eglSwapBuffers`.
    pub fn pre_present_notify(&self) {
        self.window.pre_present_notify();
    }

    pub fn current_monitor(&self) -> Option<MonitorHandle> {
        self.window.current_monitor()
    }
}

#[cfg(target_os = "macos")]
fn use_srgb_color_space(window: &WinitWindow) {
    let raw_window = match window.raw_window_handle() {
        RawWindowHandle::AppKit(handle) => handle.ns_window as id,
        _ => return,
    };

    unsafe {
        let _: () = msg_send![raw_window, setColorSpace: NSColorSpace::sRGBColorSpace(nil)];
    }
}
