//! Renders a stub library on every Classic theme screen, and in another theme, to PNG files, with
//! Slint's software renderer and no window. Each test installs its own Slint
//! platform, on its own thread.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use crate::common::*;
use launcher_core::{Area, Command, Pref, Route, SessionCmd};
use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint_interpreter::ComponentHandle;

struct Offscreen(Rc<MinimalSoftwareWindow>);

thread_local! {
    /// Slint's time on this thread: it moves only when [`pass`] moves it.
    static ELAPSED: std::cell::Cell<Duration> = const { std::cell::Cell::new(Duration::ZERO) };
}

/// Moves Slint's time on by `by`, for its timers and animations.
fn pass(by: Duration) {
    ELAPSED.with(|e| e.set(e.get() + by));
}

/// Runs the core on its clock until `done`, for at most `ms` milliseconds of it.
fn run_until(host: &Composed, ms: u64, done: impl Fn(&launcher_core::Core) -> bool) {
    for _ in 0..ms / 10 {
        if done(host.core()) {
            return;
        }
        host.advance(Duration::from_millis(10));
    }
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
    fn duration_since_start(&self) -> Duration {
        ELAPSED.with(|e| e.get())
    }
    fn debug_log(&self, arguments: std::fmt::Arguments) {
        if keeps_log_line(&arguments) {
            eprintln!("{arguments}");
        }
    }
}

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 760;

fn shoot(window: &MinimalSoftwareWindow, path: &Path) {
    let size = window.size();
    let (width, height) = (size.width, size.height);
    let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
    // A screen is made, and its entrance starts, at its first frame; draw that frame, let the
    // animations finish, then draw the one to keep.
    slint::platform::update_timers_and_animations();
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    // A timer that a change handler starts runs from the next pass: the hero's and the
    // art's wait for details takes three, as the event loop would make them.
    for _ in 0..2 {
        pass(Duration::from_millis(500));
        slint::platform::update_timers_and_animations();
    }
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    image::save_buffer(path, &bytes, width, height, image::ColorType::Rgba8).unwrap();
}

/// The next frame only, with no time for animations: a screen's first frame.
fn shoot_first(window: &MinimalSoftwareWindow, path: &Path) {
    let size = window.size();
    let mut pixels = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
    });
    let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
    image::save_buffer(path, &bytes, size.width, size.height, image::ColorType::Rgba8).unwrap();
}

/// Runs the theme's change handlers, then carries out what they asked for: the download sheet
/// opens Downloads once the download is there.
fn follow(host: &mut Composed) {
    host.ui.render().unwrap();
    slint::platform::update_timers_and_animations();
    host.pump();
    host.load_images();
}

fn key(host: &mut Composed, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    host.pump();
    host.load_images();
}

/// Every Classic theme screen in one direction, as `classic[-rtl]-<screen>.png`.
fn shoot_classic(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Home));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-home.png")));
    shoot_notices(host, window, out, prefix);
    // Down to the buttons under the game.
    key(host, Key::DownArrow);
    shoot(window, &out.join(format!("{prefix}-actions.png")));
    key(host, Key::UpArrow);
    // Up to the top bar, on to Library (left of Home from right to left), and in.
    key(host, Key::UpArrow);
    key(host, if prefix.ends_with("rtl") { Key::LeftArrow } else { Key::RightArrow });
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-library.png")));
    // The list of sorts, then Back closes it.
    host.first("library-sort").invoke_accessible_default_action();
    host.pump();
    shoot(window, &out.join(format!("{prefix}-sort.png")));
    key(host, Key::Escape);
    key(host, Key::Return);
    // The Game Hub's first frame: its cover on the card it opens from.
    shoot_first(window, &out.join(format!("{prefix}-detail-opening.png")));
    shoot(window, &out.join(format!("{prefix}-detail.png")));
    key(host, Key::Escape);
    key(host, Key::Escape);
    for (route, name) in [
        (Route::Catalog, "catalog"),
        (Route::Downloads, "downloads"),
        (Route::Emulators, "emulators"),
        (Route::Themes, "themes"),
        (Route::Settings, "settings"),
    ] {
        host.dispatch(Command::Navigate(route.clone()));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-{name}.png")));
        if route == Route::Settings {
            // Up to the rail, then down it, as a pad does.
            let tabs = ["games", "updates", "downloads", "this-pc", "backup", "power"];
            key(host, Key::UpArrow);
            for tab in tabs {
                key(host, Key::DownArrow);
                host.load_images();
                shoot(window, &out.join(format!("{prefix}-settings-tab-{tab}.png")));
            }
            key(host, Key::Escape);
        }
    }
    // A game the bundled catalog offers, when it offers any: its Game Hub with its releases.
    host.dispatch(Command::Navigate(Route::Catalog));
    if let Some(offer) = host.core().view().catalog.games.games.first().cloned() {
        host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-offer.png")));
        // Down past its releases opens the technical details; Up closes them.
        let releases = host.core().view().detail.map_or(0, |d| d.releases.len());
        for _ in 0..=releases {
            key(host, Key::DownArrow);
        }
        shoot(window, &out.join(format!("{prefix}-offer-details.png")));
        key(host, Key::UpArrow);
        // Its first release: the sheet that asks first, then the download, read and waiting.
        // Once: in the other direction it is in Downloads already.
        let detail = host.core().view().detail.unwrap();
        if let Some(release) = detail.releases.first().filter(|r| r.download.is_empty()) {
            host.test.torrents.has(
                &release.id,
                "Sample",
                &[("Sample/game.rar", b"archive"), ("Sample/readme.txt", b"hi")],
            );
            host.test.archives.holds("game.rar", &[("game.pkg", b"package")]);
            let held = host.test.archives.hold();
            key(host, Key::DownArrow);
            key(host, Key::Return);
            shoot(window, &out.join(format!("{prefix}-download-sheet.png")));
            key(host, Key::Return);
            run_until(host, 500, |core| core.view().downloads.torrents.first().is_some_and(|t| t.state == "ready"));
            follow(host);
            shoot(window, &out.join(format!("{prefix}-downloads-torrent.png")));
            // The keys on the download's first button.
            key(host, Key::DownArrow);
            shoot(window, &out.join(format!("{prefix}-downloads-keys.png")));
            // Downloaded: its archive unpacks. Once: the next theme finds it unpacked.
            let first = host.core().view().downloads.torrents.first().is_some_and(|t| t.state == "ready");
            if first {
                // Download opens the download sheet: its files, the drives and the game folder.
                key(host, Key::Return);
                shoot_strip(window, out, &format!("{prefix}-files-sheet-open"));
                shoot(window, &out.join(format!("{prefix}-files-sheet.png")));
                // Download only: the archive unpacks, and the install sheet comes later.
                i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "start-download")
                    .next()
                    .expect("the sheet's Download")
                    .invoke_accessible_default_action();
                host.pump();
                // Bytes arrive: the card says the time left, and Open folder follows its buttons.
                host.test.torrents.progress(&release.id, 4);
                host.test.torrents.rate(&release.id, 1);
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.seconds_left > 0)
                });
                host.ui.render().unwrap();
                shoot(window, &out.join(format!("{prefix}-downloads-downloading.png")));
                host.test.torrents.complete(&release.id);
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.unpack == "unpacking")
                });
                host.pump();
                host.ui.render().unwrap();
                // The card's buttons follow its actions on a drawn window, not only on the
                // testing backend: Pause, drawn while it downloaded, is gone.
                let shown = ["pause", "stop-seeding", "cancel"].map(|a| host.count(&format!("torrent-action:{a}")));
                assert_eq!(shown, [0, 1, 1]);
                shoot(window, &out.join(format!("{prefix}-downloads-unpacking.png")));
                held.release();
                run_until(host, 1000, |core| {
                    core.view().downloads.torrents.first().is_some_and(|t| t.unpack == "unpacked")
                });
                host.ui.render().unwrap();
                shoot(window, &out.join(format!("{prefix}-downloads-unpacked.png")));
            }
            key(host, Key::Escape);
        }
        key(host, Key::Escape);
    }
    shoot_vpn(host, window, out, prefix);
    shoot_install_sheet(host, window, out, prefix);
    shoot_resolving(host, window, out, prefix);
    // KytyPS5's page, then down to its settings: Resolution, Video out and Present mode.
    host.dispatch(Command::Navigate(Route::Plugin("emu-kyty".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-plugin.png")));
    // R1 jumps on to its settings, on the same page.
    key(host, Key::Tab);
    std::thread::sleep(Duration::from_millis(300));
    follow(host);
    shoot(window, &out.join(format!("{prefix}-plugin-settings.png")));
    // The Game Catalog's page: its settings, the catalog sources and the catalog refresh.
    host.dispatch(Command::Navigate(Route::Plugin("release-catalog".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-catalog-plugin.png")));
    // A theme's sheet beside Themes: use it, and its options.
    host.dispatch(Command::Navigate(Route::Themes));
    host.dispatch(Command::Navigate(Route::Plugin("classic".parse().unwrap())));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-theme.png")));
    host.dispatch(Command::Navigate(Route::Home));
}

/// The install sheet of the finished download, as `<prefix>-install-sheet.png`, then with the
/// core's refusal of a folder outside the game folders, as `<prefix>-install-sheet-error.png`.
fn shoot_install_sheet(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    shoot_strip(window, out, &format!("{prefix}-downloads-open"));
    // The screen's change handler runs on the next pass, and resets the screen's keys.
    slint::platform::update_timers_and_animations();
    let Some(t) = host.core().view().downloads.torrents.into_iter().find(|t| t.actions.iter().any(|a| a == "install"))
    else {
        return;
    };
    let install =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "torrent-action:install")
            .find(|e| e.accessible_value().as_deref() == Some(t.id.as_str()))
            .expect("the download's Install");
    install.invoke_accessible_default_action();
    host.pump();
    shoot_strip(window, out, &format!("{prefix}-install-sheet-open"));
    shoot(window, &out.join(format!("{prefix}-install-sheet.png")));
    let outside = Command::GameInstall(launcher_core::GameInstallCmd::Download {
        id: t.id.clone(),
        folder: Some("/elsewhere".into()),
    });
    assert!(host.rt.block_on(host.test.core.dispatch(outside)).is_err(), "a folder outside the game folders");
    host.ui.render().unwrap();
    shoot(window, &out.join(format!("{prefix}-install-sheet-error.png")));
    key(host, slint::platform::Key::Escape);
}

/// The Default theme's download sheet of a Ready download, its game and an update, as
/// `default-files-sheet.png`. It leaves the download Ready, and the Catalog open.
fn shoot_default_files_sheet(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path) {
    let games = host.core().view().catalog.games.games;
    let offer = games.iter().find(|g| g.platform != "PC").cloned().expect("a console game in the catalog");
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    let release = host.core().view().detail.and_then(|d| d.releases.first().cloned()).expect("a release");
    let files: &[(&str, &[u8])] = &[("Game/game.pkg", b"package"), ("Game/Update/update-1.02.pkg", b"update")];
    host.test.torrents.has(&release.id, "Game", files);
    host.dispatch(Command::Torrent(launcher_core::TorrentCmd::Download {
        offer: offer.key.parse().unwrap(),
        release: release.id.clone(),
        folder: None,
    }));
    run_until(host, 1000, |core| core.view().downloads.torrents.iter().any(|t| t.state == "ready"));
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    follow(host);
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), "torrent-action:start")
        .find(|e| e.accessible_value().as_deref() == Some(release.id.as_str()))
        .expect("the download's Download")
        .invoke_accessible_default_action();
    host.pump();
    shoot(window, &out.join("default-files-sheet.png"));
    key(host, slint::platform::Key::Escape);
    host.dispatch(Command::Navigate(Route::Catalog));
    follow(host);
}

/// The second offer's download while it looks up its files, as `<prefix>-downloads-resolving.png`:
/// its first button says so and does nothing. It leaves no download behind.
fn shoot_resolving(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    host.dispatch(Command::Navigate(Route::Catalog));
    host.load_images();
    shoot_strip(window, out, &format!("{prefix}-catalog-open"));
    let games = host.core().view().catalog.games.games;
    let Some(offer) = games.iter().skip(1).find(|g| g.platform != "PC").cloned() else { return };
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    let Some(release) = host.core().view().detail.and_then(|d| d.releases.first().cloned()) else { return };
    host.test.torrents.has(&release.id, "Sample", &[("Sample/game.pkg", b"package")]);
    let held = host.test.torrents.hold();
    host.dispatch(Command::Torrent(launcher_core::TorrentCmd::Download {
        offer: offer.key.parse().unwrap(),
        release: release.id.clone(),
        folder: None,
    }));
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    shoot(window, &out.join(format!("{prefix}-downloads-resolving.png")));
    held.release();
    run_until(host, 1000, |core| {
        core.view().downloads.torrents.iter().any(|t| t.id == release.id && t.state == "ready")
    });
    host.dispatch(Command::Torrent(launcher_core::TorrentCmd::Act {
        id: release.id,
        action: launcher_core::TorrentAction::Remove,
    }));
}

/// Three notices on Home, one of each kind, as `<prefix>-notices.png`. The core makes a
/// success only for a real download and an info only for a real launch, so the shot writes the
/// notices straight into the theme's `Vm`, then closes them.
fn shoot_notices(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint_interpreter::{Struct, Value};
    let card = |id: f64, text: &str, kind: &str, opens: &str| {
        let fields: [(&str, Value); 5] = [
            ("id", Value::Number(id)),
            ("text", Value::String(text.into())),
            ("kind", Value::String(kind.into())),
            ("art", Value::Image(slint::Image::default())),
            ("opens", Value::String(opens.into())),
        ];
        Value::Struct(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect::<Struct>())
    };
    let cards = vec![
        card(9001.0, "The trailer opened in your browser", "info", ""),
        card(9002.0, "Download complete: Astro Drift. Install it from Downloads.", "success", "Open Downloads"),
        card(9003.0, "Can't start Lumen Gate: firmware missing", "error", "Open the Game Hub"),
    ];
    let model = slint::ModelRc::new(slint::VecModel::from(cards));
    host.instance().set_global_property("Vm", "notice-cards", Value::Model(model)).unwrap();
    shoot(window, &out.join(format!("{prefix}-notices.png")));
    host.instance().set_global_property("Vm", "notice-cards", Value::Model(slint::ModelRc::default())).unwrap();
}

/// "Download only through a VPN": Settings at its DOWNLOADS rows with two VPNs, then the
/// second offer's download sheet with the VPN connected and without it, and its download
/// waiting for the VPN, as `<prefix>-settings-downloads[-end].png`, `<prefix>-download-sheet-vpn.png`,
/// `<prefix>-download-sheet-no-vpn.png` and `<prefix>-downloads-waiting.png`. It leaves no
/// download behind, so the other direction shoots the same.
fn shoot_vpn(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path, prefix: &str) {
    use slint::platform::Key;
    host.test.network.up("wg-se");
    host.test.network.down("work");
    host.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(true)));
    host.dispatch(Command::Navigate(Route::Settings));
    host.load_images();
    // Up to the tabs, three to the right to DOWNLOADS, then down to the VPN switch.
    key(host, Key::UpArrow);
    for _ in 0..3 {
        key(host, Key::RightArrow);
    }
    key(host, Key::DownArrow);
    shoot(window, &out.join(format!("{prefix}-settings-downloads.png")));
    // Down past the VPNs and "Add a VPN" to sharing.
    for _ in 0..4 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join(format!("{prefix}-settings-downloads-end.png")));
    key(host, Key::Escape);

    host.dispatch(Command::Navigate(Route::Catalog));
    let games = host.core().view().catalog.games.games;
    let Some(offer) = games.iter().skip(1).find(|g| g.platform != "PC").cloned() else { return };
    host.dispatch(Command::OpenGame(offer.key.parse().unwrap()));
    let Some(release) = host.core().view().detail.and_then(|d| d.releases.first().cloned()) else { return };
    host.test.torrents.has(&release.id, "Sample", &[("Sample/game.pkg", b"package")]);
    host.load_images();
    // Down to its first release, as on the first offer, and Return opens the sheet.
    key(host, Key::DownArrow);
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-download-sheet-vpn.png")));
    // A typed folder outside the packages folders: the core refuses, and the sheet says why.
    key(host, Key::UpArrow);
    key(host, Key::Return);
    for c in "/elsewhere".chars() {
        let text: slint::SharedString = c.to_string().into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    key(host, Key::Return);
    let text: slint::SharedString = Key::Return.into();
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    let refused = host.requests.borrow_mut().drain(..).collect::<Vec<_>>();
    for request in refused {
        if let launcher_ui_host::UiRequest::Core(command) = request {
            assert!(host.rt.block_on(host.test.core.dispatch(command)).is_err(), "the core refuses the folder");
        }
    }
    host.ui.render().unwrap();
    shoot(window, &out.join(format!("{prefix}-download-sheet-error.png")));
    // The VPN drops: the setting reads it again. The keys are still on the release.
    key(host, Key::Escape);
    host.test.network.down("wg-se");
    host.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(true)));
    key(host, Key::Return);
    shoot(window, &out.join(format!("{prefix}-download-sheet-no-vpn.png")));
    key(host, Key::Return);
    follow(host);
    shoot(window, &out.join(format!("{prefix}-downloads-waiting.png")));

    // Off: the download reads its files, then goes.
    host.dispatch(Command::Vpn(launcher_core::VpnCmd::SetOnly(false)));
    run_until(host, 1000, |core| {
        core.view().downloads.torrents.iter().any(|t| t.id == release.id && t.state == "ready")
    });
    if host.core().view().downloads.torrents.iter().any(|t| t.id == release.id) {
        host.dispatch(Command::Torrent(launcher_core::TorrentCmd::Act {
            id: release.id,
            action: launcher_core::TorrentAction::Remove,
        }));
    }
    key(host, Key::Escape);
    key(host, Key::Escape);
}

