//! DBForge's self-update, in place of Tauri's updater and reading what it
//! read, so the copies installed by the Tauri app and this one update
//! alike: `latest.json` from the latest GitHub release ([`config`],
//! [`manifest`]), the download for this system and package ([`target`]),
//! its minisign signature checked against the same public key, for the
//! version announced ([`signature`]), then the package put in place as
//! Tauri did ([`install`]) and the new version started.
//!
//! [`Updates`] holds where that stands for the whole app ([`Phase`]): it
//! looks by itself every hour while the user leaves that on, announces a
//! version when it first finds it and then once a day ([`schedule`]), and
//! answers every check the user asks for (the app menu, Settings › About).
//! The window shows it in [`UpdateDialog`].

pub mod client;
pub mod config;
mod dialog;
pub mod install;
pub mod manifest;
pub mod notes;
pub mod phase;
pub mod schedule;
pub mod signature;
pub mod target;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext, BorrowAppContext, Context, Entity, Global, Task, actions};

use self::config::{REQUIRE_SIGNED_VERSION, Settings};
pub use self::dialog::{UpdateDialog, key_bindings as dialog_key_bindings};
use self::install::Relaunch;
use self::manifest::{Release, parse_version};
pub use self::phase::{Phase, Stage, Update};
use self::target::Target;
use crate::backend::Backend;
use crate::preferences::{Preferences, keys};

actions!(updates, [CheckForUpdates]);

/// What the install, running on the core's runtime, tells the window.
enum Progress {
    Downloaded(u64, Option<u64>),
    Installing,
}

pub struct Updates {
    phase: Phase,
    settings: Settings,
    /// The check or install running; its result lands in `phase`.
    work: Option<Task<()>>,
    /// The install's progress, while it downloads.
    progress: Option<Task<()>>,
    _schedule: Option<Task<()>>,
    /// Development only: install what the next check finds, without a click.
    #[cfg(debug_assertions)]
    install_when_found: bool,
}

struct GlobalUpdates(Entity<Updates>);

impl Global for GlobalUpdates {}

/// Milliseconds since the epoch, as the former web UI kept the last check.
fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

impl Updates {
    /// Starts the updater for the app, with its automatic check.
    pub fn start(cx: &mut App) {
        Self::start_with(Settings::current(), true, cx);
    }

