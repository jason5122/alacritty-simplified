use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, ValueHint};
use log::{self, error};
use serde::{Deserialize, Serialize};

use alacritty_terminal::tty::Options as PtyOptions;

use crate::config::ui_config::Program;
use crate::config::window::{Class, Identity};

/// CLI options for the main Alacritty executable.
#[derive(Parser, Default, Debug)]
#[clap(author, about, version = env!("VERSION"))]
pub struct Options {
    /// Generates ref test.
    #[clap(long)]
    pub ref_test: bool,

    /// X11 window ID to embed Alacritty within (decimal or hexadecimal with "0x" prefix).
    #[clap(long)]
    pub embed: Option<String>,

    /// Specify alternative configuration file [default:
    /// $XDG_CONFIG_HOME/alacritty/alacritty.toml].
    #[cfg(not(any(target_os = "macos", windows)))]
    #[clap(long, value_hint = ValueHint::FilePath)]
    pub config_file: Option<PathBuf>,

    /// Specify alternative configuration file [default: %APPDATA%\alacritty\alacritty.toml].
    #[cfg(windows)]
    #[clap(long, value_hint = ValueHint::FilePath)]
    pub config_file: Option<PathBuf>,

    /// Specify alternative configuration file [default: $HOME/.config/alacritty/alacritty.toml].
    #[cfg(target_os = "macos")]
    #[clap(long, value_hint = ValueHint::FilePath)]
    pub config_file: Option<PathBuf>,

    /// Path for IPC socket creation.
    #[cfg(unix)]
    #[clap(long, value_hint = ValueHint::FilePath)]
    pub socket: Option<PathBuf>,

    /// Reduces the level of verbosity (the min level is -qq).
    #[clap(short, conflicts_with("verbose"), action = ArgAction::Count)]
    quiet: u8,

    /// Increases the level of verbosity (the max level is -vvv).
    #[clap(short, conflicts_with("quiet"), action = ArgAction::Count)]
    verbose: u8,

    /// CLI options for config overrides.
    #[clap(skip)]
    pub config_options: ParsedOptions,

    /// Options which can be passed via IPC.
    #[clap(flatten)]
    pub window_options: WindowOptions,
}

/// Parse the class CLI parameter.
fn parse_class(input: &str) -> Result<Class, String> {
    let (general, instance) = match input.split_once(',') {
        // Warn the user if they've passed too many values.
        Some((_, instance)) if instance.contains(',') => {
            return Err(String::from("Too many parameters"))
        },
        Some((general, instance)) => (general, instance),
        None => (input, input),
    };

    Ok(Class::new(general, instance))
}

/// Window specific cli options which can be passed to new windows via IPC.
#[derive(Serialize, Deserialize, Args, Default, Debug, Clone, PartialEq, Eq)]
pub struct WindowIdentity {
    /// Defines the window title [default: Alacritty].
    #[clap(short = 'T', short_alias('t'), long)]
    pub title: Option<String>,

    /// Defines window class/app_id on X11/Wayland [default: Alacritty].
    #[clap(long, value_name = "general> | <general>,<instance", value_parser = parse_class)]
    pub class: Option<Class>,
}

impl WindowIdentity {
    /// Override the [`WindowIdentity`]'s fields with the [`WindowOptions`].
    pub fn override_identity_config(&self, identity: &mut Identity) {
        if let Some(title) = &self.title {
            identity.title = title.clone();
        }
        if let Some(class) = &self.class {
            identity.class = class.clone();
        }
    }
}

/// Subset of options that we pass to 'create-window' IPC subcommand.
#[derive(Serialize, Deserialize, Args, Default, Clone, Debug, PartialEq, Eq)]
pub struct WindowOptions {
    #[clap(flatten)]
    /// Window options which could be passed via IPC.
    pub window_identity: WindowIdentity,

    #[clap(skip)]
    #[cfg(target_os = "macos")]
    /// The window tabbing identifier to use when building a window.
    pub window_tabbing_id: Option<String>,

    /// Override configuration file options [example: 'cursor.style="Beam"'].
    #[clap(short = 'o', long, num_args = 1..)]
    option: Vec<String>,
}

/// Parameters to the `config` IPC subcommand.
#[cfg(unix)]
#[derive(Args, Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct IpcConfig {
    /// Configuration file options [example: 'cursor.style="Beam"'].
    #[clap(required = true, value_name = "CONFIG_OPTIONS")]
    pub options: Vec<String>,

    /// Window ID for the new config.
    ///
    /// Use `-1` to apply this change to all windows.
    #[clap(short, long, allow_hyphen_values = true, env = "ALACRITTY_WINDOW_ID")]
    pub window_id: Option<i128>,

    /// Clear all runtime configuration changes.
    #[clap(short, long, conflicts_with = "options")]
    pub reset: bool,
}

/// Parsed CLI config overrides.
#[derive(Debug, Default)]
pub struct ParsedOptions {}