/// Settings at its LAUNCHER rows, with "Size of everything" focused and then "Start at login",
/// then Home at the smallest and the largest size, as `classic-settings-launcher.png`,
/// `classic-settings-login.png` and `classic-home-<percent>.png`.
fn shoot_sizes(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Settings));
    host.load_images();
    // Up to the tabs, then down into LAUNCHER, the first tab, to Full screen.
    key(host, Key::UpArrow);
    key(host, Key::DownArrow);
    shoot(window, &out.join("classic-settings-launcher.png"));
    // Down past the size, Back to the launcher and Interface sounds to Start at login.
    for _ in 0..4 {
        key(host, Key::DownArrow);
    }
    shoot(window, &out.join("classic-settings-login.png"));
    key(host, Key::Escape);
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::Pref(Pref::Scale, size.into()));
        host.dispatch(Command::Navigate(Route::Home));
        host.load_images();
        shoot(window, &out.join(format!("classic-home-{name}.png")));
        // Settings at the size: the tabs and the longest rows.
        host.dispatch(Command::Navigate(Route::Settings));
        host.load_images();
        shoot(window, &out.join(format!("classic-settings-{name}.png")));
        key(host, Key::Escape);
    }
    host.dispatch(Command::Pref(Pref::Scale, 1.0.into()));
}

