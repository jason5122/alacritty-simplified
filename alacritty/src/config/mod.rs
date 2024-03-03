use std::fmt::{self, Display, Formatter};
use std::{env, io};

use serde_yaml::Error as YamlError;
use toml::de::Error as TomlError;
use toml::ser::Error as TomlSeError;

pub mod color;
pub mod ui_config;
pub mod window;

mod bindings;

pub use crate::config::bindings::Action;
pub use crate::config::ui_config::UiConfig;

/// Errors occurring during config loading.
#[derive(Debug)]
pub enum Error {
    /// Config file not found.
    NotFound,

    /// Couldn't read $HOME environment variable.
    ReadingEnvHome(env::VarError),

    /// io error reading file.
    Io(io::Error),

    /// Invalid toml.
    Toml(TomlError),

    /// Failed toml serialization.
    TomlSe(TomlSeError),

    /// Invalid yaml.
    Yaml(YamlError),
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::NotFound => None,
            Error::ReadingEnvHome(err) => err.source(),
            Error::Io(err) => err.source(),
            Error::Toml(err) => err.source(),
            Error::TomlSe(err) => err.source(),
            Error::Yaml(err) => err.source(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "Unable to locate config file"),
            Error::ReadingEnvHome(err) => {
                write!(f, "Unable to read $HOME environment variable: {}", err)
            },
            Error::Io(err) => write!(f, "Error reading config file: {}", err),
            Error::Toml(err) => write!(f, "Config error: {}", err),
            Error::TomlSe(err) => write!(f, "Yaml conversion error: {}", err),
            Error::Yaml(err) => write!(f, "Config error: {}", err),
        }
    }
}

impl From<env::VarError> for Error {
    fn from(val: env::VarError) -> Self {
        Error::ReadingEnvHome(val)
    }
}

impl From<io::Error> for Error {
    fn from(val: io::Error) -> Self {
        if val.kind() == io::ErrorKind::NotFound {
            Error::NotFound
        } else {
            Error::Io(val)
        }
    }
}

impl From<TomlError> for Error {
    fn from(val: TomlError) -> Self {
        Error::Toml(val)
    }
}

impl From<TomlSeError> for Error {
    fn from(val: TomlSeError) -> Self {
        Error::TomlSe(val)
    }
}

impl From<YamlError> for Error {
    fn from(val: YamlError) -> Self {
        Error::Yaml(val)
    }
}

#[cfg(windows)]
pub fn installed_config(suffix: &str) -> Option<PathBuf> {
    let file_name = format!("alacritty.{suffix}");
    dirs::config_dir().map(|path| path.join("alacritty").join(file_name)).filter(|new| new.exists())
}
