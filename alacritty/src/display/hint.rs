use std::cmp::Reverse;
use std::collections::HashSet;
use std::iter;

use ahash::RandomState;

use alacritty_terminal::grid::{BidirectionalIterator, Dimensions};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point};
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::Term;

use crate::config::ui_config::{Hint, HintAction};

/// Maximum number of linewraps followed outside of the viewport during search highlighting.
pub const MAX_SEARCH_LINES: usize = 100;

/// Percentage of characters in the hints alphabet used for the last character.
const HINT_SPLIT_PERCENTAGE: f32 = 0.5;

/// Keyboard regex hint state.
pub struct HintState {
    /// Hint currently in use.
    hint: Option<Hint>,

    /// Alphabet for hint labels.
    alphabet: String,

    /// Visible matches.
    matches: Vec<Match>,

    /// Key label for each visible match.
    labels: Vec<Vec<char>>,

    /// Keys pressed for hint selection.
    keys: Vec<char>,
}

impl HintState {
    /// Initialize an inactive hint state.
    pub fn new<S: Into<String>>(alphabet: S) -> Self {
        Self {
            alphabet: alphabet.into(),
            hint: Default::default(),
            matches: Default::default(),
            labels: Default::default(),
            keys: Default::default(),
        }
    }

    /// Cancel the hint highlighting process.
    fn stop(&mut self) {
        self.matches.clear();
        self.labels.clear();
        self.keys.clear();
        self.hint = None;
    }

    /// Update the visible hint matches and key labels.
    pub fn update_matches<T>(&mut self, term: &Term<T>) {
        let hint = match self.hint.as_mut() {
            Some(hint) => hint,
            None => return,
        };

        // Clear current matches.
        self.matches.clear();

        // Add escape sequence hyperlinks.
        if hint.content.hyperlinks {
            self.matches.extend(visible_unique_hyperlinks_iter(term));
        }

        // Add visible regex matches.
        if let Some(regex) = hint.content.regex.as_ref() {
            regex.with_compiled(|regex| {
                let matches = visible_regex_match_iter(term, regex);

                // Apply post-processing and search for sub-matches if necessary.
                if hint.post_processing {
                    let mut matches = matches.collect::<Vec<_>>();
                    self.matches.extend(matches.drain(..).flat_map(|rm| {
                        HintPostProcessor::new(term, regex, rm).collect::<Vec<_>>()
                    }));
                } else {
                    self.matches.extend(matches);
                }
            });
        }

        // Cancel highlight with no visible matches.
        if self.matches.is_empty() {
            self.stop();
            return;
        }

        // Sort and dedup ranges. Currently overlapped but not exactly same ranges are kept.
        self.matches.sort_by_key(|bounds| (*bounds.start(), Reverse(*bounds.end())));
        self.matches.dedup_by_key(|bounds| *bounds.start());

        let mut generator = HintLabels::new(&self.alphabet, HINT_SPLIT_PERCENTAGE);
        let match_count = self.matches.len();
        let keys_len = self.keys.len();

        // Get the label for each match.
        self.labels.resize(match_count, Vec::new());
        for i in (0..match_count).rev() {
            let mut label = generator.next();
            if label.len() >= keys_len && label[..keys_len] == self.keys[..] {
                self.labels[i] = label.split_off(keys_len);
            } else {
                self.labels[i] = Vec::new();
            }
        }
    }

    /// Handle keyboard input during hint selection.
    pub fn keyboard_input<T>(&mut self, term: &Term<T>, c: char) -> Option<HintMatch> {
        match c {
            // Use backspace to remove the last character pressed.
            '\x08' | '\x1f' => {
                self.keys.pop();
            },
            // Cancel hint highlighting on ESC/Ctrl+c.
            '\x1b' | '\x03' => self.stop(),
            _ => (),
        }

        // Update the visible matches.
        self.update_matches(term);

        let hint = self.hint.as_ref()?;

        // Find the last label starting with the input character.
        let mut labels = self.labels.iter().enumerate().rev();
        let (index, label) = labels.find(|(_, label)| !label.is_empty() && label[0] == c)?;

        // Check if the selected label is fully matched.
        if label.len() == 1 {
            let bounds = self.matches[index].clone();
            let action = hint.action.clone();

            // Exit hint mode unless it requires explicit dismissal.
            if hint.persist {
                self.keys.clear();
            } else {
                self.stop();
            }

            // Hyperlinks take precedence over regex matches.
            let hyperlink = term.grid()[*bounds.start()].hyperlink();
            Some(HintMatch { action, bounds, hyperlink })
        } else {
            // Store character to preserve the selection.
            self.keys.push(c);

            None
        }
    }