/// A picture `width` by `height` that fades from `from` to `to`, as PNG bytes.
fn picture(width: u32, height: u32, from: [u8; 3], to: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let t = (x + y) as f32 / (width + height) as f32;
        image::Rgb(std::array::from_fn(|i| (from[i] as f32 + (to[i] as f32 - from[i] as f32) * t) as u8))
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// Pictures at the sample catalogs' made-up Steam art addresses.
fn serve_steam_art(host: &Composed) {
    for (app, from, to) in [(3000001, [40, 90, 160], [10, 20, 50]), (3000002, [170, 90, 40], [40, 15, 10])] {
        let art = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{app}");
        host.test.http.serve(&format!("{art}/library_600x900_2x.jpg"), picture(600, 900, from, to));
        host.test.http.serve(&format!("{art}/library_hero.jpg"), picture(1920, 620, to, from));
        host.test.http.serve(&format!("{art}/header.jpg"), picture(920, 430, from, to));
        for shot in ["ss_1", "ss_2"] {
            host.test.http.serve(&format!("{art}/{shot}.jpg"), picture(1280, 720, to, from));
        }
    }
}

/// The offers with Steam data: a PS5 package's Game Hub and its sheet, and a PC game packed
/// with Wine with its ProtonDB card and its menu.
fn shoot_steam_offers(host: &mut Composed, window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    host.dispatch(Command::Navigate(Route::Catalog));
    let offers = host.core().view().catalog.games.games;
    let key_of = |title: &str| offers.iter().find(|o| o.title == title).map(|o| o.key.clone()).unwrap();
    host.dispatch(Command::OpenGame(key_of("Sample Without Title Id").parse().unwrap()));
    host.load_images();
    shoot(window, &out.join("classic-offer-package.png"));
    key(host, Key::DownArrow);
    key(host, Key::Return);
    shoot(window, &out.join("classic-download-sheet-package.png"));
    key(host, Key::Escape);
    key(host, Key::Escape);
    host.dispatch(Command::OpenGame(key_of("Sample Windows Game").parse().unwrap()));
    host.load_images();
    shoot(window, &out.join("classic-offer-pc.png"));
    key(host, Key::Menu);
    shoot(window, &out.join("classic-offer-pc-menu.png"));
    key(host, Key::Escape);
    key(host, Key::Escape);
    host.dispatch(Command::Navigate(Route::Home));
}

/// A PC with a strong graphics card, or one whose driver is too old for Vulkan 1.3.
fn this_pc(vulkan: (u32, u32)) -> launcher_core::Specs {
    let gpu = launcher_core::Gpu {
        name: "Test GPU".into(),
        memory_gb: Some(24.0),
        discrete: true,
        unified: false,
        vulkan: Some(vulkan),
    };
    launcher_core::Specs {
        cpu: "Test CPU".into(),
        cores: 16,
        threads: 32,
        memory_gb: 64.0,
        avx2: Some(true),
        gpus: Some(vec![gpu]),
    }
}

/// A PS5 offer on a PC that can't start it, with the notice at the start and the sheet's
/// warnings, then on a PC where it doesn't fit, as `classic-this-pc-*.png`.
fn shoot_this_pc(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    for (name, vulkan) in [("no", (1, 2)), ("space", (1, 3))] {
        let home = tempfile::tempdir().unwrap();
        let mut host = composed_on_current_platform(|h| {
            h.home(home.path())
                .hardware(this_pc(vulkan))
                .free_space(10_000_000_000)
                .plugin(FakeCatalog::kyty().playable().requires("vulkan = \"1.3\"").build())
                .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        host.ui.start().unwrap();
        let topics = serde_json::json!([{ "id": "1", "title": "[PS5] Astro Drift [EUR / v01.05]", "size": "50 GB",
            "magnet": "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335aa7c1367a88a",
            "game_info": { "name": "Astro Drift", "title_id": "PPSA01234", "version": "v01.05", "region": "EUR" } }]);
        host.serve_catalog(&[("ps5", topics)]);
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.dispatch(Command::Navigate(Route::Home));
        host.load_images();
        shoot(window, &out.join(format!("classic-this-pc-{name}-start.png")));
        host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
        host.load_images();
        shoot(window, &out.join(format!("classic-this-pc-{name}.png")));
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::Return);
        shoot(window, &out.join(format!("classic-this-pc-{name}-sheet.png")));
        assert_served(&host, &[]);
    }
}

/// A Game Hub's releases as `classic[-rtl]-releases.png`: long titles, New tags, a warning, and
/// the keys on the second row. Each row's Download button ends at the same place.
fn shoot_releases(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    for (name, direction) in [("classic", "ltr"), ("classic-rtl", "rtl")] {
        let home = tempfile::tempdir().unwrap();
        let mut host = composed_on_current_platform(|h| {
            h.home(home.path())
                .plugin(FakeCatalog::kyty().playable().requires("vulkan = \"1.3\"").build())
                .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        // The first two from this week: New.
        let topic = |id: u32, flag: &str, version: &str, region: &str| {
            let added = crate::releases::days_ago(if id <= 2 { 4 } else { 24 });
            serde_json::json!({ "id": id.to_string(), "title": format!("[PS5] Astro Drift [{region} / {version} / {flag}]"),
                "published_at": added,
                "size": format!("{id}.4 GB"), "magnet": format!("magnet:?xt=urn:btih:{id:040x}"),
                "game_info": { "name": "Astro Drift", "title_id": "PPSA01234", "version": version, "region": region } })
        };
        let topics = serde_json::json!([
            topic(
                1,
                "fpkg",
                "1.000.010 (dated 30.03.2026)",
                "EU² / US² / JP² / AS² / INT(GLO)² / EU¹ / US¹ / JP¹ / AS¹"
            ),
            topic(2, "folder", "1.000.010 (dated 30.03.2026)", "EU¹ / US¹ / JP¹ / AS¹ / INT(GLO)¹"),
            topic(3, "fpkg", "v01.000.000 (v01.000.010)", "EUR"),
            topic(4, "folder", "v01.000.000 (v01.000.010)", "EUR"),
        ]);
        host.serve_catalog(&[("ps5", topics)]);
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.dispatch(Command::OpenGame("release-catalog:emu-kyty/PPSA01234".parse().unwrap()));
        host.load_images();
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::DownArrow);
        shoot(window, &out.join(format!("{name}-releases.png")));
    }
}

/// The Library's header with a controller, as `classic-pad-hints.png`, and its key hints cut
/// out at four times the size as `classic-pad-hints-zoom.png`: the d-pad's two pairs, ⮜  ⮞ and
/// ⮝  ⮟, are the same size.
fn shoot_pad_hints(window: &MinimalSoftwareWindow, out: &Path) {
    use launcher_ui_host::PadButton;
    let mut host = composed_on_current_platform(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha", "Beta"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.load_images();
    let pad = |host: &mut Composed, button: PadButton| {
        host.ui.pad(button).unwrap();
        follow(host);
        follow(host);
    };
    // Up to the Library tab, in, then up to its header.
    for button in [PadButton::Up, PadButton::Right, PadButton::South, PadButton::Up] {
        pad(&mut host, button);
    }
    let full = out.join("classic-pad-hints.png");
    shoot(window, &full);
    let image = image::open(&full).unwrap();
    let (w, h) = (image.width(), image.height());
    let hints = image.crop_imm(w / 2, h - 90, w / 2, 90);
    hints
        .resize(hints.width() * 4, hints.height() * 4, image::imageops::FilterType::Nearest)
        .save(out.join("classic-pad-hints-zoom.png"))
        .unwrap();
}

/// The Library's genres in a narrow window, with the keys on the last chip, left to right and
/// right to left, as `classic[-rtl]-genres.png`: the row scrolls, and fades where chips hide.
fn shoot_genres(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    window.set_size(slint::PhysicalSize::new(640, 900));
    for (direction, prefix) in [("ltr", "classic"), ("rtl", "classic-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
                .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
        });
        host.ui.start().unwrap();
        host.dispatch(Command::Pref(Pref::Scale, 1.4.into()));
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.until(Area::Library, |core| core.view().home.library.genres.len() >= 3);
        host.load_images();
        // Up to the Library tab, in, then up to the genres and on to the last one.
        key(&mut host, Key::UpArrow);
        key(&mut host, if direction == "rtl" { Key::LeftArrow } else { Key::RightArrow });
        key(&mut host, Key::Return);
        shoot(window, &out.join(format!("{prefix}-genres-start.png")));
        key(&mut host, Key::UpArrow);
        for _ in 0..2 {
            key(&mut host, if direction == "rtl" { Key::LeftArrow } else { Key::RightArrow });
        }
        shoot(window, &out.join(format!("{prefix}-genres.png")));
    }
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
}

/// A trailer in the window: playing with its controls, playing once they hid, and paused, left
/// to right and right to left, as `classic[-rtl]-trailer[-hidden|-paused].png`. No frame comes,
/// so the picture is black.
fn shoot_trailer(window: &MinimalSoftwareWindow, out: &Path) {
    use launcher_ui_host::TrailerStatus;
    for (direction, prefix) in [("ltr", "classic"), ("rtl", "classic-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(FakeCatalog::new("emu-a").system("PS4").games(["Alpha"]).build())
                .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").build())
        });
        host.ui.set_trailers_available(true);
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.until(Area::Library, |core| core.view().detail.is_some_and(|d| !d.trailer_url.is_empty()));
        host.load_images();
        let text: slint::SharedString = "t".into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text });
        let playing = TrailerStatus { playing: true, loading: false, position: 42.0, duration: 130.0 };
        host.ui.set_trailer(Some(playing.clone()));
        host.ui.render().unwrap();
        shoot_strip(window, out, &format!("{prefix}-trailer-open"));
        shoot(window, &out.join(format!("{prefix}-trailer.png")));
        // The controls hide 3 s after the last key.
        for _ in 0..7 {
            pass(Duration::from_millis(500));
            slint::platform::update_timers_and_animations();
        }
        shoot(window, &out.join(format!("{prefix}-trailer-hidden.png")));
        host.ui.set_trailer(Some(TrailerStatus { playing: false, ..playing }));
        host.ui.render().unwrap();
        shoot(window, &out.join(format!("{prefix}-trailer-paused.png")));
    }
}

/// The on-screen keyboard over the Library's search, opened with the controller: with the last
/// searches over the keys as `classic-keyboard-recent.png`, then with "q" typed, its best match
/// over the keys and the letters with Shift on as `classic-keyboard.png`, the symbols as
/// `classic-keyboard-symbols.png`, then at the smallest and largest size, at a 1.5 scale factor
/// and right to left, as `classic-keyboard-<percent>.png`, `classic-keyboard-scale-150.png` and
/// `classic-rtl-keyboard.png`. The keys are drawn on whole pixels at every one of them.
fn shoot_keyboard(window: &MinimalSoftwareWindow, out: &Path) {
    use launcher_ui_host::PadButton;
    let mut host = composed_on_current_platform(|h| {
        h.plugin(FakeCatalog::new("emu-a").system("PS5").games(["Astro Drift", "Quest Runner"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    for text in ["astro drift", "quest"] {
        host.dispatch(Command::RememberSearch(text.into()));
    }
    let pad = |host: &mut Composed, button: PadButton| {
        host.ui.pad(button).unwrap();
        follow(host);
        follow(host);
    };
    for button in [PadButton::Up, PadButton::Right, PadButton::South, PadButton::North] {
        pad(&mut host, button);
    }
    shoot(window, &out.join("classic-keyboard-recent.png"));
    pad(&mut host, PadButton::South);
    // The search the field runs once typing stops for a moment.
    host.dispatch(Command::LibrarySearch("q".into()));
    follow(&mut host);
    pad(&mut host, PadButton::LeftShoulder);
    shoot(window, &out.join("classic-keyboard.png"));
    pad(&mut host, PadButton::RightShoulder);
    shoot(window, &out.join("classic-keyboard-symbols.png"));
    pad(&mut host, PadButton::RightShoulder);
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::Pref(Pref::Scale, size.into()));
        shoot(window, &out.join(format!("classic-keyboard-{name}.png")));
    }
    host.dispatch(Command::Pref(Pref::Scale, 1.0.into()));
    // The same window on a screen with 1.5 pixels to the point.
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    window.set_size(slint::PhysicalSize::new(WIDTH * 3 / 2, HEIGHT * 3 / 2));
    host.ui.render().unwrap();
    shoot(window, &out.join("classic-keyboard-scale-150.png"));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    shoot(window, &out.join("classic-rtl-keyboard.png"));
}

/// The machine's settings as the whole screen, over a fake system plugin: the top bar's icons
/// with the network panel open as `classic-system-network.png`, the sound and Bluetooth panels,
/// the panel at the smallest and largest size and at a 1.5 scale factor, and the USB stick's
/// confirmation as `classic-system-install.png`.
fn shoot_system(window: &MinimalSoftwareWindow, out: &Path) {
    use launcher_core::system::{Place, SystemCommand};
    let system = launcher_testkit::FakeSystem::new();
    system.with(|m| {
        m.network = launcher_api::NetworkStatus {
            connection: launcher_api::Connection::Wifi,
            name: "Home".into(),
            signal: Some(60),
        };
        m.wifi = vec![
            launcher_api::WifiNetwork { ssid: "Home".into(), signal: 60, secured: true, saved: true, active: true },
            launcher_api::WifiNetwork { ssid: "Cafe".into(), signal: 25, secured: false, saved: false, active: false },
            launcher_api::WifiNetwork {
                ssid: "Neighbour".into(),
                signal: 90,
                secured: true,
                saved: false,
                active: false,
            },
        ];
        m.bluetooth_on = true;
        m.devices = vec![
            launcher_api::BluetoothDevice {
                address: "AA".into(),
                name: "DualSense".into(),
                paired: true,
                connected: true,
            },
            launcher_api::BluetoothDevice {
                address: "BB".into(),
                name: "Headset".into(),
                paired: true,
                connected: false,
            },
        ];
        m.outputs = vec![
            launcher_api::AudioOutput {
                name: "hdmi".into(),
                description: "TV or monitor (HDMI)".into(),
                current: true,
            },
            launcher_api::AudioOutput { name: "hp".into(), description: "Headphones".into(), current: false },
        ];
        m.level = launcher_api::AudioLevel { volume: 65, muted: false };
        m.battery = Some(launcher_api::Battery { percent: 18, charging: false });
        m.disks = vec![launcher_api::InstallDisk {
            path: "/dev/nvme0n1".into(),
            bytes: 512_000_000_000,
            windows: true,
            usb: false,
            model: "Samsung 970".into(),
        }];
    });
    let place = Place { whole_screen: true, own_os: true, stick: false };
    let plugin = system.plugin();
    let mut host = composed_on_current_platform(|h| {
        h.place(place).plugin(plugin).plugin(FakeCatalog::new("emu-a").system("PS5").games(["Astro Drift"]).build())
    });
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::System(SystemCommand::Refresh));
    // The battery at 18% raises the low battery notice. The screenshots here show none.
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    let open = |host: &mut Composed, label: &str| {
        let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), label);
        found.into_iter().next().expect("the icon").invoke_accessible_default_action();
        follow(host);
        follow(host);
    };
    open(&mut host, "status:network");
    shoot(window, &out.join("classic-system-network.png"));
    open(&mut host, "status:sound");
    shoot(window, &out.join("classic-system-sound.png"));
    open(&mut host, "status:bluetooth");
    shoot(window, &out.join("classic-system-bluetooth.png"));
    open(&mut host, "status:network");
    for (size, name) in [(0.8, "80"), (1.4, "140")] {
        host.dispatch(Command::Pref(Pref::Scale, size.into()));
        shoot(window, &out.join(format!("classic-system-{name}.png")));
    }
    host.dispatch(Command::Pref(Pref::Scale, 1.0.into()));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    window.set_size(slint::PhysicalSize::new(WIDTH * 3 / 2, HEIGHT * 3 / 2));
    host.ui.render().unwrap();
    shoot(window, &out.join("classic-system-scale-150.png"));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    drop(host);

    let place = Place { whole_screen: true, own_os: true, stick: true };
    let plugin = system.plugin();
    let mut host = composed_on_current_platform(|h| h.place(place).plugin(plugin));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::System(SystemCommand::Refresh));
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    follow(&mut host);
    for key in [slint::platform::Key::DownArrow, slint::platform::Key::Return, slint::platform::Key::Return] {
        let text: slint::SharedString = key.into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
        follow(&mut host);
    }
    shoot(window, &out.join("classic-system-install.png"));
    drop(host);

    // Settings' System and Power tabs in a desktop's Marah Launcher session, where Log out shows.
    let place = Place { whole_screen: true, own_os: false, stick: false };
    let plugin = system.plugin();
    let mut host = composed_on_current_platform(|h| h.place(place).plugin(plugin));
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::System(SystemCommand::Refresh));
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    host.dispatch(Command::Navigate(Route::Settings));
    follow(&mut host);
    key(&mut host, slint::platform::Key::UpArrow);
    for tab in ["games", "updates", "downloads", "this-pc", "system", "power"] {
        key(&mut host, slint::platform::Key::DownArrow);
        if matches!(tab, "system" | "power") {
            shoot(window, &out.join(format!("classic-settings-tab-{tab}-session.png")));
        }
    }
}

