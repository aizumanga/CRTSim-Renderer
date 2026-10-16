//! A look as a link: the settings in use, compressed into the fragment of a link to the web app
//! (`…/crtsim/#look=…`), so a look is shared by sending a link. Opening the link applies the look
//! in the web app, and **Presets → Open look from link…** applies one in either app. The link
//! holds the whole look; no server stores it, and a fragment never reaches the site's server.
//!
//! A look is the preset JSON, deflated and written in URL-safe Base64. An included NES LUT goes
//! by its name rather than its 262,144 colours; a LUT imported from a file is too large for a
//! link, which is refused, so that look is shared as a JSON preset instead.
use crate::chrome::Fitted;
use anyhow::{bail, ensure, Context, Result};
use crtsim_core::{config::Config, nes_luts};
use std::{io::Read, sync::Arc};

/// The web app, which a link opens.
pub const SITE: &str = "https://aizumanga.neocities.org/crtsim/";
/// What starts a look in a link's fragment.
const KEY: &str = "look=";
/// The longest look a link holds, in characters: well within what chat apps and browsers keep.
const LONGEST: usize = 8000;
/// The most a look decompresses to, so a crafted link cannot exhaust memory.
const LARGEST_PRESET: u64 = 1 << 20;

/// What a link holds, besides the look's settings.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Linked {
    /// The look's preset, without its LUT.
    preset: serde_json::Value,
    /// The included NES LUT the look maps colours through, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    included_lut: Option<String>,
}

/// A link to `page` that opens `config`.
pub fn link(page: &str, config: &Config) -> Result<String> {
    let mut preset = config.clone();
    let mut included_lut = None;
    if let Some(lut) = preset.lut.take() {
        included_lut = Some(included(&lut).context(
            "A LUT imported from a file is too large for a link. Save the look as a preset to \
             share it instead.",
        )?);
    }
    let linked = Linked {
        preset: serde_json::to_value(&preset)?,
        included_lut,
    };
    let json = serde_json::to_vec(&linked)?;
    let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    std::io::Write::write_all(&mut deflate, &json)?;
    let code = base64_url(&deflate.finish()?);
    ensure!(
        code.len() <= LONGEST,
        "This look is too long for a link. Save it as a preset to share it instead."
    );
    let page = page.split('#').next().unwrap_or(page);
    Ok(format!("{page}#{KEY}{code}"))
}

/// The look a link, or the part of one after `look=`, holds, checked for a source of size `input`.
pub fn look(text: &str, input: (u32, u32)) -> Result<Config> {
    let text = text.trim();
    let code = match text.find(KEY) {
        Some(start) => &text[start + KEY.len()..],
        None if !text.contains(['/', '#', ' ']) => text,
        None => bail!("This link holds no look. A look's link has #look= in it."),
    };
    // Anything after the look, such as another part of the fragment, is not part of it.
    let code = code.split(['&', '#']).next().unwrap_or_default();
    ensure!(!code.is_empty(), "This link's look is empty");
    ensure!(code.len() <= LONGEST, "This link's look is too long");
    let deflated = from_base64_url(code).context("This link's look is damaged or incomplete")?;
    let mut json = vec![];
    flate2::read::DeflateDecoder::new(deflated.as_slice())
        .take(LARGEST_PRESET + 1)
        .read_to_end(&mut json)
        .context("This link's look is damaged or incomplete")?;
    ensure!(
        json.len() as u64 <= LARGEST_PRESET,
        "This link's look is too large"
    );
    let linked: Linked =
        serde_json::from_slice(&json).context("This link's look is not one this app made")?;
    let mut config = Config::from_json_slice(&serde_json::to_vec(&linked.preset)?)
        .context("This link's look cannot be opened")?;
    if let Some(name) = linked.included_lut {
        let index = nes_luts::ENTRIES
            .iter()
            .position(|entry| entry.name == name)
            .with_context(|| format!("This look uses a LUT this app does not include: {name}"))?;
        config.set_lut(Some(Arc::new(nes_luts::load(index)?)));
    }
    config.validate_for(input)?;
    Ok(config)
}

