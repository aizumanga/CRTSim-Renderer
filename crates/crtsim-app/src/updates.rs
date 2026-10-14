//! Checking for updates: whether a newer release has been published, asked of GitHub only when
//! a person checks, or at start once they have chosen that. The app tells them; it never
//! downloads a release (see ADR 4). A browser page is the latest release already, so this is
//! the desktop's alone.
use crate::*;
use anyhow::{bail, ensure, Context, Result};

/// The latest published release, as GitHub's API gives it. Drafts and pre-releases are never
/// the latest.
const LATEST: &str = "https://api.github.com/repos/aizumanga/CRTSim-Renderer/releases/latest";
/// The page listing every release, for a check that fails.
const RELEASES: &str = "https://github.com/aizumanga/CRTSim-Renderer/releases";
/// This app's version, which a release's tag is compared with.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The most of a release's description read, and shown.
const LARGEST_RESPONSE: u64 = 1 << 20;
const LONGEST_NOTES: usize = 16 * 1024;

/// A published release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Its tag, such as `v1.2.0`.
    pub tag: String,
    /// Its notes, as written on its page.
    pub notes: String,
    /// Its page, with its downloads.
    pub page: String,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// This version is the latest, or newer.
    UpToDate,
    /// A newer release is out.
    Newer(Release),
}

/// The last check for updates, and the window that shows it.
#[derive(Default)]
pub struct Updates {
    pub open: bool,
    /// Whether the app checks each time it starts, as the person chose.
    pub at_start: bool,
    /// The last check, once one has finished.
    found: Option<Result<Found, String>>,
    /// A check running on a thread of its own.
    checking: Option<mpsc::Receiver<Result<Found, String>>>,
}

impl Updates {
    /// Asks GitHub for the latest release on a thread of its own.
    pub fn start_check(&mut self, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = check(LATEST).map_err(|e| format!("{e:#}"));
            let _ = send.send(found);
            ctx.request_repaint();
        });
        self.checking = Some(receive);
    }

    /// Takes the check that has just finished, if one has.
    fn receive(&mut self) {
        if let Some(found) = self.checking.as_ref().and_then(|r| r.try_recv().ok()) {
            self.found = Some(found);
            self.checking = None;
        }
    }

    /// The newer release the last check found, if it found one.
    pub fn newer(&self) -> Option<&Release> {
        match &self.found {
            Some(Ok(Found::Newer(release))) => Some(release),
            _ => None,
        }
    }
}

