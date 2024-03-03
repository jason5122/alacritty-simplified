use std::ops::Deref;

use alacritty_terminal::vte::ansi::Rgb as VteRgb;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Default)]
pub struct Rgb(pub VteRgb);

impl Rgb {
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self(VteRgb { r, g, b })
    }

    #[inline]
    pub fn as_tuple(self) -> (u8, u8, u8) {
        (self.0.r, self.0.g, self.0.b)
    }
}

impl Deref for Rgb {
    type Target = VteRgb;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