/// The launch splash of an emulator that lists its keyboard keys, left to right and right to
/// left, as `classic[-rtl]-splash-keys.png`.
fn shoot_splash(window: &MinimalSoftwareWindow, out: &Path) {
    let emulator = emu_kyty::KEYS
        .iter()
        .fold(FakeCatalog::new("emu-a").playable(), |emulator, (button, keys)| emulator.key(button, keys))
        .games(["Alpha"]);
    for (direction, prefix) in [("ltr", "classic"), ("rtl", "classic-rtl")] {
        let emulator = emulator.clone();
        let mut host = composed_on_current_platform(move |h| h.plugin(emulator.build()));
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        // No window yet: the game stays "starting", behind the splash.
        host.test.windows.no_windows();
        host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-splash-keys.png")));
    }
}

/// Log out's closing screen in each built-in theme: while it closes a running game that won't
/// end (its art and cover), then, once the game is ended, at its last step (Marah Launcher over the wall
/// of the library's covers), as `<theme>-closing-game.png` and `<theme>-closing-step.png`.
fn shoot_closing(window: &MinimalSoftwareWindow, out: &Path) {
    const GAMES: [&str; 6] =
        ["Paper Knights", "Quiet Orbit", "Astro Drift", "Deep Lantern", "Ember Peaks", "Neon Harbor"];
    let colors: [([u8; 3], [u8; 3]); 6] = [
        ([40, 90, 170], [190, 70, 60]),
        ([30, 120, 90], [200, 180, 60]),
        ([120, 50, 160], [40, 160, 200]),
        ([200, 110, 40], [60, 30, 90]),
        ([20, 60, 120], [120, 200, 220]),
        ([160, 40, 90], [240, 170, 80]),
    ];
    for theme in [theme_classic::ID, theme_default::ID] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(FakeCatalog::new("emu-a").playable().games(GAMES).build())
                .plugin(FakeStore::new("store").covers().build())
        });
        for (i, (from, to)) in colors.iter().enumerate() {
            host.test.http.serve(&format!("https://art.example/CUSA0000{i}.png"), picture(600, 900, *from, *to));
        }
        host.ui.start_with(Some(id(theme))).unwrap();
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.test.windows.no_windows();
        host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
        host.test.spawner.ignore_terminate();
        let core = host.core().clone();
        host.rt
            .spawn(async move { core.dispatch(Command::System(launcher_core::system::SystemCommand::LogOut)).await });
        host.until(launcher_core::Area::Sessions, |core| core.view().closing_game.is_some());
        host.load_images();
        shoot(window, &out.join(format!("{theme}-closing-game.png")));

        // Past Stop's grace the game is killed, and Log out goes on to its last step.
        host.advance(std::time::Duration::from_secs(30));
        host.until(launcher_core::Area::Sessions, |core| core.log_out_ready());
        host.load_images();
        shoot(window, &out.join(format!("{theme}-closing-step.png")));
    }
}

/// Emulators named by their console, left to right and right to left: the first start's steps
/// as `classic[-rtl]-names-welcome.png`, the Emulators list with each build as
/// `classic[-rtl]-names-emulators.png`, and a first play's prompt as
/// `classic[-rtl]-names-install-prompt.png`.
fn shoot_console_names(window: &MinimalSoftwareWindow, out: &Path) {
    for (direction, prefix) in [("ltr", "classic"), ("rtl", "classic-rtl")] {
        let mut host = composed_on_current_platform(|h| {
            h.first_run()
                .plugin(
                    FakeCatalog::new("emu-k")
                        .named("PS5")
                        .emulator("KytyPS5")
                        .platform("PS5")
                        .without_slots()
                        .playable()
                        .installed("KytyPS5-2026-10-21-e2dd48a")
                        .games(["Astro Drift"])
                        .build(),
                )
                .plugin(
                    FakeCatalog::new("emu-s")
                        .named("PS4")
                        .emulator("shadPS4")
                        .platform("PS5")
                        .without_slots()
                        .playable()
                        .installed("v.0.20.0")
                        .games([])
                        .build(),
                )
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-names-welcome.png")));
        host.dispatch(Command::Pref(Pref::Welcomed, true.into()));
        host.dispatch(Command::Navigate(Route::Emulators));
        host.load_images();
        shoot(window, &out.join(format!("{prefix}-names-emulators.png")));
        drop(host);
        // KytyPS5 not installed yet: a first play asks to install it.
        let mut host = composed_on_current_platform(|h| {
            h.plugin(
                FakeCatalog::new("emu-k")
                    .named("PS5")
                    .emulator("KytyPS5")
                    .platform("PS5")
                    .without_slots()
                    .playable()
                    .installed("")
                    .games(["Astro Drift"])
                    .build(),
            )
        });
        host.ui.start().unwrap();
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.dispatch(Command::Session(SessionCmd::Launch("emu-k:G0".parse().unwrap())));
        host.load_images();
        shoot_strip(window, out, &format!("{prefix}-install-prompt-open"));
        shoot(window, &out.join(format!("{prefix}-names-install-prompt.png")));
    }
}

/// PS4 box art: a band of PlayStation blue with a white "PS4" across the top, then artwork.
fn ps4_box_art() -> Vec<u8> {
    let image = image::RgbImage::from_fn(600, 900, |x, y| match y {
        0..80 if (40..160).contains(&x) && (25..55).contains(&y) => image::Rgb([255, 255, 255]),
        0..80 => image::Rgb([10, 80, 190]),
        _ => image::Rgb([150 + (x / 8) as u8, 60, 40 + (y / 10) as u8]),
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A PS4 game whose store art is box art with the blue band, and whose community result says
/// when it was tested: its card and its Game Hub, as `classic-ps4-*.png`.
fn shoot_ps4_cover(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    let mut host = composed_on_current_platform(|h| {
        h.plugin(FakeCatalog::new("emu-b").tested().games(["Paper Knights", "Quiet Orbit"]).build())
            .plugin(FakeStore::new("store").knows("CUSA00000", 4.5, "").icon("https://art.example/ps4.png").build())
    });
    host.test.http.serve("https://art.example/ps4.png", ps4_box_art());
    host.ui.start().unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Navigate(Route::Home));
    host.load_images();
    shoot(window, &out.join("classic-ps4-home.png"));
    key(&mut host, Key::Return);
    host.load_images();
    shoot(window, &out.join("classic-ps4-detail.png"));
    // The same Game Hub from right to left: its buttons, the trailer's among them, and its facts.
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.load_images();
    shoot(window, &out.join("classic-rtl-ps4-detail.png"));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "ltr".into() });
    // The stub's screenshots: this picture is about the cover, so the Game Hub shows none.
    assert_served(&host, &["https://art.example/1.png".into(), "https://art.example/2.png".into()]);
}

/// Fails when `host` asked for a URL its fake web server doesn't serve, other than `missing`.
/// The fake answers those with a 404, and a plugin shows that as a notice on the screenshots.
fn assert_served(host: &Composed, missing: &[String]) {
    let unserved = host.test.http.unserved();
    assert_eq!(unserved, missing, "the screenshot host asked for URLs it doesn't serve");
}

/// This test thread's offscreen window (Slint's platform is per thread, so each test has its
/// own), and the folder the pictures go to.
fn offscreen() -> (Rc<MinimalSoftwareWindow>, std::path::PathBuf) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone()))).unwrap();
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("screenshots");
    std::fs::create_dir_all(&out).unwrap();
    (window, out)
}

/// The times of a frame strip, in ms after the move starts.
const STRIP: [u64; 7] = [0, 16, 80, 160, 260, 420, 1000];

/// A move as frames at the [`STRIP`] times, as `strips/<name>-<ms>.png`, and each frame's
/// hash in `strips/<name>.txt`: two runs of the same motion give the same file.
fn shoot_strip(window: &MinimalSoftwareWindow, out: &Path, name: &str) {
    use std::hash::{Hash, Hasher};
    let dir = out.join("strips");
    std::fs::create_dir_all(&dir).unwrap();
    let size = window.size();
    let mut hashes = String::new();
    let mut at = 0;
    for ms in STRIP {
        pass(Duration::from_millis(ms - at));
        at = ms;
        slint::platform::update_timers_and_animations();
        let mut pixels = vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size.width as usize);
        });
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        hashes.push_str(&format!("{ms} {:016x}\n", hasher.finish()));
        let path = dir.join(format!("{name}-{ms:04}.png"));
        image::save_buffer(path, &bytes, size.width, size.height, image::ColorType::Rgba8).unwrap();
    }
    std::fs::write(dir.join(format!("{name}.txt")), hashes).unwrap();
}

