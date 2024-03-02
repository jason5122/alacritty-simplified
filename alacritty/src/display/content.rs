use std::mem;

use alacritty_terminal::grid::Indexed;
use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::{self, RenderableContent as TerminalContent, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

use crate::config::UiConfig;
use crate::display::color::{CellRgb, List, Rgb, DIM_FACTOR};
use crate::display::SizeInfo;

/// Minimum contrast between a fixed cursor color and the cell's background.
pub const MIN_CURSOR_CONTRAST: f64 = 1.5;

/// Renderable terminal content.
///
/// This provides the terminal cursor and an iterator over all non-empty cells.
pub struct RenderableContent<'a> {
    terminal_content: TerminalContent<'a>,
    cursor: RenderableCursor,
    cursor_shape: CursorShape,
    cursor_point: Point<usize>,
    config: &'a UiConfig,
    colors: &'a List,
    focused_match: Option<&'a Match>,
    size: &'a SizeInfo,
}

impl<'a> RenderableContent<'a> {
    /// Get the RGB value for a color index.
    pub fn color(&self, color: usize) -> Rgb {
        self.terminal_content.colors[color].map(Rgb).unwrap_or(self.colors[color])
    }

    /// Assemble the information required to render the terminal cursor.
    fn renderable_cursor(&mut self, cell: &RenderableCell) -> RenderableCursor {
        // Cursor colors.
        let color = if self.terminal_content.mode.contains(TermMode::VI) {
            self.config.colors.vi_mode_cursor
        } else {
            self.config.colors.cursor
        };
        let cursor_color = self.terminal_content.colors[NamedColor::Cursor]
            .map_or(color.background, |c| CellRgb::Rgb(Rgb(c)));
        let text_color = color.foreground;

        let insufficient_contrast = (!matches!(cursor_color, CellRgb::Rgb(_))
            || !matches!(text_color, CellRgb::Rgb(_)))
            && cell.fg.contrast(*cell.bg) < MIN_CURSOR_CONTRAST;

        // Convert from cell colors to RGB.
        let mut text_color = text_color.color(cell.fg, cell.bg);
        let mut cursor_color = cursor_color.color(cell.fg, cell.bg);

        // Invert cursor color with insufficient contrast to prevent invisible cursors.
        if insufficient_contrast {
            cursor_color = self.config.colors.primary.foreground;
            text_color = self.config.colors.primary.background;
        }

        RenderableCursor {
            is_wide: cell.flags.contains(Flags::WIDE_CHAR),
            shape: self.cursor_shape,
            point: self.cursor_point,
            cursor_color,
            text_color,
        }
    }
}

impl<'a> Iterator for RenderableContent<'a> {
    type Item = RenderableCell;

    /// Gets the next renderable cell.
    ///
    /// Skips empty (background) cells and applies any flags to the cell state
    /// (eg. invert fg and bg colors).
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let cell = self.terminal_content.display_iter.next()?;
            let mut cell = RenderableCell::new(self, cell);

            if self.cursor_point == cell.point {
                // Store the cursor which should be rendered.
                self.cursor = self.renderable_cursor(&cell);
                if self.cursor.shape == CursorShape::Block {
                    cell.fg = self.cursor.text_color;
                    cell.bg = self.cursor.cursor_color;

                    // Since we draw Block cursor by drawing cell below it with a proper color,
                    // we must adjust alpha to make it visible.
                    cell.bg_alpha = 1.;
                }

                return Some(cell);
            } else if !cell.is_empty() && !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                // Skip empty cells and wide char spacers.
                return Some(cell);
            }
        }
    }
}

/// Cell ready for rendering.
#[derive(Clone, Debug)]
pub struct RenderableCell {
    pub character: char,
    pub point: Point<usize>,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bg_alpha: f32,
    pub underline: Rgb,
    pub flags: Flags,
    pub extra: Option<Box<RenderableCellExtra>>,
}

/// Extra storage with rarely present fields for [`RenderableCell`], to reduce the cell size we
/// pass around.
#[derive(Clone, Debug)]
pub struct RenderableCellExtra {
    pub zerowidth: Option<Vec<char>>,
    pub hyperlink: Option<Hyperlink>,
}

