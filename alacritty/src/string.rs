use std::iter::Skip;
use std::str::Chars;

use unicode_width::UnicodeWidthChar;

/// The action performed by [`StrShortener`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAction {
    /// Yield a spacer.
    Spacer,
    /// Terminate state reached.
    Terminate,
    /// Yield a character.
    Char,
}

/// Iterator that yield shortened version of the text.
pub struct StrShortener<'a> {
    chars: Skip<Chars<'a>>,
    accumulated_len: usize,
    max_width: usize,
    shortener: Option<char>,
    text_action: TextAction,
}

impl<'a> Iterator for StrShortener<'a> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        match self.text_action {
            TextAction::Spacer => {
                self.text_action = TextAction::Char;
                Some(' ')
            },
            TextAction::Terminate => {
                // We've reached the termination state.
                None
            },
            TextAction::Char => {
                let ch = self.chars.next()?;
                let ch_width = ch.width().unwrap_or(1);

                // Advance width.
                self.accumulated_len += ch_width;

                if self.accumulated_len > self.max_width {
                    self.text_action = TextAction::Terminate;
                    return self.shortener;
                } else if self.accumulated_len == self.max_width && self.shortener.is_some() {
                    // Check if we have a next char.
                    let has_next = self.chars.clone().next().is_some();

                    // We should terminate after that.
                    self.text_action = TextAction::Terminate;

                    return has_next.then(|| self.shortener.unwrap()).or(Some(ch));
                }

                // Add a spacer for wide character.
                if ch_width == 2 {
                    self.text_action = TextAction::Spacer;
                }

                Some(ch)
            },
        }
    }
}