/// A library of six games with covers, in the Default theme.
fn covered(first_run: bool) -> Composed {
    const GAMES: [&str; 6] =
        ["Paper Knights", "Quiet Orbit", "Astro Drift", "Deep Lantern", "Ember Peaks", "Neon Harbor"];
    let mut host = composed_on_current_platform(|h| {
        let h = if first_run { h.first_run() } else { h };
        h.plugin(FakeCatalog::new("emu-a").playable().games(GAMES).build())
            .plugin(FakeStore::new("store").covers().build())
    });
    for i in 0..GAMES.len() {
        let (from, to) = ([40 + 30 * i as u8, 90, 170], [190, 70 + 25 * i as u8, 60]);
        host.test.http.serve(&format!("https://art.example/CUSA0000{i}.png"), picture(600, 900, from, to));
    }
    host.ui.start_with(Some(id(theme_default::ID))).unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.load_images();
    host
}

/// The motion the kit's work must keep: the Welcome screen's wall of covers, the closing
/// screen's, and Settings sliding in, with Animations on and off. Compare `strips/*.txt` before
/// and after a change of the kit's motion.
fn shoot_strips(window: &MinimalSoftwareWindow, out: &Path) {
    let mut host = covered(true);
    follow(&mut host);
    shoot_strip(window, out, "default-welcome");

    let mut host = covered(false);
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);
    shoot(window, &out.join("strips-home.png"));
    host.dispatch(Command::Navigate(Route::Settings));
    shoot_strip(window, out, "default-settings-open");
    host.dispatch(Command::Back);
    shoot_strip(window, out, "default-settings-close");
    shoot(window, &out.join("strips-home.png"));
    host.dispatch(Command::Navigate(Route::Emulators));
    shoot_strip(window, out, "default-emulators-open");
    host.dispatch(Command::Navigate(Route::Plugin("emu-a".parse().unwrap())));
    shoot_strip(window, out, "default-emulators-push");
    host.dispatch(Command::Back);
    shoot_strip(window, out, "default-emulators-pull");
    host.dispatch(Command::Back);
    shoot(window, &out.join("strips-home.png"));
    host.core().report("Couldn't read the folder".into());
    host.ui.render().unwrap();
    shoot_strip(window, out, "default-notice-in");
    // The pop-ups: the result prompt, the power menu and a game's menu.
    host.dispatch(Command::AskResult("emu-a:G1".parse().unwrap()));
    host.ui.render().unwrap();
    shoot_strip(window, out, "default-result-prompt");
    host.dispatch(Command::ReportResult(None));
    follow(&mut host);
    shoot(window, &out.join("strips-home.png"));
    let window_of = host.instance().window();
    for (text, pressed) in [
        (slint::platform::Key::Control.into(), true),
        ("p".into(), true),
        ("p".into(), false),
        (slint::platform::Key::Control.into(), false),
    ] {
        let text: slint::SharedString = text;
        let event = if pressed {
            slint::platform::WindowEvent::KeyPressed { text }
        } else {
            slint::platform::WindowEvent::KeyReleased { text }
        };
        window_of.dispatch_event(event);
    }
    host.pump();
    shoot_strip(window, out, "default-power-menu");
    key(&mut host, slint::platform::Key::Escape);
    follow(&mut host);
    host.dispatch(Command::OpenGame("emu-a:G1".parse().unwrap()));
    host.load_images();
    shoot_strip(window, out, "default-detail-open");
    follow(&mut host);
    shoot(window, &out.join("strips-detail.png"));
    key(&mut host, slint::platform::Key::Menu);
    shoot_strip(window, out, "default-game-menu");
    key(&mut host, slint::platform::Key::Escape);
    follow(&mut host);
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);

    host.dispatch(Command::Pref(Pref::Animations, false.into()));
    follow(&mut host);
    host.dispatch(Command::Navigate(Route::Settings));
    shoot_strip(window, out, "default-settings-open-still");
    host.dispatch(Command::Back);
    host.dispatch(Command::Pref(Pref::Animations, true.into()));
    follow(&mut host);

    host.dispatch(Command::Navigate(Route::Library));
    host.load_images();
    shoot_strip(window, out, "default-library-open");
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);

    host.test.windows.no_windows();
    host.dispatch(Command::Session(SessionCmd::Launch("emu-a:G0".parse().unwrap())));
    host.ui.render().unwrap();
    shoot_strip(window, out, "default-running-pill");
    host.test.spawner.ignore_terminate();
    let core = host.core().clone();
    host.rt.spawn(async move { core.dispatch(Command::System(launcher_core::system::SystemCommand::LogOut)).await });
    host.until(launcher_core::Area::Sessions, |core| core.view().closing_game.is_some());
    host.load_images();
    shoot_strip(window, out, "default-closing-in");
    host.advance(std::time::Duration::from_secs(30));
    host.until(launcher_core::Area::Sessions, |core| core.log_out_ready());
    host.load_images();
    shoot_strip(window, out, "default-closing-step");

    // With Animations off, Settings is in place from the frame after it opens. The frame it
    // opens on still shows Home: the sheet comes in at the first frame's end, to animate from
    // its start when Animations are on.
    let still = std::fs::read_to_string(out.join("strips/default-settings-open-still.txt")).unwrap();
    let frames: Vec<&str> = still.lines().skip(1).map(|l| l.split(' ').nth(1).unwrap()).collect();
    assert!(frames.windows(2).all(|w| w[0] == w[1]), "with Animations off nothing moves:\n{still}");
    let moving = std::fs::read_to_string(out.join("strips/default-closing-step.txt")).unwrap();
    let frames: Vec<&str> = moving.lines().map(|l| l.split(' ').nth(1).unwrap()).collect();
    assert!(frames.windows(2).all(|w| w[0] != w[1]), "the closing screen's covers drift:\n{moving}");
}

/// The screens moving forward and back: the Library opening and going back to Home, a game's
/// page opening from the Library and going back, Downloads going back, a game opened while the
/// Library still comes in, and Back with Animations off.
fn shoot_screen_moves(window: &MinimalSoftwareWindow, out: &Path) {
    let mut host = covered(false);
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);
    shoot(window, &out.join("moves-home.png"));
    host.dispatch(Command::Navigate(Route::Library));
    host.load_images();
    shoot_strip(window, out, "move-library-forward");
    host.dispatch(Command::OpenGame("emu-a:G1".parse().unwrap()));
    host.load_images();
    shoot_strip(window, out, "move-game-forward");
    host.dispatch(Command::Back);
    host.load_images();
    shoot_strip(window, out, "move-game-back");
    host.dispatch(Command::Back);
    host.load_images();
    shoot_strip(window, out, "move-library-back");
    host.dispatch(Command::Navigate(Route::Downloads));
    host.load_images();
    shoot(window, &out.join("moves-downloads.png"));
    host.dispatch(Command::Back);
    host.load_images();
    shoot_strip(window, out, "move-downloads-back");
    // A game opened 80 ms after the Library: the Library goes out from where it got to.
    host.dispatch(Command::Navigate(Route::Library));
    host.load_images();
    pass(Duration::from_millis(80));
    slint::platform::update_timers_and_animations();
    host.dispatch(Command::OpenGame("emu-a:G2".parse().unwrap()));
    host.load_images();
    shoot_strip(window, out, "move-game-mid-library");
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);
    shoot(window, &out.join("moves-home.png"));
    // With Animations off, the screen it leaves is gone after the first frame.
    host.dispatch(Command::Pref(Pref::Animations, false.into()));
    host.dispatch(Command::Navigate(Route::Downloads));
    follow(&mut host);
    shoot(window, &out.join("moves-downloads.png"));
    host.dispatch(Command::Back);
    host.load_images();
    shoot_strip(window, out, "move-downloads-back-still");
    let still = std::fs::read_to_string(out.join("strips/move-downloads-back-still.txt")).unwrap();
    let frames: Vec<&str> = still.lines().skip(1).map(|l| l.split(' ').nth(1).unwrap()).collect();
    assert!(frames.windows(2).all(|w| w[0] == w[1]), "with Animations off nothing moves:\n{still}");
}

// Each part that builds its own library is its own test, so they render at the same time.
macro_rules! shots {
    ($($test:ident => $shoot:ident),* $(,)?) => {$(
        #[test]
        fn $test() {
            let (window, out) = offscreen();
            $shoot(&window, &out);
        }
    )*};
}

/// Settings → Backup with a backup and its menu open, as `classic-backup-menu.png`; and the
/// browser of "Restore from another folder" in each built-in theme: a folder, a backup in it, and
/// the dialog before a restore, as `<theme>-backup-browser[-backup|-confirm].png`, and with a
/// keyboard's hints as `<theme>-backup-browser-confirm-keys.png`. Over the library the other screens
/// show: the real KytyPS5 plugin names its saves; the home folder is a temporary one.
fn shoot_backup(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("Downloads/old-pc")).unwrap();
    let mut host = library(home.path());
    host.load_images();
    host.dispatch(Command::Backup(launcher_core::BackupCmd::BackUp { large: false }));
    host.dispatch(Command::Navigate(Route::Settings));
    // Up to the tabs, and along them to Backup, which asks for the list as it opens.
    key(&mut host, Key::UpArrow);
    for _ in 0..5 {
        key(&mut host, Key::RightArrow);
    }
    host.pump();
    // Down to the backup, and Return opens its menu.
    for _ in 0..4 {
        key(&mut host, Key::DownArrow);
    }
    key(&mut host, Key::Return);
    shoot(window, &out.join("classic-backup-menu.png"));
    key(&mut host, Key::Escape);
    // A backup on another computer's folder, and folders beside it.
    let downloads = home.path().join("Downloads");
    let made = std::fs::read_dir(home.path().join("Marah Launcher Backups")).unwrap().next().unwrap().unwrap().path();
    std::fs::copy(&made, downloads.join("old-pc").join(made.file_name().unwrap())).unwrap();
    for folder in ["photos", "Switch saves"] {
        std::fs::create_dir_all(downloads.join(folder)).unwrap();
    }
    for theme in [theme_classic::ID, theme_default::ID] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Backup(launcher_core::BackupCmd::Browse(Some(downloads.clone()))));
        follow(&mut host);
        shoot_strip(window, out, &format!("{theme}-backup-browser-open"));
        shoot(window, &out.join(format!("{theme}-backup-browser.png")));
        round_corners(&host, out, &format!("{theme}-backup-browser"), "browser-title");
        key(&mut host, Key::Return);
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-backup-browser-backup.png")));
        // △ on a pad.
        let triangle: slint::SharedString = "/".into();
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: triangle.clone() });
        host.instance().window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: triangle });
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-backup-browser-confirm.png")));
        round_corners(&host, out, &format!("{theme}-backup-browser-confirm"), "browser-confirm-title");
        // The same with a mouse and keyboard: the hints show the keys.
        let mode = host.ui.input_mode().unwrap();
        host.ui.set_input_mode("pointer");
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-backup-browser-confirm-keys.png")));
        host.ui.set_input_mode(&mode);
        key(&mut host, Key::Escape);
        host.dispatch(Command::Backup(launcher_core::BackupCmd::CloseBrowser));
    }
    not_blank(out, &["classic-backup-browser-confirm", "default-backup-browser-confirm"]);
}

