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
        mask_antialias: true,
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

/// One setting that differs between two looks, written for a person to read.
#[derive(Debug, PartialEq)]
pub struct Difference {
    pub setting: String,
    pub from: String,
    pub to: String,
}

/// Every setting that differs between `from` and `to`, in the settings panel's order. Worked
/// out from the settings' serialized form rather than a hand-kept list, so a setting added
/// later is still reported, under a name derived from its key until it is given a label.
pub fn differences(from: &Config, to: &Config) -> Vec<Difference> {
    // A LUT is a large table; its name is what a person tells them apart by.
    let lut = |c: &Config| c.lut.as_ref().map_or("None".to_owned(), |l| l.name.clone());
    let flat = |c: &Config| {
        let mut entries = vec![];
        let value = serde_json::to_value(Config {
            lut: None,
            ..c.clone()
        })
        .expect("settings serialize");
        flatten("", &value, &mut entries);
        entries
    };
    let mut out = vec![];
    for ((key, a), (_, b)) in flat(from).into_iter().zip(flat(to)) {
        if key == "version" {
            continue;
        }
        let (a, b) = if key == "lut" {
            (lut(from), lut(to))
        } else {
            (show(&a), show(&b))
        };
        if a != b {
            let (order, setting) = label(&key);
            out.push((
                order,
                Difference {
                    setting,
                    from: a,
                    to: b,
                },
            ));
        }
    }
    out.sort_by_key(|(order, _)| *order);
    out.into_iter().map(|(_, difference)| difference).collect()
}

fn flatten(prefix: &str, value: &serde_json::Value, out: &mut Vec<(String, serde_json::Value)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&key, value, out);
            }
        }
        _ => out.push((prefix.to_owned(), value.clone())),
    }
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

/// The names the settings panel uses, in the order it shows them, which is also the order
/// differences are listed in.
const LABELS: &[(&str, &str)] = &[
    ("signal", "Signal"),
    ("output", "Export size"),
    ("fit", "Fit on 4:3 tube"),
    ("filter", "Resize filter"),
    ("pixel_aspect", "Pixel aspect"),
    ("screen_only", "Screen only"),
    ("source.crop", "Crop (left, top, right, bottom)"),
    ("source.rotation", "Rotation °"),
    ("source.zoom", "Source zoom"),
    ("source.position", "Pan (X, Y)"),
    ("source.background", "Transparency background"),
    ("source.checkerboard", "Checker background"),
    ("lut", "LUT"),
    ("color_mode", "Color processing"),
    ("hue", "Hue"),
    ("chroma", "Chroma"),
    ("mask_antialias", "Filter mask when shrinking"),
    ("saturation", "Saturation"),
    ("sharpness", "Sharpness / ringing"),
    ("bleed", "Color bleed"),
    ("artifacts", "Composite artifacts"),
    ("barrel", "Barrel distortion"),
    ("overscan", "Overscan"),
    ("mask_opacity", "Mask opacity"),
    ("mask_brightness", "Mask brightness"),
    ("mask_repeats", "Mask columns, rows"),
    ("dimming", "Edge dimming"),
    ("fov", "Camera field of view"),
    ("bloom", "Bloom amount"),
    ("bloom_power", "Bloom power"),
    ("bloom_spread", "Bloom spread"),
    ("reflection", "Edge reflection"),
    ("frame_color", "Frame color"),
    ("diffuse", "Diffuse light"),
    ("specular", "Specular light"),
    ("specular_power", "Specular power"),
    ("rim", "Rim light"),
    ("light_position", "Light position (X, Y, Z)"),
    ("persistence", "Persistence (R, G, B)"),
    ("warmup", "Warm-up ticks"),
    ("phase", "Phase"),
    ("interlace", "Interlaced fields"),
];

/// A setting's label and its place in `LABELS`. A setting missing from the table goes last,
/// under a name made from its key.
fn label(key: &str) -> (usize, String) {
    if let Some(index) = LABELS.iter().position(|(k, _)| *k == key) {
        return (index, LABELS[index].1.to_owned());
    }
    let words = key.rsplit('.').next().unwrap_or(key).replace('_', " ");
    let mut chars = words.chars();
    let name = chars.next().map_or(String::new(), |first| {
        first.to_uppercase().chain(chars).collect()
    });
    (LABELS.len(), name)
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
        assert!(differences(&general(), &general()).is_empty());
        let found = differences(&general(), &Config::default());
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
        assert_eq!(find("Filter mask when shrinking").unwrap().from, "On");
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
        // Every setting has a label: one missing from the table would be reported by key.
        let keys = {
            let mut keys = vec![];
            flatten(
                "",
                &serde_json::to_value(Config::default()).unwrap(),
                &mut keys,
            );
            keys
        };
        for (key, _) in keys.iter().filter(|(key, _)| key != "version") {
            assert!(label(key).0 < LABELS.len(), "no label for {key}");
        }
        // Nested and array settings, and a LUT by name rather than by table.
        let mut edited = general();
        edited.source.position = [0.25, 0.];
        edited.persistence[2] = 0.5;
        edited.lut = Some(std::sync::Arc::new(crtsim_core::nes_luts::load(0).unwrap()));
        let found = differences(&general(), &edited);
        let settings: Vec<_> = found.iter().map(|d| d.setting.as_str()).collect();
        assert!(settings.contains(&"Pan (X, Y)"));
        assert!(settings.contains(&"Persistence (R, G, B)"));
        let lut = found.iter().find(|d| d.setting == "LUT").unwrap();
        assert_eq!(
            (lut.from.as_str(), lut.to.as_str()),
            ("None", "00 - NES SMPTE-2025")
        );
    }
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