    /// The updater with `settings`; `scheduled` starts the automatic check.
    pub fn start_with(settings: Settings, scheduled: bool, cx: &mut App) {
        let updates = cx.new(|cx: &mut Context<Self>| {
            let schedule = scheduled.then(|| {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(schedule::FIRST_LOOK).await;
                    while this.update(cx, |this, cx| this.look(cx)).is_ok() {
                        cx.background_executor().timer(schedule::LOOK_EVERY).await;
                    }
                })
            });
            Self {
                phase: Phase::Idle,
                settings,
                work: None,
                progress: None,
                _schedule: schedule,
                #[cfg(debug_assertions)]
                install_when_found: false,
            }
        });
        cx.set_global(GlobalUpdates(updates));
        cx.on_action(|_: &CheckForUpdates, cx| Self::global(cx).update(cx, |updates, cx| updates.check(true, cx)));
    }

    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<GlobalUpdates>().0.clone()
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Whether the automatic check is on (Settings › About).
    pub fn automatic(cx: &App) -> bool {
        schedule::automatic(Preferences::global(cx).get(keys::UPDATES_AUTOMATIC))
    }

    pub fn set_automatic(on: bool, cx: &mut App) {
        cx.update_global::<Preferences, _>(|preferences, _| {
            preferences.set(keys::UPDATES_AUTOMATIC, Some(on.to_string()))
        });
    }

    /// The hourly look: checks quietly while the automatic check is on.
    fn look(&mut self, cx: &mut Context<Self>) {
        if Self::automatic(cx) {
            self.check(false, cx);
        }
    }

    /// Looks for a newer version; `manual` when the user asked, which
    /// always gets an answer. Nothing starts while a check or an install runs.
    pub fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.phase.busy() {
            return;
        }
        if manual {
            self.phase = Phase::Checking;
            cx.notify();
        }
        let endpoint = self.settings.endpoint.clone();
        let checked = Backend::global(cx).spawn(cx, move |_| async move { newest(&endpoint).await });
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = checked.await;
            let _ = this.update(cx, |this, cx| {
                this.receive(manual, result, now_ms(), cx);
                #[cfg(debug_assertions)]
                if std::mem::take(&mut this.install_when_found) && matches!(this.phase, Phase::Available(_)) {
                    this.install(cx);
                }
            });
        }));
    }

    /// Takes a check's answer: what it found shows, except that an automatic
    /// check announces a version only when it is new, or a day after it was
    /// last announced (see [`schedule::announces`]).
    fn receive(&mut self, manual: bool, result: Result<Option<Update>, String>, now: i64, cx: &mut Context<Self>) {
        let mut next = Phase::checked(manual, result);
        if let Phase::Available(update) = &next {
            let last = schedule::Announced::parse(Preferences::global(cx).get(keys::UPDATES_ANNOUNCED));
            if manual || schedule::announces(&update.version, last.as_ref(), now) {
                let announced = schedule::Announced { version: update.version.clone(), at: now };
                cx.update_global::<Preferences, _>(|preferences, _| {
                    preferences.set(keys::UPDATES_ANNOUNCED, Some(announced.saved()))
                });
            } else {
                next = Phase::Idle;
            }
        }
        // A quiet automatic check leaves an answer the user is reading.
        if manual || next != Phase::Idle || !self.phase.shown() || self.phase == Phase::Checking {
            self.phase = next;
        }
        cx.notify();
    }

    /// Downloads, verifies and installs the update offered (or the one
    /// that failed), then starts the new version.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(update) = self.phase.installable().cloned() else { return };
        self.phase =
            Phase::Installing { update: update.clone(), stage: Stage::Downloading { downloaded: 0, total: None } };
        cx.notify();

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        self.progress = Some(cx.spawn(async move |this, cx| {
            while let Some(progress) = receiver.recv().await {
                let applied = this.update(cx, |this, cx| {
                    if let Phase::Installing { stage, .. } = &mut this.phase {
                        *stage = match progress {
                            Progress::Downloaded(downloaded, total) => Stage::Downloading { downloaded, total },
                            Progress::Installing => Stage::Installing,
                        };
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        }));

        let settings = self.settings.clone();
        let installing = update.clone();
        let installed = Backend::global(cx)
            .spawn(cx, move |_| async move { download_and_install(installing, settings, sender).await });
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = installed.await;
            let _ = this.update(cx, |this, cx| this.installed(result, update, cx));
        }));
    }

    fn installed(&mut self, result: Result<Relaunch, String>, update: Arc<Update>, cx: &mut Context<Self>) {
        self.progress = None;
        let failed = |error: String| Phase::Failed { error, update: Some(update.clone()) };
        match result {
            Ok(Relaunch::Quit) => cx.quit(),
            Ok(Relaunch::Start(executable)) => match std::process::Command::new(&executable).spawn() {
                Ok(_) => cx.quit(),
                Err(error) => {
                    self.phase = failed(format!(
                        "DBForge {} is installed, but couldn't start ({error}): quit DBForge and open it again.",
                        update.version
                    ));
                }
            },
            Err(error) => self.phase = failed(error),
        }
        cx.notify();
    }

    /// Closes the dialog: Later, OK, or Escape. Not while installing.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Installing { .. }) {
            return;
        }
        // A check still running is left to finish quietly.
        self.phase = Phase::Idle;
        cx.notify();
    }

    /// Development only: checks, and installs what it finds as if Install
    /// and Restart were pressed (point `DBFORGE_UPDATE_INSTALL_PATH` at a stand-in).
    #[cfg(debug_assertions)]
    pub fn check_and_install_for_development(&mut self, cx: &mut Context<Self>) {
        self.install_when_found = true;
        self.check(true, cx);
    }

    /// Development only: shows `phase` as if a check had found it.
    #[cfg(debug_assertions)]
    pub fn show_for_development(&mut self, phase: Phase, cx: &mut Context<Self>) {
        self.phase = phase;
        cx.notify();
    }
}

