use std::collections::VecDeque;

use unicode_width::UnicodeWidthChar;

use alacritty_terminal::grid::Dimensions;

use crate::display::SizeInfo;

pub const CLOSE_BUTTON_TEXT: &str = "[X]";
const CLOSE_BUTTON_PADDING: usize = 1;
const MIN_FREE_LINES: usize = 3;
const TRUNCATED_MESSAGE: &str = "[MESSAGE TRUNCATED]";

/// Message for display in the MessageBuffer.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct Message {
    text: String,
    ty: MessageType,
    target: Option<String>,
}

/// Purpose of the message.
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum MessageType {
    /// A message represents an error.
    Error,

    /// A message represents a warning.
    Warning,
}

impl Message {
    /// Create a new message.
    pub fn new(text: String, ty: MessageType) -> Message {
        Message { text, ty, target: None }
    }

    /// Formatted message text lines.
    pub fn text(&self, size_info: &SizeInfo) -> Vec<String> {
        let num_cols = size_info.columns();
        let total_lines =
            (size_info.height() - 2. * size_info.padding_y()) / size_info.cell_height();
        let max_lines = (total_lines as usize).saturating_sub(MIN_FREE_LINES);
        let button_len = CLOSE_BUTTON_TEXT.chars().count();

        // Split line to fit the screen.
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut line_len = 0;
        for c in self.text.trim().chars() {
            if c == '\n'
                || line_len == num_cols
                // Keep space in first line for button.
                || (lines.is_empty()
                    && num_cols >= button_len
                    && line_len == num_cols.saturating_sub(button_len + CLOSE_BUTTON_PADDING))
            {
                let is_whitespace = c.is_whitespace();

                // Attempt to wrap on word boundaries.
                let mut new_line = String::new();
                if let Some(index) = line.rfind(char::is_whitespace).filter(|_| !is_whitespace) {
                    let split = line.split_off(index + 1);
                    line.pop();
                    new_line = split;
                }

                lines.push(Self::pad_text(line, num_cols));
                line = new_line;
                line_len = line.chars().count();

                // Do not append whitespace at EOL.
                if is_whitespace {
                    continue;
                }
            }

            line.push(c);

            // Reserve extra column for fullwidth characters.
            let width = c.width().unwrap_or(0);
            if width == 2 {
                line.push(' ');
            }

            line_len += width
        }
        lines.push(Self::pad_text(line, num_cols));

        // Truncate output if it's too long.
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if TRUNCATED_MESSAGE.len() <= num_cols {
                if let Some(line) = lines.iter_mut().last() {
                    *line = Self::pad_text(TRUNCATED_MESSAGE.into(), num_cols);
                }
            }
        }

        // Append close button to first line.
        if button_len <= num_cols {
            if let Some(line) = lines.get_mut(0) {
                line.truncate(num_cols - button_len);
                line.push_str(CLOSE_BUTTON_TEXT);
            }
        }

        lines
    }

    /// Message type.
    #[inline]
    pub fn ty(&self) -> MessageType {
        self.ty
    }

    /// Message target.
    #[inline]
    pub fn target(&self) -> Option<&String> {
        self.target.as_ref()
    }

    /// Update the message target.
    #[inline]
    pub fn set_target(&mut self, target: String) {
        self.target = Some(target);
    }

    /// Right-pad text to fit a specific number of columns.
    #[inline]
    fn pad_text(mut text: String, num_cols: usize) -> String {
        let padding_len = num_cols.saturating_sub(text.chars().count());
        text.extend(vec![' '; padding_len]);
        text
    }
}

/// Storage for message bar.
#[derive(Debug, Default)]
pub struct MessageBuffer {
    messages: VecDeque<Message>,
}

impl MessageBuffer {
    /// Check if there are any messages queued.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Current message.
    #[inline]
    pub fn message(&self) -> Option<&Message> {
        self.messages.front()
    }

    /// Remove the currently visible message.
    #[inline]
    pub fn pop(&mut self) {
        // Remove the message itself.
        let msg = self.messages.pop_front();

        // Remove all duplicates.
        if let Some(msg) = msg {
            self.messages = self.messages.drain(..).filter(|m| m != &msg).collect();
        }
    }

    /// Add a new message to the queue.
    #[inline]
    pub fn push(&mut self, message: Message) {
        self.messages.push_back(message);
    }

    /// Check whether the message is already queued in the message bar.
    #[inline]
    pub fn is_queued(&self, message: &Message) -> bool {
        self.messages.contains(message)
    }
}
