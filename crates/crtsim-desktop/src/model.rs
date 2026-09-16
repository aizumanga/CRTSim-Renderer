use anyhow::Result;
use crtsim_core::config::{Config, Filter, Fit};

pub fn general() -> Config {
    Config {
        signal: "auto".into(),
        output: "1080p".into(),
        fit: Fit::Contain,
        filter: Filter::Lanczos,
        pixel_aspect: 1.,
        saturation: 1.,
        ..Config::default()
    }
}

/// Limit only the canvas, preserving its aspect and the logical signal.
pub fn preview_config(c: &Config, input: (u32, u32), max_side: Option<u32>) -> Result<Config> {
    c.validate()?;
    c.signal_size(input)?;
    let (w, h) = c.output_size(input)?;
    let scale = max_side.map_or(1., |m| (m as f64 / w.max(h) as f64).min(1.));
    let mut preview = c.clone();
    preview.output = format!(
        "{}x{}",
        (w as f64 * scale).round().max(1.) as u32,
        (h as f64 * scale).round().max(1.) as u32
    );
    Ok(preview)
}

pub struct History {
    past: Vec<Config>,
    current: Config,
    future: Vec<Config>,
}
impl History {
    pub fn new(c: Config) -> Self {
        Self {
            past: Vec::new(),
            current: c,
            future: Vec::new(),
        }
    }
    pub fn commit(&mut self, c: &Config) {
        if *c == self.current {
            return;
        }
        self.past
            .push(std::mem::replace(&mut self.current, c.clone()));
        if self.past.len() > 100 {
            self.past.remove(0);
        }
        self.future.clear();
    }
    pub fn undo(&mut self, c: &Config) -> Option<Config> {
        self.commit(c);
        let previous = self.past.pop()?;
        self.future
            .push(std::mem::replace(&mut self.current, previous.clone()));
        Some(previous)
    }
    pub fn redo(&mut self, c: &Config) -> Option<Config> {
        self.commit(c);
        let next = self.future.pop()?;
        self.past
            .push(std::mem::replace(&mut self.current, next.clone()));
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_preserves_signal_and_canvas_aspect() {
        let mut c = general();
        c.output = "4k".into();
        let p = preview_config(&c, (1216, 832), Some(1280)).unwrap();
        assert_eq!(p.output_size((1216, 832)).unwrap(), (1280, 720));
        assert_eq!(
            p.signal_size((1216, 832)).unwrap(),
            c.signal_size((1216, 832)).unwrap()
        );
        assert_eq!(c.output, "4k");
        c.output = "600x1200".into();
        assert_eq!(
            preview_config(&c, (1, 1), Some(800)).unwrap().output,
            "400x800"
        );
        c.output = "0x100".into();
        assert!(preview_config(&c, (1, 1), Some(800)).is_err());
    }
    #[test]
    fn undo_redo_and_new_edit_branch() {
        let a = general();
        let mut b = a.clone();
        b.bloom = 1.;
        let mut h = History::new(a.clone());
        assert_eq!(h.undo(&b), Some(a.clone()));
        assert_eq!(h.redo(&a), Some(b.clone()));
        assert_eq!(h.undo(&b), Some(a.clone()));
        let mut c = a;
        c.bloom = 0.;
        h.commit(&c);
        assert!(h.redo(&c).is_none());
    }
}
