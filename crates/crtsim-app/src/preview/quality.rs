//! The size previews render at: one the person chose, or Auto, which starts at Balanced and
//! steps down to Fast while previews of an edit being dragged come back slowly, as on
//! integrated graphics, and back up once Fast ones come back quickly. Only those previews
//! count: a settled one is a still with its warm-up, slower whatever the size.

/// The longest side of a Fast preview, in pixels.
pub const FAST: u32 = 800;
/// The longest side of a Balanced preview, in pixels.
pub const BALANCED: u32 = 1280;
/// A dragged preview slower than this, in seconds, is slow at Balanced.
const SLOW: f32 = 0.5;
/// A dragged preview quicker than this at Fast would be well under `SLOW` at Balanced, which
/// has about 2.6 times the pixels, so stepping up does not soon step down again.
const QUICK: f32 = 0.15;
/// How many slow previews in a row step down, and quick ones step up: one slow preview may be
/// a stall rather than the graphics.
const SLOW_IN_A_ROW: u8 = 2;
const QUICK_IN_A_ROW: u8 = 5;

/// What the person chose in the Preview quality list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Auto,
    /// A longest side of its own, or none for the export's size.
    Fixed(Option<u32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quality {
    choice: Choice,
    /// The side Auto renders at now: Balanced or Fast.
    auto: u32,
    /// Slow or quick previews in a row so far.
    run: u8,
}

impl Default for Quality {
    fn default() -> Self {
        Self::new(Choice::Auto)
    }
}

impl Quality {
    pub fn new(choice: Choice) -> Self {
        Self {
            choice,
            auto: BALANCED,
            run: 0,
        }
    }

    /// The longest side previews render at now; none for the export's size.
    pub fn side(&self) -> Option<u32> {
        match self.choice {
            Choice::Auto => Some(self.auto),
            Choice::Fixed(side) => side,
        }
    }

    pub fn choice(&self) -> Choice {
        self.choice
    }

    /// The person chose `choice`. Auto starts again at Balanced.
    pub fn choose(&mut self, choice: Choice) {
        *self = Self::new(choice);
    }

    /// What Auto renders at now, for the list to say.
    pub fn auto_name(&self) -> &'static str {
        if self.auto == FAST {
            "Fast"
        } else {
            "Balanced"
        }
    }

    /// A preview of an edit being dragged took `seconds`. Whether Auto stepped to another size.
    pub fn dragged(&mut self, seconds: f32) -> bool {
        if self.choice != Choice::Auto {
            return false;
        }
        let (counts, next, needed) = match self.auto {
            BALANCED => (seconds > SLOW, FAST, SLOW_IN_A_ROW),
            _ => (seconds < QUICK, BALANCED, QUICK_IN_A_ROW),
        };
        self.run = if counts { self.run + 1 } else { 0 };
        if self.run < needed {
            return false;
        }
        self.auto = next;
        self.run = 0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_steps_down_after_slow_drags_and_up_after_quick_ones() {
        let mut quality = Quality::default();
        assert_eq!(quality.side(), Some(BALANCED));
        // One slow preview is not enough, and a quick one in between starts the count again.
        assert!(!quality.dragged(0.8));
        assert!(!quality.dragged(0.2));
        assert!(!quality.dragged(0.8));
        assert!(quality.dragged(0.9));
        assert_eq!((quality.side(), quality.auto_name()), (Some(FAST), "Fast"));
        // At Fast, previews under the slow mark but not quick keep it there.
        for _ in 0..10 {
            assert!(!quality.dragged(0.3));
        }
        for _ in 0..4 {
            assert!(!quality.dragged(0.1));
        }
        assert!(quality.dragged(0.1));
        assert_eq!(quality.side(), Some(BALANCED));
    }

    #[test]
    fn a_size_chosen_stays() {
        let mut quality = Quality::new(Choice::Fixed(Some(BALANCED)));
        for _ in 0..10 {
            assert!(!quality.dragged(2.));
        }
        assert_eq!(quality.side(), Some(BALANCED));
        quality.choose(Choice::Fixed(None));
        assert_eq!(quality.side(), None);
        quality.choose(Choice::Auto);
        assert_eq!(quality.side(), Some(BALANCED));
    }
}
