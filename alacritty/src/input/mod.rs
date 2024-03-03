use crate::display::window::Window;

/// Processes input from winit.
///
/// An escape sequence may be emitted in case specific keys or key combinations
/// are activated.
pub struct Processor<A: ActionContext> {
    pub ctx: A,
}

pub trait ActionContext {
    fn window(&mut self) -> &mut Window;
}

impl<A: ActionContext> Processor<A> {
    pub fn new(ctx: A) -> Self {
        Self { ctx }
    }
}
