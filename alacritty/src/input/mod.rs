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
    fn window(&mut self) -> &mut Window;
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
