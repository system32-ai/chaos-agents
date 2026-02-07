const FRAMES: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];

pub struct Spinner {
    tick: usize,
}

impl Spinner {
    pub fn new() -> Self {
        Self { tick: 0 }
    }

    pub fn tick(&mut self) {
        self.tick = (self.tick + 1) % FRAMES.len();
    }

    pub fn frame(&self) -> &str {
        FRAMES[self.tick]
    }
}
