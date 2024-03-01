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

use std::env;
use std::error::Error;

#[cfg(windows)]
use windows_sys::Win32::System::Console::{AttachConsole, FreeConsole, ATTACH_PARENT_PROCESS};
use winit::event_loop::EventLoopBuilder as WinitEventLoopBuilder;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use winit::platform::x11::EventLoopWindowTargetExtX11;

use alacritty_terminal::tty;

mod cli;
mod clipboard;
mod config;
mod daemon;
mod display;
mod event;
mod input;
mod logging;
#[cfg(target_os = "macos")]
mod macos;
mod message_bar;
mod renderer;
mod scheduler;
mod string;
mod window_context;

mod gl {
    #![allow(clippy::all)]
    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}

use crate::cli::Options;
use crate::config::{monitor, UiConfig};
use crate::event::{Event, Processor};
#[cfg(target_os = "macos")]
use crate::macos::locale;

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::new();
    alacritty(options)?;

    Ok(())
}

/// Run main Alacritty entrypoint.
///
/// Creates a window, the terminal state, PTY, I/O event loop, input processor,
/// config change monitor, and runs the main display loop.
fn alacritty(options: Options) -> Result<(), Box<dyn Error>> {
    // Setup winit event loop.
    let window_event_loop = WinitEventLoopBuilder::<Event>::with_user_event().build()?;

    let config = UiConfig::default();

    // Set tty environment variables.
    tty::setup_env();

    // Set env vars from config.
    for (key, value) in config.env.iter() {
        env::set_var(key, value);
    }

    // Switch to home directory.
    #[cfg(target_os = "macos")]
    env::set_current_dir(home::home_dir().unwrap()).unwrap();

    // Set macOS locale.
    #[cfg(target_os = "macos")]
    locale::set_locale_environment();

    // Create a config monitor when config was loaded from path.
    //
    // The monitor watches the config file for changes and reloads it. Pending
    // config changes are processed in the main loop.
    if config.live_config_reload {
        monitor::watch(config.config_paths.clone(), window_event_loop.create_proxy());
    }

    // Event processor.
    let window_options = options.window_options.clone();
    let mut processor = Processor::new(config, &window_event_loop);

    // Start event loop and block until shutdown.
    let result = processor.run(window_event_loop, window_options);

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
