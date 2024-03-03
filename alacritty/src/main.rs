//! Alacritty - The GPU Enhanced Terminal.

#![warn(rust_2018_idioms, future_incompatible)]
#![deny(clippy::all, clippy::if_not_else, clippy::enum_glob_use)]
#![cfg_attr(clippy, deny(warnings))]
// With the default subsystem, 'console', windows creates an additional console
// window for the program.
// This is silently ignored on non-windows systems.
// See https://msdn.microsoft.com/en-us/library/4cc7ya5b.aspx for more details.
#![windows_subsystem = "windows"]

#[cfg(not(any(feature = "x11", feature = "wayland", target_os = "macos", windows)))]
compile_error!(r#"at least one of the "x11"/"wayland" features must be enabled"#);

use std::error::Error;

#[cfg(windows)]
use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole, ATTACH_PARENT_PROCESS};
use winit::event_loop::EventLoopBuilder as WinitEventLoopBuilder;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use winit::platform::x11::EventLoopWindowTargetExtX11;

mod display;
mod event;
mod input;
#[cfg(target_os = "macos")]
mod macos;
mod renderer;
mod scheduler;
mod window_context;

mod gl {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}

use crate::event::{Event, Processor};
#[cfg(target_os = "macos")]
use crate::macos::locale;

fn main() -> Result<(), Box<dyn Error>> {
    alacritty()?;
    Ok(())
}

/// Run main Alacritty entrypoint.
///
/// Creates a window, the terminal state, PTY, I/O event loop, input processor,
/// config change monitor, and runs the main display loop.
fn alacritty() -> Result<(), Box<dyn Error>> {
    // Setup winit event loop.
    let window_event_loop = WinitEventLoopBuilder::<Event>::with_user_event().build()?;

    // Set macOS locale.
    #[cfg(target_os = "macos")]
    locale::set_locale_environment();

    // Event processor.
    let mut processor = Processor::new(&window_event_loop);

    // Start event loop and block until shutdown.
    let result = processor.run(window_event_loop);

    // This explicit drop is needed for Windows, ConPTY backend. Otherwise a deadlock can occur.
    // The cause:
    //   - Drop for ConPTY will deadlock if the conout pipe has already been dropped
    //   - ConPTY is dropped when the last of processor and window context are dropped, because both
    //     of them own an Arc<ConPTY>
    //
    // The fix is to ensure that processor is dropped first. That way, when window context (i.e.
    // PTY) is dropped, it can ensure ConPTY is dropped before the conout pipe in the PTY drop
    // order.
    //
    // FIXME: Change PTY API to enforce the correct drop order with the typesystem.
    drop(processor);

    // Without explicitly detaching the console cmd won't redraw it's prompt.
    #[cfg(windows)]
    unsafe {
        FreeConsole();
    }

    result
}