/// The update `endpoint`'s manifest offers this copy, if newer.
async fn newest(endpoint: &str) -> Result<Option<Update>, String> {
    let target = Target::current().ok_or("DBForge publishes no updates for this system")?;
    let current = parse_version(dbforge_core::VERSION).map_err(|error| error.to_string())?;
    let manifest = client::fetch_manifest(endpoint).await?;
    let release = Release::parse(&manifest).map_err(|error| error.to_string())?;
    phase::update_from(release, &current, &target).map_err(|error| error.to_string())
}

/// Downloads `update`, checks its signature and installs it; what starts the new version.
async fn download_and_install(
    update: Arc<Update>,
    settings: Settings,
    progress: tokio::sync::mpsc::UnboundedSender<Progress>,
) -> Result<Relaunch, String> {
    let bytes = client::download(&update.url, |downloaded, total| {
        let _ = progress.send(Progress::Downloaded(downloaded, total));
    })
    .await?;
    let _ = progress.send(Progress::Installing);
    let installing = tokio::task::spawn_blocking(move || {
        signature::verify(&bytes, &update.signature, &settings.public_key, &update.version, REQUIRE_SIGNED_VERSION)
            .map_err(|error| error.to_string())?;
        let place = match settings.install_place {
            Some(place) => place,
            None => current_place(update.installer)?,
        };
        install::install(update.installer, &bytes, &place, &update.version)
    });
    installing.await.map_err(|error| format!("The install stopped: {error}"))?
}

