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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn into_shortened_with_shortener() {
        let s = "Hello";
        let len = s.chars().count();
        assert_eq!(
            "",
            StrShortener::new("", 1, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".",
            StrShortener::new(s, 1, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".",
            StrShortener::new(s, 1, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            "H.",
            StrShortener::new(s, 2, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".o",
            StrShortener::new(s, 2, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            s,
            &StrShortener::new(s, len * 2, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            s,
            &StrShortener::new(s, len * 2, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        let s = "ちはP";
        let len = 2 + 2 + 1;
        assert_eq!(
            ".",
            &StrShortener::new(s, 1, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            &".",
            &StrShortener::new(s, 1, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".",
            &StrShortener::new(s, 2, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".P",
            &StrShortener::new(s, 2, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            "ち .",
            &StrShortener::new(s, 3, ShortenDirection::Right, Some('.')).collect::<String>()
        );

        assert_eq!(
            ".P",
            &StrShortener::new(s, 3, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            "ち は P",
            &StrShortener::new(s, len * 2, ShortenDirection::Left, Some('.')).collect::<String>()
        );

        assert_eq!(
            "ち は P",
            &StrShortener::new(s, len * 2, ShortenDirection::Right, Some('.')).collect::<String>()
        );
    }

    #[test]
    fn into_shortened_without_shortener() {
        let s = "Hello";
        assert_eq!("", StrShortener::new("", 1, ShortenDirection::Left, None).collect::<String>());

        assert_eq!(
            "H",
            &StrShortener::new(s, 1, ShortenDirection::Right, None).collect::<String>()
        );

        assert_eq!("o", &StrShortener::new(s, 1, ShortenDirection::Left, None).collect::<String>());

        assert_eq!(
            "He",
            &StrShortener::new(s, 2, ShortenDirection::Right, None).collect::<String>()
        );

        assert_eq!(
            "lo",
            &StrShortener::new(s, 2, ShortenDirection::Left, None).collect::<String>()
        );

        assert_eq!(
            &s,
            &StrShortener::new(s, s.len(), ShortenDirection::Right, None).collect::<String>()
        );

        assert_eq!(
            &s,
            &StrShortener::new(s, s.len(), ShortenDirection::Left, None).collect::<String>()
        );

        let s = "こJんにちはP";
        let len = 2 + 1 + 2 + 2 + 2 + 2 + 1;
        assert_eq!("", &StrShortener::new(s, 1, ShortenDirection::Right, None).collect::<String>());

        assert_eq!("P", &StrShortener::new(s, 1, ShortenDirection::Left, None).collect::<String>());

        assert_eq!(
            "こ ",
            &StrShortener::new(s, 2, ShortenDirection::Right, None).collect::<String>()
        );

        assert_eq!("P", &StrShortener::new(s, 2, ShortenDirection::Left, None).collect::<String>());

        assert_eq!(
            "こ J",
            &StrShortener::new(s, 3, ShortenDirection::Right, None).collect::<String>()
        );

        assert_eq!(
            "は P",
            &StrShortener::new(s, 3, ShortenDirection::Left, None).collect::<String>()
        );

        assert_eq!(
            "こ Jん に ち は P",
            &StrShortener::new(s, len, ShortenDirection::Left, None).collect::<String>()
        );

        assert_eq!(
            "こ Jん に ち は P",
            &StrShortener::new(s, len, ShortenDirection::Right, None).collect::<String>()
        );
    }
}