shots! {
    the_backup_tab => shoot_backup,
    the_motion_strips => shoot_strips,
    the_screens_move_forward_and_back => shoot_screen_moves,
    this_pc_screens => shoot_this_pc,
    the_releases => shoot_releases,
    a_ps4_cover => shoot_ps4_cover,
    genres => shoot_genres,
    the_trailer => shoot_trailer,
    the_splash => shoot_splash,
    the_closing_screen => shoot_closing,
    console_names => shoot_console_names,
    the_keyboard => shoot_keyboard,
    the_pad_hints => shoot_pad_hints,
    the_system_controls => shoot_system,
    the_status_areas => shoot_status_areas,
    the_system_controls_on_a_desktop => shoot_desktop_system,
    a_game_s_own_settings => shoot_game_settings,
    send_ratings => shoot_send_ratings,
}

/// On a desktop, in each built-in theme: the top bar with every machine icon, as
/// `<theme>-desktop-status-bar.png`; Settings on its System tab, with the whole tab list, as
/// `<theme>-desktop-settings-tabs.png`; and Restart's dialog, with the focus on Cancel, as
/// `<theme>-desktop-restart-confirm.png`.
fn shoot_desktop_system(window: &MinimalSoftwareWindow, out: &Path) {
    use i_slint_backend_testing::ElementHandle;
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let press = |host: &mut Composed, label: &str| {
        let found = ElementHandle::find_by_accessible_label(host.instance(), label).next();
        found.unwrap_or_else(|| panic!("no {label}")).invoke_accessible_default_action();
        host.pump();
        follow(host);
    };
    for theme in [theme_classic::ID, theme_default::ID] {
        let system = launcher_testkit::FakeSystem::new();
        system.with(|m| {
            m.network = launcher_api::NetworkStatus {
                connection: launcher_api::Connection::Wifi,
                name: "Home".into(),
                signal: Some(80),
            };
            m.bluetooth_on = true;
            m.devices = vec![launcher_api::BluetoothDevice {
                address: "AA".into(),
                name: "DualSense".into(),
                paired: true,
                connected: true,
            }];
            m.level = launcher_api::AudioLevel { volume: 65, muted: false };
            m.battery = Some(launcher_api::Battery { percent: 64, charging: false });
        });
        let plugin = system.plugin();
        let mut host = composed_on_current_platform(|h| {
            h.place(launcher_core::system::Place::default())
                .plugin(plugin)
                .plugin(FakeCatalog::new("emu-a").system("PS5").games(["Astro Drift"]).build())
        });
        host.ui.start_with(Some(id(theme))).unwrap();
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        host.dispatch(Command::System(launcher_core::system::SystemCommand::Refresh));
        follow(&mut host);
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-desktop-status-bar.png")));
        host.dispatch(Command::Navigate(Route::Settings));
        follow(&mut host);
        press(&mut host, "settings-tab:System");
        shoot(window, &out.join(format!("{theme}-desktop-settings-tabs.png")));
        press(&mut host, "settings-tab:Power");
        press(&mut host, "system-restart");
        shoot(window, &out.join(format!("{theme}-desktop-restart-confirm.png")));
        round_corners(&host, out, &format!("{theme}-desktop-restart-confirm"), "power-confirm-title");
    }
}

/// The top bar's status icons in each built-in theme, one area in each state: the network is
/// ready, the sound still reads, and Bluetooth and the drives are disabled (no adapter, a long
/// reason), as `<theme>-status-areas.png`; then with the keys on Bluetooth's icon, which shows
/// why, as `<theme>-status-disabled-focused.png`; then on the drives' icon, last but one in the
/// bar, whose long reason moves in from the window's edge, as `<theme>-status-disabled-edge.png`.
fn shoot_status_areas(window: &MinimalSoftwareWindow, out: &Path) {
    use launcher_core::system::Place;
    use slint::platform::Key;
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    for theme in [theme_classic::ID, theme_default::ID] {
        let system = launcher_testkit::FakeSystem::new();
        system.with(|m| {
            m.network = launcher_api::NetworkStatus {
                connection: launcher_api::Connection::Wifi,
                name: "Home".into(),
                signal: Some(80),
            };
            m.unavailable = vec![
                ("bluetooth", "This PC has no Bluetooth adapter".into()),
                ("drives", "Drives can't be managed here: this system has no udisks service running".into()),
            ];
            // The sound's read doesn't end while the shots are taken.
            m.slow = vec![("audio.unavailable", Duration::from_secs(600))];
        });
        let place = Place { whole_screen: true, own_os: true, stick: false };
        let plugin = system.plugin();
        let mut host = composed_on_current_platform(|h| {
            h.place(place)
                .call_timeout(Duration::from_secs(900))
                .plugin(plugin)
                .plugin(FakeCatalog::new("emu-a").system("PS5").games(["Astro Drift"]).build())
        });
        host.ui.start_with(Some(id(theme))).unwrap();
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        // The machine's own tick reads it: a Refresh would wait for the sound's read.
        follow(&mut host);
        follow(&mut host);
        // The drives are read on their own tick.
        host.advance(Duration::from_secs(31));
        follow(&mut host);
        let vm = host.core().view().system;
        assert_eq!(
            [&vm.area("network").state, &vm.area("audio").state, &vm.area("bluetooth").state, &vm.area("drives").state],
            ["ready", "loading", "unsupported", "unsupported"],
            "{theme}"
        );
        shoot(window, &out.join(format!("{theme}-status-areas.png")));
        // From Settings, back past the drives' icon to Bluetooth's.
        for _ in 0..3 {
            key(&mut host, Key::Backtab);
        }
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-status-disabled-focused.png")));
        key(&mut host, Key::Tab);
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-status-disabled-edge.png")));
    }
}

/// Settings → Send ratings in each built-in theme: the ratings to pick from, then a report in
/// full, with the personal data out of its log, as `<theme>-send-ratings[-report].png`, and the
/// report scrolled to its end, as `<theme>-send-ratings-report-end.png`.
fn shoot_send_ratings(window: &MinimalSoftwareWindow, out: &Path) {
    use slint::platform::Key;
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    for theme in [theme_classic::ID, theme_default::ID] {
        let mut host = composed_on_current_platform(|h| {
            let mut emu = launcher_testkit::FakeEmulator::new(vec!["Paper Knights", "Quiet Orbit"]);
            emu.list = true;
            emu.build_sha = h.http().serve(&emu.build_url, vec![7u8; 20_000]);
            h.home(Path::new("/home/alice")).computer("alice-desktop").plugin(emu.plugin("emu-a"))
        });
        host.ui.start_with(Some(id(theme))).unwrap();
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        run_until(&host, 5000, |core| core.view().boot.ready);
        let logs = host.test.host.data_dir().join("logs").join("emu-a");
        std::fs::create_dir_all(&logs).unwrap();
        // A long log: the report scrolls.
        let log = (1..=60).map(|i| format!("[Render] frame {i}: a line of the emulator's log\n")).collect::<String>()
            + "[Loader] open /home/alice/Games/PS4/CUSA1/eboot.bin\n\
               [Net] alice@alice-desktop at 192.168.1.20\n[Render] vkCreateDevice failed\n";
        for (game, level) in [("G0", launcher_api::CompatLevel::Menus), ("G1", launcher_api::CompatLevel::Ingame)] {
            std::fs::write(logs.join(format!("{game}.log")), &log).unwrap();
            host.dispatch(Command::AskResult(format!("emu-a:{game}").parse().unwrap()));
            host.dispatch(Command::ReportResult(Some(level)));
        }
        host.dispatch(Command::Navigate(Route::Settings));
        host.dispatch(Command::Ratings(launcher_core::RatingsCmd::Open));
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-send-ratings.png")));
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::DownArrow);
        key(&mut host, Key::Return);
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-send-ratings-report.png")));
        for _ in 0..20 {
            key(&mut host, Key::DownArrow);
        }
        follow(&mut host);
        shoot(window, &out.join(format!("{theme}-send-ratings-report-end.png")));
    }
    not_blank(out, &["classic-send-ratings", "default-send-ratings-report"]);
}

/// A KytyPS5 game's own settings in each built-in theme, as `<theme>[-rtl]-game-settings.png`.
fn shoot_game_settings(window: &MinimalSoftwareWindow, out: &Path) {
    window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    let games = tempfile::tempdir().unwrap();
    let sce = games.path().join("PS5/Astro/sce_sys");
    std::fs::create_dir_all(&sce).unwrap();
    let param = serde_json::json!({
        "titleId": "PPSA01234", "contentVersion": "01.000.000",
        "localizedParameters": { "defaultLanguage": "en-US", "en-US": { "titleName": "Astro Bot" } },
    });
    std::fs::write(sce.join("param.json"), param.to_string()).unwrap();
    let mut host = composed_on_current_platform(|h| h.built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin)));
    host.ui.start().unwrap();
    host.dispatch(Command::SetFolders { name: "games".into(), paths: vec![games.path().into()] });
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    host.dispatch(Command::Navigate(Route::GameSettings("emu-kyty:PPSA01234".parse().unwrap())));
    assert_eq!(host.core().view().screen, launcher_ui_api::ScreenId::GameSettings);
    for theme in [theme_classic::ID, theme_default::ID] {
        host.ui.switch(&id(theme)).unwrap();
        for (direction, name) in [("ltr", theme.to_string()), ("rtl", format!("{theme}-rtl"))] {
            host.dispatch(Command::SetThemeOption { key: "direction".into(), value: direction.into() });
            follow(&mut host);
            shoot(window, &out.join(format!("{name}-game-settings.png")));
        }
        host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "ltr".into() });
    }
    not_blank(out, &["classic-game-settings", "default-game-settings"]);
}