/// Asks `url` for the latest release and compares it with this version.
fn check(url: &str) -> Result<Found> {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent(concat!("CRTSim-Renderer/", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("Could not reach GitHub")?;
    let body = response
        .body_mut()
        .with_config()
        .limit(LARGEST_RESPONSE)
        .read_to_string()
        .context("Could not read GitHub's answer")?;
    found(&release(&body)?, VERSION)
}

/// The release GitHub's API describes in `json`.
fn release(json: &str) -> Result<Release> {
    let value: serde_json::Value =
        serde_json::from_str(json).context("GitHub's answer is not a release")?;
    let text = |key: &str| value.get(key).and_then(serde_json::Value::as_str);
    let tag = text("tag_name").context("GitHub's answer names no release")?;
    let mut notes = text("body").unwrap_or_default().replace("\r\n", "\n");
    if notes.len() > LONGEST_NOTES {
        let mut end = LONGEST_NOTES;
        while !notes.is_char_boundary(end) {
            end -= 1;
        }
        notes.truncate(end);
        notes.push_str("\n…");
    }
    Ok(Release {
        tag: tag.to_owned(),
        notes,
        page: text("html_url").unwrap_or(RELEASES).to_owned(),
    })
}

/// Whether `latest` is newer than the version `current`.
fn found(latest: &Release, current: &str) -> Result<Found> {
    Ok(if version(&latest.tag)? > version(current)? {
        Found::Newer(latest.clone())
    } else {
        Found::UpToDate
    })
}

/// A version written as `1.2.3` or tagged `v1.2.3`, as numbers to compare.
fn version(text: &str) -> Result<[u64; 3]> {
    let numbers: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
    ensure!(
        numbers.len() == 3,
        "{text:?} is not a version such as v1.2.3"
    );
    let mut version = [0; 3];
    for (part, number) in version.iter_mut().zip(numbers) {
        let Ok(n) = number.parse() else {
            bail!("{text:?} is not a version such as v1.2.3");
        };
        *part = n;
    }
    Ok(version)
}

impl App {
    /// Checks for updates at start, if the person chose to.
    pub(crate) fn check_for_updates_at_start(&mut self, ctx: &egui::Context) {
        self.updates.at_start = self
            .app_data()
            .is_some_and(app_data::Store::check_updates_at_start);
        if self.updates.at_start {
            self.updates.start_check(ctx);
        }
    }

    pub(crate) fn show_updates(&mut self, ctx: &egui::Context) {
        self.updates.open = true;
        if self.updates.checking.is_none() {
            self.updates.start_check(ctx);
        }
    }

    pub(crate) fn updates_window(&mut self, ctx: &egui::Context) {
        self.updates.receive();
        if !self.updates.open || self.show_welcome {
            return;
        }
        let mut again = false;
        let mut at_start = self.updates.at_start;
        let mut window = self.window_state("Updates");
        let updates = &self.updates;
        let open = chrome::tool_window(ctx, "Updates", [520., 460.], &mut window, |ui| {
            ui.label(format!("This is CRTSim Renderer v{VERSION}."));
            ui.separator();
            match (&updates.checking, &updates.found) {
                (Some(_), _) | (None, None) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Asking GitHub for the latest release…");
                    });
                }
                (None, Some(Ok(Found::UpToDate))) => {
                    ui.label("✔ This is the latest release.");
                }
                (None, Some(Ok(Found::Newer(release)))) => {
                    ui.strong(format!("{} is out.", release.tag));
                    ui.hyperlink_to(
                        format!("Download {} from its release page", release.tag),
                        &release.page,
                    );
                    ui.label(
                        "Installed with Scoop, Homebrew or winget? Update with it instead. An \
                         AppImage updates itself with AppImageUpdate or Gear Lever.",
                    );
                    if !release.notes.trim().is_empty() {
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(220.)
                            .show(ui, |ui| {
                                ui.label(&release.notes);
                            });
                    }
                }
                (None, Some(Err(e))) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("Could not check: {e}"));
                    ui.hyperlink_to("See the releases on GitHub", RELEASES);
                }
            }
            ui.separator();
            ui.add_enabled_ui(updates.checking.is_none(), |ui| {
                again = ui.button("Check again").clicked();
            });
            ui.checkbox(&mut at_start, "Check when the app starts")
                .on_hover_text(
                    "Ask GitHub for the latest release each time the app starts. The app never \
                     downloads or installs a release itself.",
                );
            ui.weak(
                "Checking sends one request to GitHub, naming this app and its version. \
                 Nothing else reaches the network.",
            );
        });
        self.store_window_state("Updates", window);
        self.updates.open = open;
        if again {
            self.updates.start_check(ctx);
        }
        if at_start != self.updates.at_start {
            self.updates.at_start = at_start;
            if let Some(Err(e)) = self
                .app_data()
                .map(|store| store.set_check_updates_at_start(at_start))
            {
                self.error = Some(format!("Could not remember that choice: {e:#}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn tagged(tag: &str) -> Release {
        Release {
            tag: tag.into(),
            notes: String::new(),
            page: RELEASES.into(),
        }
    }

    #[test]
    fn only_a_higher_version_is_newer() {
        assert_eq!(found(&tagged("v1.1.0"), "1.1.0").unwrap(), Found::UpToDate);
        assert_eq!(found(&tagged("v1.0.9"), "1.1.0").unwrap(), Found::UpToDate);
        assert_eq!(
            found(&tagged("v1.1.1"), "1.1.0").unwrap(),
            Found::Newer(tagged("v1.1.1"))
        );
        // Numbers, not text: 10 is after 9.
        assert!(matches!(
            found(&tagged("v1.10.0"), "1.9.0").unwrap(),
            Found::Newer(_)
        ));
        assert!(found(&tagged("v2"), "1.1.0").is_err());
        assert!(found(&tagged("v1.2.0-rc1"), "1.1.0").is_err());
    }

    #[test]
    fn a_release_is_read_from_githubs_answer() {
        let json = r#"{"tag_name": "v1.2.0", "html_url": "https://example.com/v1.2.0",
                       "body": "Notes\r\nmore", "draft": false}"#;
        assert_eq!(
            release(json).unwrap(),
            Release {
                tag: "v1.2.0".into(),
                notes: "Notes\nmore".into(),
                page: "https://example.com/v1.2.0".into(),
            }
        );
        assert!(release(r#"{"message": "Not Found"}"#).is_err());
        assert!(release("<html>").is_err());
        let long = format!(
            r#"{{"tag_name": "v1.2.0", "body": "{}"}}"#,
            "é".repeat(LONGEST_NOTES)
        );
        let notes = release(&long).unwrap().notes;
        assert!(notes.len() <= LONGEST_NOTES + "\n…".len() && notes.ends_with('…'));
    }

    /// A server on this computer that answers one request with `status` and `body`.
    fn serve(status: &str, body: &str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}/releases/latest")
    }

    #[test]
    fn a_check_asks_and_compares() {
        let url = serve(
            "200 OK",
            r#"{"tag_name": "v999.0.0", "html_url": "https://example.com"}"#,
        );
        assert!(matches!(check(&url).unwrap(), Found::Newer(r) if r.tag == "v999.0.0"));
        let url = serve("200 OK", &format!(r#"{{"tag_name": "v{VERSION}"}}"#));
        assert_eq!(check(&url).unwrap(), Found::UpToDate);
        let url = serve("404 Not Found", r#"{"message": "Not Found"}"#);
        assert!(check(&url).is_err());
    }
}

#[cfg(test)]
mod network {
    /// Asks the real GitHub, which needs the network: `cargo test -p crtsim-app asks_github -- --ignored`.
    #[test]
    #[ignore]
    fn asks_github() {
        let found = super::check(super::LATEST).unwrap();
        println!("{found:?}");
    }
}
