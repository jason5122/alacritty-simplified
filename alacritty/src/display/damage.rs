use std::cmp;
use std::iter::Peekable;

use glutin::surface::Rect;

use alacritty_terminal::index::Point;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::{LineDamageBounds, TermDamageIterator};

use crate::display::SizeInfo;

/// State of the damage tracking for the [`Display`].
///
/// [`Display`]: crate::display::Display
#[derive(Debug)]
pub struct DamageTracker {
    /// Position of the previously drawn Vi cursor.
    pub old_vi_cursor: Option<Point<usize>>,
    /// The location of the old selection.
    pub old_selection: Option<SelectionRange>,
    /// Highlight damage submitted for the compositor.
    pub debug: bool,

    /// The damage for the frames.
    frames: [FrameDamage; 2],
    screen_lines: usize,
    columns: usize,
}

impl DamageTracker {
    pub fn new(screen_lines: usize, columns: usize) -> Self {
        let mut tracker = Self {
            columns,
            screen_lines,
            debug: false,
            old_vi_cursor: None,
            old_selection: None,
            frames: Default::default(),
        };
        tracker.resize(screen_lines, columns);
        tracker
    }

    #[inline]
    #[must_use]
    pub fn frame(&mut self) -> &mut FrameDamage {
        &mut self.frames[0]
    }

    /// Advance to the next frame resetting the state for the active frame.
    #[inline]
    pub fn swap_damage(&mut self) {
        let screen_lines = self.screen_lines;
        let columns = self.columns;
        self.frame().reset(screen_lines, columns);
        self.frames.swap(0, 1);
    }

    /// Resize the damage information in the tracker.
    pub fn resize(&mut self, screen_lines: usize, columns: usize) {
        self.screen_lines = screen_lines;
        self.columns = columns;
        for frame in &mut self.frames {
            frame.reset(screen_lines, columns);
        }
        self.frame().full = true;
    }
}

/// Damage state for the rendering frame.
#[derive(Debug, Default)]
pub struct FrameDamage {
    /// The entire frame needs to be redrawn.
    full: bool,
    /// Terminal lines damaged in the given frame.
    lines: Vec<LineDamageBounds>,
    /// Rectangular regions damage in the given frame.
    rects: Vec<Rect>,
}

impl FrameDamage {
    /// Mark the frame as fully damaged.
    #[inline]
    pub fn mark_fully_damaged(&mut self) {
        self.full = true;
    }

    fn reset(&mut self, num_lines: usize, num_cols: usize) {
        self.full = false;
        self.rects.clear();
        self.lines.clear();
        self.lines.reserve(num_lines);
        for line in 0..num_lines {
            self.lines.push(LineDamageBounds::undamaged(line, num_cols));
        }
    }
}

/// Iterator which converts `alacritty_terminal` damage information into renderer damaged rects.
struct RenderDamageIterator<'a> {
    damaged_lines: Peekable<TermDamageIterator<'a>>,
    size_info: &'a SizeInfo<u32>,
}

impl<'a> RenderDamageIterator<'a> {
    #[inline]
    fn rect_for_line(&self, line_damage: LineDamageBounds) -> Rect {
        let size_info = &self.size_info;
        let y_top = size_info.height() - size_info.padding_y();
        let x = size_info.padding_x() + line_damage.left as u32 * size_info.cell_width();
        let y = y_top - (line_damage.line + 1) as u32 * size_info.cell_height();
        let width = (line_damage.right - line_damage.left + 1) as u32 * size_info.cell_width();
        Rect::new(x as i32, y as i32, width as i32, size_info.cell_height() as i32)
    }

    // Make sure to damage near cells to include wide chars.
    #[inline]
    fn overdamage(size_info: &SizeInfo<u32>, mut rect: Rect) -> Rect {
        rect.x = (rect.x - size_info.cell_width() as i32).max(0);
        rect.width = cmp::min(
            size_info.width() as i32 - rect.x,
            rect.width + 2 * size_info.cell_width() as i32,
        );
        rect.y = (rect.y - size_info.cell_height() as i32 / 2).max(0);
        rect.height = cmp::min(
            size_info.height() as i32 - rect.y,
            rect.height + size_info.cell_height() as i32,
        );

        rect
    }
}

impl<'a> Iterator for RenderDamageIterator<'a> {
    type Item = Rect;

    fn next(&mut self) -> Option<Rect> {
        let line = self.damaged_lines.next()?;
        let size_info = &self.size_info;
        let mut total_damage_rect = Self::overdamage(size_info, self.rect_for_line(line));

        // Merge rectangles which overlap with each other.
        while let Some(line) = self.damaged_lines.peek().copied() {
            let next_rect = Self::overdamage(size_info, self.rect_for_line(line));
            if !rects_overlap(total_damage_rect, next_rect) {
                break;
            }

            total_damage_rect = merge_rects(total_damage_rect, next_rect);
            let _ = self.damaged_lines.next();
        }

        Some(total_damage_rect)
    }
}

/// Check if two given [`glutin::surface::Rect`] overlap.
fn rects_overlap(lhs: Rect, rhs: Rect) -> bool {
    !(
        // `lhs` is left of `rhs`.
        lhs.x + lhs.width < rhs.x
        // `lhs` is right of `rhs`.
        || rhs.x + rhs.width < lhs.x
        // `lhs` is below `rhs`.
        || lhs.y + lhs.height < rhs.y
        // `lhs` is above `rhs`.
        || rhs.y + rhs.height < lhs.y
    )
}

/// Merge two [`glutin::surface::Rect`] by producing the smallest rectangle that contains both.
#[inline]
fn merge_rects(lhs: Rect, rhs: Rect) -> Rect {
    let left_x = cmp::min(lhs.x, rhs.x);
    let right_x = cmp::max(lhs.x + lhs.width, rhs.x + rhs.width);
    let y_top = cmp::max(lhs.y + lhs.height, rhs.y + rhs.height);
    let y_bottom = cmp::min(lhs.y, rhs.y);
    Rect::new(left_x, y_bottom, right_x - left_x, y_top - y_bottom)
}
