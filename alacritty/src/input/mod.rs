use std::marker::PhantomData;

use alacritty_terminal::event::EventListener;

use crate::display::window::Window;

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

impl<T: EventListener, A: ActionContext<T>> Processor<T, A> {
    pub fn new(ctx: A) -> Self {
        Self { ctx, _phantom: Default::default() }
    }
}
