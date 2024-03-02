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
    /// Yield a shortener.
    Shortener,
    /// Yield a character.
    Char,
}

/// The direction which we should shorten.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShortenDirection {
    /// Shorten to the start of the string.
    Left,

    /// Shorten to the end of the string.
    Right,
}

/// Iterator that yield shortened version of the text.
pub struct StrShortener<'a> {
    chars: Skip<Chars<'a>>,
    accumulated_len: usize,
    max_width: usize,
    direction: ShortenDirection,
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
            TextAction::Shortener => {
                // When we shorten from the left we yield the shortener first and process the rest.
                self.text_action = if self.direction == ShortenDirection::Left {
                    TextAction::Char
                } else {
                    TextAction::Terminate
                };

                // Consume the shortener to avoid yielding it later when shortening left.
                self.shortener.take()
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