    /// Hint key labels.
    pub fn labels(&self) -> &Vec<Vec<char>> {
        &self.labels
    }

    /// Visible hint regex matches.
    pub fn matches(&self) -> &[Match] {
        &self.matches
    }
}

/// Hint match which was selected by the user.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct HintMatch {
    /// Action for handling the text.
    action: HintAction,

    /// Terminal range matching the hint.
    bounds: Match,

    hyperlink: Option<Hyperlink>,
}

impl HintMatch {
    #[inline]
    pub fn action(&self) -> &HintAction {
        &self.action
    }

    #[inline]
    pub fn bounds(&self) -> &Match {
        &self.bounds
    }

    pub fn hyperlink(&self) -> Option<&Hyperlink> {
        self.hyperlink.as_ref()
    }
}

/// Generator for creating new hint labels.
struct HintLabels {
    /// Full character set available.
    alphabet: Vec<char>,

    /// Alphabet indices for the next label.
    indices: Vec<usize>,

    /// Point separating the alphabet's head and tail characters.
    ///
    /// To make identification of the tail character easy, part of the alphabet cannot be used for
    /// any other position.
    ///
    /// All characters in the alphabet before this index will be used for the last character, while
    /// the rest will be used for everything else.
    split_point: usize,
}

impl HintLabels {
    /// Create a new label generator.
    ///
    /// The `split_ratio` should be a number between 0.0 and 1.0 representing the percentage of
    /// elements in the alphabet which are reserved for the tail of the hint label.
    fn new(alphabet: impl Into<String>, split_ratio: f32) -> Self {
        let alphabet: Vec<char> = alphabet.into().chars().collect();
        let split_point = ((alphabet.len() - 1) as f32 * split_ratio.min(1.)) as usize;

        Self { indices: vec![0], split_point, alphabet }
    }

    /// Get the characters for the next label.
    fn next(&mut self) -> Vec<char> {
        let characters = self.indices.iter().rev().map(|index| self.alphabet[*index]).collect();
        self.increment();
        characters
    }

    /// Increment the character sequence.
    fn increment(&mut self) {
        // Increment the last character; if it's not at the split point we're done.
        let tail = &mut self.indices[0];
        if *tail < self.split_point {
            *tail += 1;
            return;
        }
        *tail = 0;

        // Increment all other characters in reverse order.
        let alphabet_len = self.alphabet.len();
        for index in self.indices.iter_mut().skip(1) {
            if *index + 1 == alphabet_len {
                // Reset character and move to the next if it's already at the limit.
                *index = self.split_point + 1;
            } else {
                // If the character can be incremented, we're done.
                *index += 1;
                return;
            }
        }

        // Extend the sequence with another character when nothing could be incremented.
        self.indices.push(self.split_point + 1);
    }
}

/// Iterate over all visible regex matches.
pub fn visible_regex_match_iter<'a, T>(
    term: &'a Term<T>,
    regex: &'a mut RegexSearch,
) -> impl Iterator<Item = Match> + 'a {
    let viewport_start = Line(-(term.grid().display_offset() as i32));
    let viewport_end = viewport_start + term.bottommost_line();
    let mut start = term.line_search_left(Point::new(viewport_start, Column(0)));
    let mut end = term.line_search_right(Point::new(viewport_end, Column(0)));
    start.line = start.line.max(viewport_start - MAX_SEARCH_LINES);
    end.line = end.line.min(viewport_end + MAX_SEARCH_LINES);

    RegexIter::new(start, end, Direction::Right, term, regex)
        .skip_while(move |rm| rm.end().line < viewport_start)
        .take_while(move |rm| rm.start().line <= viewport_end)
}