impl RenderableCell {
    fn new(content: &mut RenderableContent<'_>, cell: Indexed<&Cell>) -> Self {
        // Lookup RGB values.
        let mut fg = Self::compute_fg_rgb(content, cell.fg, cell.flags);
        let mut bg = Self::compute_bg_rgb(content, cell.bg);

        let mut bg_alpha = if cell.flags.contains(Flags::INVERSE) {
            mem::swap(&mut fg, &mut bg);
            1.0
        } else {
            Self::compute_bg_alpha(content.config, cell.bg)
        };

        let display_offset = content.terminal_content.display_offset;
        let character = cell.c;
        let flags = cell.flags;

        // Apply transparency to all renderable cells if `transparent_background_colors` is set
        if bg_alpha > 0. && content.config.colors.transparent_background_colors {
            bg_alpha = content.config.window_opacity();
        }

        // Convert cell point to viewport position.
        let cell_point = cell.point;
        let point = term::point_to_viewport(display_offset, cell_point).unwrap();

        let underline = cell
            .underline_color()
            .map_or(fg, |underline| Self::compute_fg_rgb(content, underline, flags));

        let zerowidth = cell.zerowidth();
        let hyperlink = cell.hyperlink();

        let extra = (zerowidth.is_some() || hyperlink.is_some()).then(|| {
            Box::new(RenderableCellExtra {
                zerowidth: zerowidth.map(|zerowidth| zerowidth.to_vec()),
                hyperlink,
            })
        });

        RenderableCell { flags, character, bg_alpha, point, fg, bg, underline, extra }
    }

    /// Check if cell contains any renderable content.
    fn is_empty(&self) -> bool {
        self.bg_alpha == 0.
            && self.character == ' '
            && self.extra.is_none()
            && !self.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
    }

    /// Get the RGB color from a cell's foreground color.
    fn compute_fg_rgb(content: &RenderableContent<'_>, fg: Color, flags: Flags) -> Rgb {
        let config = &content.config;
        match fg {
            Color::Spec(rgb) => match flags & Flags::DIM {
                Flags::DIM => {
                    let rgb: Rgb = rgb.into();
                    rgb * DIM_FACTOR
                },
                _ => rgb.into(),
            },
            Color::Named(ansi) => {
                match (config.draw_bold_text_with_bright_colors(), flags & Flags::DIM_BOLD) {
                    // If no bright foreground is set, treat it like the BOLD flag doesn't exist.
                    (_, Flags::DIM_BOLD)
                        if ansi == NamedColor::Foreground
                            && config.colors.primary.bright_foreground.is_none() =>
                    {
                        content.color(NamedColor::DimForeground as usize)
                    },
                    // Draw bold text in bright colors *and* contains bold flag.
                    (true, Flags::BOLD) => content.color(ansi.to_bright() as usize),
                    // Cell is marked as dim and not bold.
                    (_, Flags::DIM) | (false, Flags::DIM_BOLD) => {
                        content.color(ansi.to_dim() as usize)
                    },
                    // None of the above, keep original color..
                    _ => content.color(ansi as usize),
                }
            },
            Color::Indexed(idx) => {
                let idx = match (
                    config.draw_bold_text_with_bright_colors(),
                    flags & Flags::DIM_BOLD,
                    idx,
                ) {
                    (true, Flags::BOLD, 0..=7) => idx as usize + 8,
                    (false, Flags::DIM, 8..=15) => idx as usize - 8,
                    (false, Flags::DIM, 0..=7) => NamedColor::DimBlack as usize + idx as usize,
                    _ => idx as usize,
                };

                content.color(idx)
            },
        }
    }

    /// Get the RGB color from a cell's background color.
    #[inline]
    fn compute_bg_rgb(content: &RenderableContent<'_>, bg: Color) -> Rgb {
        match bg {
            Color::Spec(rgb) => rgb.into(),
            Color::Named(ansi) => content.color(ansi as usize),
            Color::Indexed(idx) => content.color(idx as usize),
        }
    }

    /// Compute background alpha based on cell's original color.
    ///
    /// Since an RGB color matching the background should not be transparent, this is computed
    /// using the named input color, rather than checking the RGB of the background after its color
    /// is computed.
    #[inline]
    fn compute_bg_alpha(config: &UiConfig, bg: Color) -> f32 {
        if bg == Color::Named(NamedColor::Background) {
            0.
        } else if config.colors.transparent_background_colors {
            config.window_opacity()
        } else {
            1.
        }
    }
}

/// Cursor storing all information relevant for rendering.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct RenderableCursor {
    shape: CursorShape,
    cursor_color: Rgb,
    text_color: Rgb,
    is_wide: bool,
    point: Point<usize>,
}

impl RenderableCursor {
    pub fn color(&self) -> Rgb {
        self.cursor_color
    }

    pub fn shape(&self) -> CursorShape {
        self.shape
    }

    pub fn is_wide(&self) -> bool {
        self.is_wide
    }

    pub fn point(&self) -> Point<usize> {
        self.point
    }
}