/// The library the default screens show: a home folder (for the packages folder downloads go
/// to), emulators on each console, the real KytyPS5 plugin and game catalog with the sample
/// catalogs, a game selected and a search typed.
fn library(home: &Path) -> Composed {
    let mut host = composed_on_current_platform(|h| {
        h.home(home)
            .hardware(this_pc((1, 3)))
            .computer("gaming-pc")
            .discover(&fixture_themes())
            .plugin(
                FakeCatalog::new("emu-a")
                    .system("PS5")
                    .games(["Astro Drift", "Deep Lantern", "Ember Peaks", "Neon Harbor"])
                    .build(),
            )
            .plugin(FakeCatalog::new("emu-b").system("PS4").games(["Paper Knights", "Quiet Orbit"]).build())
            // The real KytyPS5 plugin, for its Settings section and Emulators block.
            .built_in(launcher_api::PluginBox::code(emu_kyty::KytyPlugin))
            .plugin(
                FakeCatalog::new("emu-x").named("Emu").playable().needing_its_emulator().games(["Starfall"]).build(),
            )
            .plugin(FakeCatalog::new("emu-p").playable().games(["Lumen Gate"]).build())
            .plugin(FakeCatalog::new("emu-pc").system("PC").games([]).build().console("PC"))
            // The game catalog, for the games it offers.
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    host.ui.start().unwrap();
    // The sample catalogs, fetched from the catalog address.
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/services/release-catalog/samples");
    for console in ["ps5", "ps4", "pc"] {
        let file = std::fs::read(samples.join(format!("{console}.json"))).unwrap();
        host.test.http.serve(&format!("https://xyz.dev/catalog/{console}.json"), file);
    }
    serve_steam_art(&host);
    // KytyPS5's builds: none yet, so its block shows no notice.
    let platform = host.core().kernel().host().platform;
    let (repo, _) = emu_kyty::release_source(&emu_kyty::default_settings(platform), platform).unwrap();
    host.test.http.serve(&format!("https://api.github.com/repos/{repo}/releases?per_page=10"), "[]");
    // Its community list, which the Catalog looks its PS5 offers up in: empty.
    host.test.http.serve(emu_kyty::COMPATIBILITY, "{}");
    host.dispatch(Command::SetConfig {
        plugin: "release-catalog".parse().unwrap(),
        scope: launcher_api::ConfigScope::Emulator,
        value: serde_json::json!({ "catalog_url": "https://xyz.dev/catalog", "hide_without_images": false }),
    });
    host.catalogs_fetched();
    // Install emulators automatically off: Emu's first play asks first, and nothing downloads
    // behind the screens.
    host.dispatch(Command::Pref(launcher_core::Pref::AutoInstall, false.into()));
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Select("emu-a:G1".parse().unwrap()));
    host.dispatch(Command::Navigate(Route::Catalog));
    host.dispatch(Command::CatalogSearch("a".into()));
    host
}

/// Fails unless the card that holds the element labelled `inside` has round corners in the
/// picture `name`: its top-left corner shows what is outside the card, not the card's fill. The
/// card's edges are found from the element: left from its middle and up from it, the first dark
/// pixel is outside.
fn round_corners(host: &Composed, out: &Path, name: &str, inside: &str) {
    round_corners_of(host, out, name, inside, 0);
}

/// [`round_corners`] for the `nth` element labelled `inside`, counted from 0.
fn round_corners_of(host: &Composed, out: &Path, name: &str, inside: &str, nth: usize) {
    let png = image::open(out.join(format!("{name}.png"))).unwrap().to_rgba8();
    let found = i_slint_backend_testing::ElementHandle::find_by_accessible_label(host.instance(), inside).nth(nth);
    let element = found.unwrap_or_else(|| panic!("{name}: no {inside} number {nth}"));
    let (at, size) = (element.absolute_position(), element.size());
    let (x, y) = ((at.x as u32).saturating_sub(4), (at.y + size.height / 2.0) as u32);
    let dark = |p: &image::Rgba<u8>| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) < 45;
    let left = (0..x).rev().find(|&i| dark(png.get_pixel(i, y))).expect("the card's left edge") + 1;
    let top = (0..y).rev().find(|&j| dark(png.get_pixel(x, j))).expect("the card's top edge") + 1;
    let fill = png.get_pixel(left + 8, y);
    // One pixel in: outside even a small card's curve, still in a square card's fill.
    let corner = png.get_pixel(left + 1, top + 1);
    let outside = png.get_pixel(left.saturating_sub(3), top.saturating_sub(3));
    let near = |a: &image::Rgba<u8>, b: &image::Rgba<u8>| (0..3).all(|c| a[c].abs_diff(b[c]) <= 12);
    assert!(
        near(corner, outside) && !near(corner, fill),
        "{name}: the corner at ({}, {}) is {corner:?}, the card's fill {fill:?}, outside {outside:?}",
        left + 1,
        top + 1
    );
}

/// Fails unless each named picture in `out` has more than one colour.
fn not_blank(out: &Path, names: &[&str]) {
    for name in names {
        let png = image::open(out.join(format!("{name}.png"))).unwrap().to_rgba8();
        let first = png.get_pixel(0, 0);
        assert!(png.pixels().any(|p| p != first), "{name}: the screenshot is not blank");
    }
}

#[test]
fn every_classic_screen_renders_the_library() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    shoot_classic(&mut host, &window, &out, "classic");
    shoot_sizes(&mut host, &window, &out);
    shoot_steam_offers(&mut host, &window, &out);
    // A game runs; the PS button brought the launcher back in front of it.
    host.dispatch(Command::Session(SessionCmd::Launch("emu-p:G0".parse().unwrap())));
    // The game watcher's rounds bring the game to the front.
    run_until(&host, 5000, |core| core.game_in_front());
    host.dispatch(Command::Session(SessionCmd::ToggleFocus));
    // The theme selects the running game: carry that out, as the app does.
    slint::platform::update_timers_and_animations();
    host.pump();
    host.load_images();
    shoot(&window, &out.join("classic-running.png"));
    // The other stub emulators name no console, so no catalog is asked for them.
    assert_served(&host, &[]);
    println!("screenshots in {}", out.display());
    not_blank(&out, &["classic-home", "classic-library", "classic-detail", "classic-settings", "classic-running"]);
}

/// A first play asks before the emulator downloads: the prompt in each built-in theme, as
/// `<theme>-install-prompt.png`.
#[test]
fn the_install_prompt_renders_in_each_theme() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    for theme in [theme_classic::ID, theme_default::ID] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Session(SessionCmd::Launch("emu-x:G0".parse().unwrap())));
        follow(&mut host);
        shoot(&window, &out.join(format!("{theme}-install-prompt.png")));
        round_corners(&host, &out, &format!("{theme}-install-prompt"), "install-prompt");
        host.dispatch(Command::CancelInstall);
    }
}

/// A game with no cover shows its system's art and its name: in each built-in theme's Catalog,
/// as `<theme>-catalog-no-cover.png`.
#[test]
fn a_game_without_a_cover_shows_round_corners_in_each_theme() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    for theme in [theme_classic::ID, theme_default::ID] {
        host.ui.switch(&id(theme)).unwrap();
        host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
        host.dispatch(Command::Navigate(Route::Catalog));
        host.load_images();
        follow(&mut host);
        shoot(&window, &out.join(format!("{theme}-catalog-no-cover.png")));
        // The second: the first has the focus, and its ring around it in Classic.
        round_corners_of(&host, &out, &format!("{theme}-catalog-no-cover"), "cover-name", 1);
    }
}

#[test]
fn the_classic_screens_render_right_to_left() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    shoot_classic(&mut host, &window, &out, "classic-rtl");
    not_blank(&out, &["classic-rtl-home"]);
}

#[test]
fn another_theme_renders_the_same_library() {
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    host.ui.switch(&id("fixture-grid")).unwrap();
    host.load_images();
    shoot(&window, &out.join("fixture-grid-home.png"));
    not_blank(&out, &["fixture-grid-home"]);
}

/// The Default theme's Home, left to right and right to left, on the five built-in emulators
/// with their own icons and covers, as `default-all-games.png` (All games focused),
/// A service with a Firmware block as Lichii's looks while it downloads.
fn downloading_block() -> FakeService {
    use launcher_api::{BlockRow, Tone};
    let rows = vec![
        BlockRow {
            label: "Installed".into(),
            value: "Not from Marah Launcher".into(),
            hint: "Update now puts the set firmware release in its place".into(),
            ..BlockRow::default()
        },
        BlockRow {
            label: "Downloading the firmware".into(),
            value: "46%".into(),
            hint: "159.4 MB of 340.5 MB".into(),
            tone: Tone::Info,
            progress: Some(0.46),
            ..BlockRow::default()
        },
        BlockRow {
            label: "Firmware release".into(),
            value: "22.5.0".into(),
            hint: "The Switch version to install, with its keys. Type one like 22.5.0.".into(),
            action: Some("set-firmware-release".into()),
            input: true,
            ..BlockRow::default()
        },
        BlockRow {
            label: "Automatic updates".into(),
            hint: "Marah Launcher keeps the set release installed, checked every 6 hours. Nintendo's firmware: check that \
                       you may use it where you live."
                .into(),
            action: Some("auto".into()),
            switch: Some(true),
            ..BlockRow::default()
        },
    ];
    FakeService::new("firm", "Firm").block(launcher_api::SlotId::EmulatorPanel, "firmware", "Firmware", rows)
}

/// `default-home.png` and `default-rtl-home.png`.
#[test]
fn the_default_theme_renders_its_list_both_ways() {
    use launcher_api::PluginBox;
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = composed_on_current_platform(|h| {
        h.home(home.path())
            .built_in(PluginBox::code(emu_kyty::KytyPlugin))
            .built_in(PluginBox::code(emu_shadps4::ShadPlugin))
            .built_in(PluginBox::code(emu_lichii::LichiiPlugin))
            .built_in(PluginBox::code(emu_pc::PcPlugin))
            .built_in(PluginBox::code(emu_steam::SteamPlugin))
            .plugin(downloading_block().build())
    });
    host.ui.start().unwrap();
    host.ui.switch(&id(theme_default::ID)).unwrap();
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    // The fake network has no emulator builds: their notices would cover the list.
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    host.load_images();
    shoot(&window, &out.join("default-all-games.png"));
    // The focus on the third entry, so the list shows a big tile between small ones.
    for _ in 0..2 {
        key(&mut host, slint::platform::Key::DownArrow);
    }
    shoot(&window, &out.join("default-home.png"));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    host.load_images();
    shoot(&window, &out.join("default-rtl-home.png"));
    // Lichii's page: its build, its firmware and its settings.
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "ltr".into() });
    host.dispatch(Command::Navigate(Route::Plugin("emu-lichii".parse().unwrap())));
    follow(&mut host);
    shoot(&window, &out.join("default-lichii.png"));
    host.first("settings-tab:Settings").invoke_accessible_default_action();
    follow(&mut host);
    shoot(&window, &out.join("default-lichii-settings.png"));
    host.dispatch(Command::Navigate(Route::Plugin("firm".parse().unwrap())));
    follow(&mut host);
    shoot(&window, &out.join("default-block-downloading.png"));
    // Settings, one page: its top, R1 twice on to Updates, then from right to left. Each shot
    // waits out the scroll's animation.
    let still = |host: &mut Composed| {
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            follow(host);
        }
    };
    host.dispatch(Command::Navigate(Route::Settings));
    still(&mut host);
    shoot(&window, &out.join("default-settings.png"));
    key(&mut host, slint::platform::Key::Tab);
    key(&mut host, slint::platform::Key::Tab);
    still(&mut host);
    shoot(&window, &out.join("default-settings-updates.png"));
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    still(&mut host);
    shoot(&window, &out.join("default-rtl-settings.png"));
    println!("screenshots in {}", out.display());
    not_blank(
        &out,
        &[
            "default-all-games",
            "default-home",
            "default-rtl-home",
            "default-lichii",
            "default-lichii-settings",
            "default-block-downloading",
        ],
    );
}

/// The Default theme's Library on All games and on PS5, with its list of sorts, the Catalog,
/// and the Library right to left, as `default-library[-ps5|-sort|-rtl].png` and
/// `default-catalog.png`.
#[test]
fn the_default_theme_renders_its_library() {
    use slint::platform::Key;
    let (window, out) = offscreen();
    let home = tempfile::tempdir().unwrap();
    let mut host = library(home.path());
    host.ui.switch(&id(theme_default::ID)).unwrap();
    host.dispatch(Command::Notice(launcher_core::NoticeCmd::DismissAll));
    host.dispatch(Command::Navigate(Route::Home));
    host.load_images();
    // All games, at the top of the list.
    key(&mut host, Key::Return);
    follow(&mut host);
    shoot(&window, &out.join("default-library.png"));
    host.first("filter:PS5").invoke_accessible_default_action();
    host.pump();
    follow(&mut host);
    shoot(&window, &out.join("default-library-ps5.png"));
    host.first("library-sort").invoke_accessible_default_action();
    follow(&mut host);
    shoot(&window, &out.join("default-library-sort.png"));
    key(&mut host, Key::Escape);
    host.dispatch(Command::Navigate(Route::Catalog));
    host.load_images();
    follow(&mut host);
    shoot(&window, &out.join("default-catalog.png"));
    // The Catalog's genres, with a count each, in both directions.
    host.first("library-genre").invoke_accessible_default_action();
    follow(&mut host);
    shoot(&window, &out.join("default-catalog-genres.png"));
    key(&mut host, Key::Escape);
    shoot_default_files_sheet(&mut host, &window, &out);
    host.dispatch(Command::SetThemeOption { key: "direction".into(), value: "rtl".into() });
    follow(&mut host);
    host.first("library-genre").invoke_accessible_default_action();
    follow(&mut host);
    shoot(&window, &out.join("default-catalog-genres-rtl.png"));
    key(&mut host, Key::Escape);
    host.dispatch(Command::Navigate(Route::Home));
    follow(&mut host);
    shoot(&window, &out.join("default-library-rtl.png"));
    println!("screenshots in {}", out.display());
    not_blank(
        &out,
        &[
            "default-library",
            "default-library-ps5",
            "default-library-sort",
            "default-catalog",
            "default-catalog-genres",
            "default-files-sheet",
            "default-catalog-genres-rtl",
            "default-library-rtl",
        ],
    );
}

