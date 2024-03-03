use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, ValueHint};
use serde::{Deserialize, Serialize};

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
}