/// Where this copy is installed (see [`install::install_place`]).
fn current_place(installer: target::Installer) -> Result<std::path::PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("DBForge's executable can't be found: {error}"))?;
    let appimage = std::env::var_os("APPIMAGE").filter(|path| !path.is_empty()).map(std::path::PathBuf::from);
    install::install_place(installer, &executable, appimage.as_deref())
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::test_support::setup;

    fn update() -> Arc<Update> {
        Arc::new(Update {
            version: "9.0.0".into(),
            current_version: dbforge_core::VERSION.into(),
            notes: Some("## Features\n\n- **app:** native".into()),
            url: "http://127.0.0.1:9/never".into(),
            signature: "sig".into(),
            installer: target::Installer::App,
        })
    }

    /// The whole install against a local server: download, signature check
    /// against a throwaway key, then the bundle swapped, as for a real update.
    #[cfg(target_os = "macos")]
    mod end_to_end {
        use std::io::Write;
        use std::path::{Path, PathBuf};

        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use super::super::*;

        /// Serves `body` once per connection at any path, as GitHub serves a release asset.
        async fn serve(body: Vec<u8>) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                }
            });
            format!("http://{address}/DBForge-0.4.0.app.tar.gz")
        }

        fn archive(executable: &[u8]) -> Vec<u8> {
            let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
            let mut header = tar::Header::new_gnu();
            header.set_size(executable.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, "DBForge.app/Contents/MacOS/DBForge", executable).unwrap();
            let mut encoder = builder.into_inner().unwrap();
            encoder.flush().unwrap();
            encoder.finish().unwrap()
        }

        /// A throwaway key's public half and `data`'s signature for `version`, as `dbforge-release` signs.
        fn signed(data: &[u8], version: &str) -> (String, String) {
            let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            let comment = format!("timestamp:1\tfile:DBForge-{version}.app.tar.gz\tversion:{version}");
            let signature = minisign::sign(Some(&pair.pk), &pair.sk, data, Some(&comment), None).unwrap();
            (STANDARD.encode(pair.pk.to_box().unwrap().into_string()), STANDARD.encode(signature.into_string()))
        }

        fn fake_bundle(dir: &Path) -> PathBuf {
            let bundle = dir.join("DBForge.app");
            std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
            std::fs::write(bundle.join("Contents/MacOS/DBForge"), b"old").unwrap();
            bundle
        }

        fn update(url: String, signature: String, version: &str) -> Arc<Update> {
            Arc::new(Update {
                version: version.into(),
                current_version: "0.3.5".into(),
                notes: None,
                url,
                signature,
                installer: target::Installer::App,
            })
        }

        #[tokio::test]
        async fn downloads_verifies_and_swaps_the_bundle() {
            let dir = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(dir.path());
            let body = archive(b"new");
            let (public_key, signature) = signed(&body, "0.4.0");
            let url = serve(body).await;
            let settings = Settings { endpoint: String::new(), public_key, install_place: Some(bundle.clone()) };
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let relaunch = download_and_install(update(url, signature, "0.4.0"), settings, sender).await.unwrap();
            let name = std::env::current_exe().unwrap().file_name().unwrap().to_owned();
            assert_eq!(relaunch, Relaunch::Start(bundle.join("Contents/MacOS").join(name)));
            assert_eq!(std::fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"new");
            let mut told = Vec::new();
            while let Ok(progress) = receiver.try_recv() {
                told.push(matches!(progress, Progress::Installing));
            }
            assert_eq!(told.last(), Some(&true), "the download's progress, then the install");
        }

        #[tokio::test]
        async fn installs_nothing_signed_for_another_version() {
            let dir = tempfile::tempdir().unwrap();
            let bundle = fake_bundle(dir.path());
            let body = archive(b"new");
            let (public_key, signature) = signed(&body, "0.3.9");
            let url = serve(body).await;
            let settings = Settings { endpoint: String::new(), public_key, install_place: Some(bundle.clone()) };
            let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
            let error = download_and_install(update(url, signature, "0.4.0"), settings, sender).await.unwrap_err();
            assert!(error.contains("0.3.9"), "{error}");
            assert_eq!(std::fs::read(bundle.join("Contents/MacOS/DBForge")).unwrap(), b"old");
        }
    }

    #[gpui::test]
    fn turns_the_daily_check_on_and_off_in_the_preferences(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        cx.update(|cx| {
            assert!(Updates::automatic(cx), "on until turned off");
            Updates::set_automatic(false, cx);
            assert!(!Updates::automatic(cx));
            assert_eq!(Preferences::global(cx).get(keys::UPDATES_AUTOMATIC), Some("false"));
        });
    }

    #[gpui::test]
    fn announces_a_new_version_at_once_then_once_a_day(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let updates = cx.update(|cx| {
            Updates::start_with(Settings::current(), false, cx);
            Updates::global(cx)
        });
        let found = |version: &str| Ok(Some(Update { version: version.into(), ..Arc::unwrap_or_clone(update()) }));
        const NOW: i64 = 1_800_000_000_000;
        const HOUR: i64 = 60 * 60 * 1000;
        updates.update(cx, |updates, cx| {
            updates.receive(false, found("9.0.0"), NOW, cx);
            assert!(matches!(updates.phase, Phase::Available(_)), "a version found for the first time shows");
            updates.dismiss(cx);

            updates.receive(false, found("9.0.0"), NOW + HOUR, cx);
            assert_eq!(updates.phase, Phase::Idle, "the hourly look keeps quiet about it");
            updates.receive(false, found("9.0.0"), NOW + 23 * HOUR, cx);
            assert_eq!(updates.phase, Phase::Idle);
            updates.receive(false, found("9.0.0"), NOW + 24 * HOUR, cx);
            assert!(matches!(updates.phase, Phase::Available(_)), "a day later it is told again");
            updates.dismiss(cx);

            updates.receive(false, found("9.0.1"), NOW + 25 * HOUR, cx);
            assert!(matches!(updates.phase, Phase::Available(_)), "a newer release shows at once");
            updates.dismiss(cx);

            updates.receive(true, found("9.0.1"), NOW + 26 * HOUR, cx);
            assert!(matches!(updates.phase, Phase::Available(_)), "a check the user asks for always answers");
            assert_eq!(
                Preferences::global(cx).get(keys::UPDATES_ANNOUNCED),
                Some(format!("9.0.1@{}", NOW + 26 * HOUR).as_str())
            );
        });
    }

    #[gpui::test]
    fn dismisses_an_answer_but_never_an_install(cx: &mut TestAppContext) {
        let _dir = setup(cx);
        let updates = cx.update(|cx| {
            Updates::start_with(Settings::current(), false, cx);
            Updates::global(cx)
        });
        updates.update(cx, |updates, cx| {
            updates.show_for_development(Phase::Available(update()), cx);
            updates.dismiss(cx);
            assert_eq!(updates.phase, Phase::Idle);
            let installing = Phase::Installing { update: update(), stage: Stage::Installing };
            updates.show_for_development(installing.clone(), cx);
            updates.dismiss(cx);
            assert_eq!(updates.phase, installing);
            updates.check(true, cx);
            assert_eq!(updates.phase, installing, "no check while installing");
        });
    }
}