/// The boot screen on a run that isn't the first, while the start's install downloads: its
/// checklist with the download's percent, as `{classic,default}-boot-installing.png`.
#[test]
fn the_boot_screen_renders_the_start_s_installs() {
    let (window, out) = offscreen();
    for theme in ["classic", "default"] {
        let gate = std::cell::RefCell::new(None);
        let mut host = composed_on_current_platform(|h| {
            let mut emu = launcher_testkit::FakeEmulator::new(vec!["Astro Drift"]);
            emu.build_sha = h.http().serve(&emu.build_url, vec![7u8; 20_480]);
            emu.build_size = Some(20_480);
            *gate.borrow_mut() = Some(h.http().hold_after(&emu.build_url, 8_192));
            h.plugin(emu.plugin("emu-a"))
        });
        if theme == "default" {
            host.ui.start_with(Some(id(theme_default::ID))).unwrap();
        } else {
            host.ui.start().unwrap();
        }
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        run_until(&host, 2000, |core| core.view().boot.steps.iter().any(|s| s.detail == "Downloading · 40%"));
        host.load_images();
        assert_eq!(host.values("boot"), ["Installing emulators and updates…"]);
        shoot(&window, &out.join(format!("{theme}-boot-installing.png")));
        drop(host);
        drop(gate);
    }
}

/// A plugin that asks the firmware's question at the start, for the screenshot.
struct FirmwareAsker;

#[launcher_api::async_trait]
impl launcher_api::StartupWork for FirmwareAsker {
    async fn status(&self) -> Option<launcher_api::StartupStep> {
        None
    }

    async fn question(&self) -> Option<launcher_api::StartupQuestion> {
        Some(launcher_api::StartupQuestion {
            title: "Install Switch firmware 22.5.0?".into(),
            text: "Switch games need the firmware and its keys. Marah Launcher downloads firmware 22.5.0 from \
                   THZoria/NX_Firmware on GitHub, and the keys of the same version, into Eden's folder."
                .into(),
            yes: "Install".into(),
            no: "Not now".into(),
        })
    }
}

/// A plugin's question over the boot screen, as `{classic,default}-boot-question.png`.
#[test]
fn the_boot_screen_renders_a_plugin_s_question() {
    let (window, out) = offscreen();
    for theme in ["classic", "default"] {
        let mut host = composed_on_current_platform(|h| {
            h.plugin(
                launcher_testkit::StubPlugin::from_toml(
                    "id = \"asker\"\nkind = \"service\"\nname = \"Asker\"\nversion = \"1.0.0\"\n\
                     host_api = \"^0.4\"\ncapabilities = [\"startup\"]",
                )
                .provides(|_| {
                    Ok(launcher_api::Provided {
                        startup: Some(std::sync::Arc::new(FirmwareAsker)),
                        ..Default::default()
                    })
                }),
            )
        });
        if theme == "default" {
            host.ui.start_with(Some(id(theme_default::ID))).unwrap();
        } else {
            host.ui.start().unwrap();
        }
        host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
        run_until(&host, 2000, |core| core.view().question.is_some());
        host.load_images();
        assert_eq!(host.values("question"), ["Install Switch firmware 22.5.0?"], "{theme}");
        shoot(&window, &out.join(format!("{theme}-boot-question.png")));
        round_corners(&host, &out, &format!("{theme}-boot-question"), "question");
    }
    not_blank(&out, &["classic-boot-question", "default-boot-question"]);
}

/// Astro Drift on PS5 and PS4, and Quest on PS4, in the Catalog of `theme`, sorted by title.
fn astro_drift(theme: &str) -> Composed {
    // shadPS4 recommends less than KytyPS5: on a PC that meets both, the PS4 version is the best.
    let needs = "vulkan = \"1.3\"\n[requires.recommended]\ncpu_threads = 8\nmemory_gb = 7.0";
    let mut host = composed_on_current_platform(|h| {
        h.hardware(this_pc((1, 3)))
            .plugin(FakeCatalog::kyty().playable().requires(&format!("{needs}\ngpu_memory_gb = 7.5")).build())
            .plugin(
                FakeCatalog::new("emu-ps4")
                    .system("PS4")
                    .console("PS4")
                    .requires(&format!("{needs}\ngpu_memory_gb = 3.5"))
                    .games([])
                    .build(),
            )
            .built_in(launcher_api::PluginBox::code(release_catalog::ReleaseCatalog::default()))
    });
    if theme == "default" {
        host.ui.start_with(Some(id(theme_default::ID))).unwrap();
    } else {
        host.ui.start().unwrap();
    }
    let topic = |console: &str, id: u32, title_id: &str, title: &str| {
        serde_json::json!({ "id": id.to_string(), "title": format!("[{console}] {title} [EUR / v1.00]"), "size": "20 GB",
            "magnet": format!("magnet:?xt=urn:btih:{id:040x}"),
            "game_info": { "name": title, "title_id": title_id, "version": "1.00", "region": "EUR",
                "release_year": "2024", "publisher": "Sony" } })
    };
    let ps5 = serde_json::json!([topic("PS5", 1, "PPSA01234", "Astro Drift")]);
    let ps4 = serde_json::json!([topic("PS4", 2, "CUSA01234", "Astro Drift"), topic("PS4", 3, "CUSA00002", "Quest")]);
    host.serve_catalog(&[("ps5", ps5), ("ps4", ps4)]);
    host.until(launcher_core::Area::Hardware, |core| core.view().settings.hardware.known);
    host.dispatch(Command::Library(launcher_core::LibraryCmd::Refresh));
    host.dispatch(Command::Navigate(Route::Catalog));
    host.dispatch(Command::CatalogSort(launcher_core::Sort::Title));
    host.load_images();
    host
}

/// A game on PS5 and PS4 in each built-in theme: its one card in the Catalog's "All", with a
/// chip for each console, as `<theme>-catalog-versions.png`; its Game Hub with the keys on the
/// versions, the best for this PC first, as `<theme>-versions.png`. Right and Return open the
/// other version.
#[test]
fn a_game_on_two_consoles_is_one_card_and_its_hub_lists_the_versions_in_each_theme() {
    use slint::platform::Key;
    let (window, out) = offscreen();
    for theme in ["classic", "default"] {
        let mut host = astro_drift(theme);
        assert_eq!(host.values("card-console"), ["PS4", "PS5", "PS4"], "{theme}: Astro Drift's two, Quest's one");
        shoot(&window, &out.join(format!("{theme}-catalog-versions.png")));

        host.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA01234".parse().unwrap()));
        host.load_images();
        key(&mut host, Key::DownArrow);
        assert_eq!(
            host.values("version-PS4"),
            ["Best for this PC · Not tested · Should run well"],
            "{theme}: the best first"
        );
        assert_eq!(host.values("version-PS5"), ["Not tested · Should run well"], "{theme}");
        shoot(&window, &out.join(format!("{theme}-versions.png")));
        key(&mut host, Key::RightArrow);
        key(&mut host, Key::Return);
        assert_eq!(host.core().view().detail.map(|d| d.platform), Some("PS5".into()), "{theme}: the other version");
    }
    not_blank(&out, &["classic-catalog-versions", "classic-versions", "default-catalog-versions", "default-versions"]);
}

/// Hide on a game on two consoles, in each built-in theme: the Game Hub's Hide opens the game's
/// menu with the keys on "Hide the PS4 version", as `<theme>-hide-versions.png`; Down reaches
/// "Hide every version", and Return on the first hides the PS4 version alone. The card's eye
/// hides the game under "All", and one version under a console's chip.
#[test]
fn hide_on_a_game_on_two_consoles_asks_which_versions_in_each_theme() {
    use slint::platform::Key;
    let (window, out) = offscreen();
    for theme in ["classic", "default"] {
        let mut host = astro_drift(theme);
        let eyes = host.values("card-hide");
        assert_eq!(eyes, ["Hide every version of Astro Drift", "Hide Quest"], "{theme}: the card is the game");
        // Under a console's chip, the eye hides that console's version alone.
        let system =
            |id: Option<&str>| Command::CatalogSystem(id.map(|id| launcher_core::SystemChoice::System(id.into())));
        host.dispatch(system(Some("PS5")));
        host.load_images();
        assert_eq!(host.values("card-hide"), ["Hide Astro Drift"], "{theme}");
        host.first("card-hide").invoke_accessible_default_action();
        host.pump();
        host.dispatch(system(None));
        host.load_images();
        assert_eq!(host.values("card-console"), ["PS4", "PS4"], "{theme}: Astro Drift's PS4 version stays");
        // Show, on a version of the game, shows every version again.
        let show = launcher_core::LibraryCmd::Hide {
            game: "release-catalog:emu-ps4/CUSA01234".parse().unwrap(),
            hidden: false,
        };
        host.dispatch(Command::Library(show));
        host.dispatch(Command::OpenGame("release-catalog:emu-ps4/CUSA01234".parse().unwrap()));
        host.load_images();
        // The screen's change handler runs on the next pass, and closes the game's menu.
        slint::platform::update_timers_and_animations();
        // The pointer's way: the pill is outside the key order. Not `press_first`: its settle
        // moves the testing backend's clock, and this platform has its own.
        host.first("hub-hide").invoke_accessible_default_action();
        host.pump();
        let items = host.values("game-menu");
        assert!(items.contains(&"Hide every version".to_owned()), "{theme}: {items:?}");
        assert_eq!(host.focused("game-menu"), ["Hide the PS4 version"], "{theme}: the keys on the one version");
        shoot(&window, &out.join(format!("{theme}-hide-versions.png")));
        key(&mut host, Key::DownArrow);
        assert_eq!(host.focused("game-menu"), ["Hide every version"], "{theme}");
        key(&mut host, Key::UpArrow);
        key(&mut host, Key::Return);
        assert_eq!(host.core().view().detail.map(|d| d.hidden), Some(true), "{theme}: this version is hidden");
        host.dispatch(Command::Navigate(Route::Catalog));
        host.load_images();
        assert_eq!(host.values("card-console"), ["PS5", "PS4"], "{theme}: Astro Drift's PS5 alone, and Quest");
    }
    not_blank(&out, &["classic-hide-versions", "default-hide-versions"]);
}