/// Iterate over all visible hyperlinks, yanking only unique ones.
pub fn visible_unique_hyperlinks_iter<T>(term: &Term<T>) -> impl Iterator<Item = Match> + '_ {
    let mut display_iter = term.grid().display_iter().peekable();

    // Avoid creating hints for the same hyperlinks, but from a different places.
    let mut unique_hyperlinks = HashSet::<Hyperlink, RandomState>::default();

    iter::from_fn(move || {
        // Find the start of the next unique hyperlink.
        let (cell, hyperlink) = display_iter.find_map(|cell| {
            let hyperlink = cell.hyperlink()?;
            (!unique_hyperlinks.contains(&hyperlink)).then(|| {
                unique_hyperlinks.insert(hyperlink.clone());
                (cell, hyperlink)
            })
        })?;

        let start = cell.point;
        let mut end = start;

        // Find the end bound of just found unique hyperlink.
        while let Some(next_cell) = display_iter.peek() {
            // Cell at display iter doesn't match, yield the hyperlink and start over with
            // `find_map`.
            if next_cell.hyperlink().as_ref() != Some(&hyperlink) {
                break;
            }

            // Advance to the next cell.
            end = next_cell.point;
            let _ = display_iter.next();
        }

        Some(start..=end)
    })
}

/// Iterator over all post-processed matches inside an existing hint match.
struct HintPostProcessor<'a, T> {
    /// Regex search DFAs.
    regex: &'a mut RegexSearch,

    /// Terminal reference.
    term: &'a Term<T>,

    /// Next hint match in the iterator.
    next_match: Option<Match>,

    /// Start point for the next search.
    start: Point,

    /// End point for the hint match iterator.
    end: Point,
}

impl<'a, T> HintPostProcessor<'a, T> {
    /// Create a new iterator for an unprocessed match.
    fn new(term: &'a Term<T>, regex: &'a mut RegexSearch, regex_match: Match) -> Self {
        let mut post_processor = Self {
            next_match: None,
            start: *regex_match.start(),
            end: *regex_match.end(),
            term,
            regex,
        };

        // Post-process the first hint match.
        post_processor.next_processed_match(regex_match);

        post_processor
    }

    /// Apply some hint post processing heuristics.
    ///
    /// This will check the end of the hint and make it shorter if certain characters are determined
    /// to be unlikely to be intentionally part of the hint.
    ///
    /// This is most useful for identifying URLs appropriately.
    fn hint_post_processing(&self, regex_match: &Match) -> Option<Match> {
        let mut iter = self.term.grid().iter_from(*regex_match.start());

        let mut c = iter.cell().c;

        // Truncate uneven number of brackets.
        let end = *regex_match.end();
        let mut open_parents = 0;
        let mut open_brackets = 0;
        loop {
            match c {
                '(' => open_parents += 1,
                '[' => open_brackets += 1,
                ')' => {
                    if open_parents == 0 {
                        iter.prev();
                        break;
                    } else {
                        open_parents -= 1;
                    }
                },
                ']' => {
                    if open_brackets == 0 {
                        iter.prev();
                        break;
                    } else {
                        open_brackets -= 1;
                    }
                },
                _ => (),
            }

            if iter.point() == end {
                break;
            }

            match iter.next() {
                Some(indexed) => c = indexed.cell.c,
                None => break,
            }
        }

        // Truncate trailing characters which are likely to be delimiters.
        let start = *regex_match.start();
        while iter.point() != start {
            if !matches!(c, '.' | ',' | ':' | ';' | '?' | '!' | '(' | '[' | '\'') {
                break;
            }

            match iter.prev() {
                Some(indexed) => c = indexed.cell.c,
                None => break,
            }
        }

        if start > iter.point() {
            None
        } else {
            Some(start..=iter.point())
        }
    }

    /// Loop over submatches until a non-empty post-processed match is found.
    fn next_processed_match(&mut self, mut regex_match: Match) {
        self.next_match = loop {
            if let Some(next_match) = self.hint_post_processing(&regex_match) {
                self.start = next_match.end().add(self.term, Boundary::Grid, 1);
                break Some(next_match);
            }

            self.start = regex_match.start().add(self.term, Boundary::Grid, 1);
            if self.start > self.end {
                return;
            }

            match self.term.regex_search_right(self.regex, self.start, self.end) {
                Some(rm) => regex_match = rm,
                None => return,
            }
        };
    }
}

impl<'a, T> Iterator for HintPostProcessor<'a, T> {
    type Item = Match;

    fn next(&mut self) -> Option<Self::Item> {
        let next_match = self.next_match.take()?;

        if self.start <= self.end {
            if let Some(rm) = self.term.regex_search_right(self.regex, self.start, self.end) {
                self.next_processed_match(rm);
            }
        }

        Some(next_match)
    }
}
