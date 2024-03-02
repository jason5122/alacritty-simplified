//! Handle input from winit.
//!
//! Certain key combinations should send some escape sequence back to the PTY.
//! In order to figure that out, state about which modifier keys are pressed
//! needs to be tracked. Additionally, we need a bit of a state machine to
//! determine what to do when a non-modifier key is pressed.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::marker::PhantomData;

use winit::event::Modifiers;
#[cfg(target_os = "macos")]
use winit::event_loop::EventLoopWindowTarget;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Direction, Point, Side};
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::Term;

use crate::clipboard::Clipboard;
use crate::config::{Action, UiConfig};
use crate::display::hint::HintMatch;
use crate::display::window::Window;
use crate::display::{Display, SizeInfo};
use crate::event::{Event, InlineSearchState, Mouse, TouchPurpose};
use crate::scheduler::Scheduler;

/// Processes input from winit.
///
/// An escape sequence may be emitted in case specific keys or key combinations
/// are activated.
pub struct Processor<T: EventListener, A: ActionContext<T>> {
    pub ctx: A,
    _phantom: PhantomData<T>,
}

pub trait ActionContext<T: EventListener> {
    fn write_to_pty<B: Into<Cow<'static, [u8]>>>(&self, _data: B) {}
    fn mark_dirty(&mut self) {}
    fn size_info(&self) -> SizeInfo;
    fn mouse_mut(&mut self) -> &mut Mouse;
    fn mouse(&self) -> &Mouse;
    fn touch_purpose(&mut self) -> &mut TouchPurpose;
    fn modifiers(&mut self) -> &mut Modifiers;
    fn scroll(&mut self, _scroll: Scroll) {}
    fn window(&mut self) -> &mut Window;
    fn display(&mut self) -> &mut Display;
    fn terminal(&self) -> &Term<T>;
    fn terminal_mut(&mut self) -> &mut Term<T>;
    fn spawn_new_instance(&mut self) {}
    #[cfg(target_os = "macos")]
    fn create_new_window(&mut self, _tabbing_id: Option<String>) {}
    #[cfg(not(target_os = "macos"))]
    fn create_new_window(&mut self) {}
    fn change_font_size(&mut self, _delta: f32) {}
    fn reset_font_size(&mut self) {}
    fn pop_message(&mut self) {}
    fn config(&self) -> &UiConfig;
    #[cfg(target_os = "macos")]
    fn event_loop(&self) -> &EventLoopWindowTarget<Event>;
    fn mouse_mode(&self) -> bool;
    fn clipboard_mut(&mut self) -> &mut Clipboard;
    fn scheduler_mut(&mut self) -> &mut Scheduler;
    fn start_search(&mut self, _direction: Direction) {}
    fn confirm_search(&mut self) {}
    fn cancel_search(&mut self) {}
    fn search_input(&mut self, _c: char) {}
    fn search_pop_word(&mut self) {}
    fn search_history_previous(&mut self) {}
    fn search_history_next(&mut self) {}
    fn search_next(&mut self, origin: Point, direction: Direction, side: Side) -> Option<Match>;
    fn advance_search_origin(&mut self, _direction: Direction) {}
    fn search_direction(&self) -> Direction;
    fn search_active(&self) -> bool;
    fn on_typing_start(&mut self) {}
    fn toggle_vi_mode(&mut self) {}
    fn inline_search_state(&mut self) -> &mut InlineSearchState;
    fn start_inline_search(&mut self, _direction: Direction, _stop_short: bool) {}
    fn inline_search_next(&mut self) {}
    fn inline_search_previous(&mut self) {}
    fn hint_input(&mut self, _character: char) {}
    fn trigger_hint(&mut self, _hint: &HintMatch) {}
    fn on_terminal_input_start(&mut self) {}
    fn paste(&mut self, _text: &str, _bracketed: bool) {}
    fn spawn_daemon<I, S>(&self, _program: &str, _args: I)
    where
        I: IntoIterator<Item = S> + Debug + Copy,
        S: AsRef<OsStr>,
    {
    }
}

trait Execute<T: EventListener> {
    fn execute<A: ActionContext<T>>(&self, ctx: &mut A);
}

impl<T: EventListener> Execute<T> for Action {
    #[inline]
    fn execute<A: ActionContext<T>>(&self, _: &mut A) {
        match self {
            _ => (),
        }
    }
}

impl<T: EventListener, A: ActionContext<T>> Processor<T, A> {
    pub fn new(ctx: A) -> Self {
        Self { ctx, _phantom: Default::default() }
    }
}