/// The name of the included NES LUT `lut` is, if it is one.
fn included(lut: &crtsim_core::workflow::Lut) -> Option<String> {
    let index = nes_luts::ENTRIES
        .iter()
        .position(|entry| entry.name == lut.name)?;
    // A LUT imported from a file could be given an included one's name; only the same
    // colours go by it.
    (nes_luts::load(index).ok()? == *lut).then(|| lut.name.clone())
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// `bytes` in URL-safe Base64, without padding.
fn base64_url(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            text.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    text
}

/// The bytes URL-safe Base64 `text` holds, with or without padding.
fn from_base64_url(text: &str) -> Result<Vec<u8>> {
    let text = text.trim_end_matches('=');
    ensure!(text.len() % 4 != 1, "not Base64");
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.as_bytes().chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let value = ALPHABET
                .iter()
                .position(|&a| a == c)
                .context("not Base64")?;
            n |= (value as u32) << (18 - 6 * i);
        }
        for i in 0..chunk.len() - 1 {
            bytes.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Ok(bytes)
}

impl crate::App {
    /// The page a link to a look opens: this one in a browser, the web app from the desktop.
    fn page() -> String {
        #[cfg(target_arch = "wasm32")]
        if let Some(href) = web_sys::window().and_then(|w| w.location().href().ok()) {
            return href;
        }
        SITE.into()
    }

    /// Copies a link to the look in use, saying so in the status bar.
    pub(crate) fn copy_look_link(&mut self, ctx: &eframe::egui::Context) {
        match link(&Self::page(), &self.config) {
            Ok(link) => {
                ctx.copy_text(link);
                self.status = "Copied a link to this look. Opening it shows the look in the \
                               web app."
                    .into();
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Cannot share this look as a link: {e:#}")),
        }
    }

    /// Applies the look `text` links to, as an undoable change.
    fn open_look(&mut self, text: &str) -> bool {
        match look(text, self.source.input().dimensions()) {
            Ok(config) => {
                self.bring_in(Ok(crate::incoming::Brought {
                    config,
                    options: None,
                    preset_name: None,
                    status: "Opened a look from a link".into(),
                }));
                true
            }
            Err(e) => {
                self.error = Some(format!("Cannot open this link: {e:#}"));
                false
            }
        }
    }

    /// The window a link is pasted into, while it is open.
    pub(crate) fn look_link_window(&mut self, ctx: &eframe::egui::Context) {
        use eframe::egui;
        let Some(mut text) = self.look_link.take() else {
            return;
        };
        let mut open = true;
        let mut opened = false;
        egui::Window::new("Open look from link")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .fitted(ctx, 460.)
            .show(ctx, |ui| {
                ui.label("Paste a link to a look, made with Presets → Copy link to this look.");
                let field = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text("https://aizumanga.neocities.org/crtsim/#look=…")
                        .desired_width(f32::INFINITY),
                );
                let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    let clicked = ui
                        .add_enabled(!text.trim().is_empty(), egui::Button::new("Open look"))
                        .clicked();
                    if (clicked || entered) && !text.trim().is_empty() {
                        opened = self.open_look(&text);
                    }
                    ui.weak("It replaces the settings in use; Undo brings them back.");
                });
            });
        if open && !opened {
            self.look_link = Some(text);
        }
    }

    /// Applies a look the page's address holds after `#look=`, then takes it off the address,
    /// so reloading the page keeps the session rather than applying the link again.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn look_from_address_bar(&mut self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let location = window.location();
        let Ok(hash) = location.hash() else {
            return;
        };
        if !hash.contains(KEY) {
            return;
        }
        self.open_look(&hash);
        if let (Ok(history), Ok(path), Ok(search)) =
            (window.history(), location.pathname(), location.search())
        {
            let _ = history.replace_state_with_url(
                &wasm_bindgen::JsValue::NULL,
                "",
                Some(&format!("{path}{search}")),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gallery;

    const INPUT: (u32, u32) = (640, 480);

    #[test]
    fn base64_round_trips_every_length() {
        for length in 0..40 {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 37 + 11) as u8).collect();
            let text = base64_url(&bytes);
            assert!(!text.contains(['+', '/', '=']));
            assert_eq!(from_base64_url(&text).unwrap(), bytes);
        }
        assert!(from_base64_url("a").is_err());
        assert!(from_base64_url("ab!c").is_err());
    }

    #[test]
    fn every_included_preset_round_trips_in_a_short_link() {
        for preset in gallery::builtins() {
            let link = link(SITE, &preset.config).unwrap();
            assert!(link.starts_with("https://aizumanga.neocities.org/crtsim/#look="));
            assert!(
                link.len() < 2000,
                "{}: {} characters",
                preset.name,
                link.len()
            );
            assert_eq!(
                look(&link, INPUT).unwrap(),
                preset.config,
                "{}",
                preset.name
            );
        }
    }

    #[test]
    fn an_included_lut_goes_by_name_and_an_imported_one_is_refused() {
        let mut config = gallery::default_preset().config;
        let lut = nes_luts::load(3).unwrap();
        config.set_lut(Some(Arc::new(lut.clone())));
        let shared = link(SITE, &config).unwrap();
        assert!(shared.len() < 2000);
        assert_eq!(look(&shared, INPUT).unwrap(), config);

        let mut imported = lut;
        imported.values[0] = [0.5; 3];
        config.set_lut(Some(Arc::new(imported)));
        let refused = link(SITE, &config).unwrap_err();
        assert!(format!("{refused:#}").contains("too large for a link"));
    }

    #[test]
    fn a_look_is_found_in_a_link_or_on_its_own() {
        let config = gallery::default_preset().config;
        let shared = link("http://localhost:8000/?x=1#old", &config).unwrap();
        assert!(shared.starts_with("http://localhost:8000/?x=1#look="));
        let code = shared.split(KEY).nth(1).unwrap();
        assert_eq!(look(code, INPUT).unwrap(), config);
        assert_eq!(
            look(&format!("  {shared}&other=1 "), INPUT).unwrap(),
            config
        );
    }

    #[test]
    fn broken_links_say_why() {
        let config = gallery::default_preset().config;
        let shared = link(SITE, &config).unwrap();
        let message = |text: &str| format!("{:#}", look(text, INPUT).unwrap_err());
        assert!(message(SITE).contains("holds no look"));
        assert!(message(&shared[..shared.len() - 20]).contains("damaged or incomplete"));
        assert!(message(&format!("{SITE}#look=")).contains("empty"));
        // A look whose compressed bytes expand past the limit is refused, not read whole.
        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut deflate, &vec![b' '; 4 << 20]).unwrap();
        let bomb = base64_url(&deflate.finish().unwrap());
        assert!(message(&bomb).contains("too large"));
    }
}
