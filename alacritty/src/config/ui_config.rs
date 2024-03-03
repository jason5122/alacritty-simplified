use std::cell::RefCell;
use std::error::Error;
use std::rc::Rc;

use alacritty_config::SerdeReplace;
use alacritty_terminal::tty::Shell;
use serde::{self, Deserialize, Deserializer};

use alacritty_config_derive::{ConfigDeserialize, SerdeReplace};

/// A delta for a point in a 2 dimensional plane.
#[derive(ConfigDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delta<T: Default> {
    /// Horizontal change.
    pub x: T,
    /// Vertical change.
    pub y: T,
}
