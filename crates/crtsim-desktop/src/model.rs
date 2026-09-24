use anyhow::Result;
use crtsim_core::{config::Config, settings};

/// Limit only the canvas, preserving its aspect and the logical signal.
pub fn preview_config(c: &Config, input: (u32, u32), max_side: Option<u32>) -> Result<Config> {
    c.with_max_output_side(input, max_side)
}

/// One setting that differs between two looks, written for a person to read.
#[derive(Debug, PartialEq)]
pub struct Difference {
    pub setting: String,
    pub from: String,
    pub to: String,
}

/// Every setting that differs between `from` and `to`, in the settings panel's order and under
/// its names, both from `settings::SETTINGS`. That table lists every key a preset holds -- a core
/// test keeps it so -- so a setting added later cannot go unreported.
pub fn differences(from: &Config, to: &Config) -> Vec<Difference> {
    // A LUT is a large table; its name is what a person tells them apart by.
    let lut = |c: &Config| c.lut.as_ref().map_or("None".to_owned(), |l| l.name.clone());
    let json = |c: &Config| {
        serde_json::to_value(Config {
            lut: None,
            ..c.clone()
        })
        .expect("settings serialize")
    };
    let (before, after) = (json(from), json(to));
    settings::SETTINGS
        .iter()
        .filter_map(|setting| {
            let (was, now) = if setting.key == "lut" {
                (lut(from), lut(to))
            } else {
                let pointer = format!("/{}", setting.key.replace('.', "/"));
                let at = |value: &serde_json::Value| {
                    show(value.pointer(&pointer).unwrap_or(&serde_json::Value::Null))
                };
                (at(&before), at(&after))
            };
            (was != now).then(|| Difference {
                setting: setting.label.into(),
                from: was,
                to: now,
            })
        })
        .collect()
}

fn show(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Number(n) => {
            let text = format!("{:.3}", n.as_f64().unwrap_or_default());
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        serde_json::Value::Bool(true) => "On".into(),
        serde_json::Value::Bool(false) => "Off".into(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items.iter().map(show).collect::<Vec<_>>().join(", "),
        serde_json::Value::Null => "None".into(),
        serde_json::Value::Object(_) => value.to_string(),
    }
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
    fn differences_name_each_changed_setting_with_both_values() {
        assert!(differences(&Config::general(), &Config::general()).is_empty());
        let found = differences(&Config::general(), &Config::default());
        let find = |setting: &str| found.iter().find(|d| d.setting == setting);
        assert_eq!(
            find("Signal"),
            Some(&Difference {
                setting: "Signal".into(),
                from: "auto".into(),
                to: "original".into(),
            })
        );
        assert_eq!(find("Pixel aspect").unwrap().to, "1.143");
        assert!(
            find("Bloom amount").is_none(),
            "unchanged settings are not listed"
        );
        let order: Vec<_> = found.iter().map(|d| d.setting.as_str()).collect();
        assert_eq!(
            &order[..2],
            ["Signal", "Export size"],
            "listed in panel order"
        );
        // Nested and array settings, and a LUT by name rather than by table.
        let mut edited = Config::general();
        edited.source.position = [0.25, 0.];
        edited.persistence[2] = 0.5;
        edited.mask_antialias = false;
        edited.lut = Some(std::sync::Arc::new(crtsim_core::nes_luts::load(0).unwrap()));
        let found = differences(&Config::general(), &edited);
        let settings: Vec<_> = found.iter().map(|d| d.setting.as_str()).collect();
        assert!(settings.contains(&"Pan (X, Y)"));
        assert!(settings.contains(&"Persistence (R, G, B)"));
        let filter = found
            .iter()
            .find(|d| d.setting == "Filter mask when shrinking")
            .unwrap();
        assert_eq!((filter.from.as_str(), filter.to.as_str()), ("On", "Off"));
        let lut = found.iter().find(|d| d.setting == "LUT").unwrap();
        assert_eq!(
            (lut.from.as_str(), lut.to.as_str()),
            ("None", "00 - NES SMPTE-2025")
        );
    }
    #[test]
    fn preview_preserves_signal_and_canvas_aspect() {
        let mut c = Config::general();
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
        let a = Config::general();
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
