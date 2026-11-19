//! The text the view models show: sizes, dates, durations and counts in words, as the player
//! reads them. The view builds every display string with these; themes show them as they are.

use launcher_ui_api::{AppView, GameCardVm, GameDetailVm, TorrentVm};

/// Writes the view's display text from the facts the view holds: each `*_text` field, and the
/// sizes, dates and times a theme shows in words. The view calls it once, when it is built.
pub(crate) fn write(view: &mut AppView) {
    let home = &mut view.home;
    let groups = home.groups.iter_mut().flat_map(|g| g.games.iter_mut());
    let cards = home.games.iter_mut().chain(groups).chain(&mut home.library.games).chain(&mut view.catalog.games.games);
    cards.for_each(card);
    if let Some(detail) = &mut view.detail {
        game(detail);
    }
    view.catalog.updated_text = date_text(&view.catalog.updated);
    for job in &mut view.downloads.jobs {
        job.progress_text = progress_text(job.received, job.total);
    }
    for t in &mut view.downloads.torrents {
        t.state_text = torrent_state_text(t);
        t.detail = torrent_detail(t);
        t.file_summary = file_summary(t);
    }
    let page = view.plugin_page.as_mut();
    let folders = view.settings.folders.iter_mut().chain(page.into_iter().flat_map(|p| p.folders.iter_mut()));
    for folder in folders {
        folder.requested_by_text = folder.requested_by.join(", ");
    }
    let page = view.plugin_page.as_mut().and_then(|p| p.emulator.as_mut());
    for emulator in view.emulators.emulators.iter_mut().chain(page) {
        emulator.controllers_text = emulator.controllers.join(", ");
    }
    if let Some(running) = &mut view.running {
        running.elapsed = duration_text(running.elapsed_secs);
    }
}

fn card(card: &mut GameCardVm) {
    (card.size, card.last_played, card.availability_text) =
        game_text(card.size_bytes, card.last_played_days, &card.availability);
}

fn game(d: &mut GameDetailVm) {
    (d.size, d.last_played, d.availability_text) = game_text(d.size_bytes, d.last_played_days, &d.availability);
    d.rating_text = rating_text(d.rating);
    d.release_text = date_text(&d.release);
    d.year = d.release.get(..4).unwrap_or_default().to_owned();
    d.played = played_text(d.played_secs);
    if let Some(compat) = &mut d.compat {
        compat.tested_text = date_text(&compat.tested);
    }
    for r in &mut d.releases {
        r.size = maybe_size_text(r.size_bytes);
        r.added_text = date_text(&r.added);
        r.counted_text = date_text(r.counted_at.get(..10).unwrap_or_default());
        r.availability_text = availability_text(&r.availability);
        r.download_text = download_text(&r.store, &r.download, r.download_progress);
        r.title_text = release_title(&r.version, &r.region, &r.system);
        r.peers_text = peers_text(r.seeders, r.leechers);
        // After `size`: the sheet shows it.
        r.sheet_text = sheet_text(&r.version, &r.region, &r.size, &d.emulator);
        // After `size` and `added_text`: the row shows them.
        r.line_text = release_line(&r.version_label, &r.size, &r.added_text);
        r.not_here_text = not_here_text(&r.not_here);
    }
}

/// The text a game's card and its page both show: its size, when it was last played, and
/// its availability, in that order.
fn game_text(size_bytes: Option<u64>, last_played_days: Option<u64>, availability: &str) -> (String, String, String) {
    (maybe_size_text(size_bytes), days_text(last_played_days), availability_text(availability))
}

/// The date of day `days` since 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A time in seconds since 1970 as UTC: year, month, day, hours, minutes and seconds.
pub(crate) fn utc(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let (y, m, d) = civil(secs / 86_400);
    let t = secs % 86_400;
    (y, m, d, t / 3600, t % 3600 / 60, t % 60)
}

/// A time in seconds since 1970 as `Nov 9, 2026, 14:30 UTC`: the core knows no time zone.
pub(crate) fn date_time_text(secs: u64) -> String {
    let (y, m, d, hh, mm, _) = utc(secs);
    format!("{}, {hh:02}:{mm:02} UTC", date_text(&format!("{y:04}-{m:02}-{d:02}")))
}

/// `path` with the home folder as `~`.
pub(crate) fn short_home(path: &str, home: Option<&str>) -> String {
    match home.and_then(|h| path.strip_prefix(h)) {
        Some(rest) if rest.is_empty() || rest.starts_with(std::path::MAIN_SEPARATOR) => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

/// `7.6 GB`, `820 MB`: decimal units, as the PS5 shows them.
pub(crate) fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// A whole disk's size: `1.9 GB` below 10 GB, and whole gigabytes from there, `512 GB`. It is
/// rounded, not cut: a "32 GB" stick holds a little less than 32 billion bytes and reads as 32.
pub(crate) fn disk_size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    // 9.95 GB and up would round to "10.0 GB": those read as whole gigabytes too.
    if gb < 9.95 { format!("{gb:.1} GB") } else { format!("{gb:.0} GB") }
}

/// [`size_text`], or empty when the size isn't known.
fn maybe_size_text(bytes: Option<u64>) -> String {
    bytes.map(size_text).unwrap_or_default()
}

/// `4 hr 6 min`, `12 min`, or `Less than a minute`.
fn duration_text(secs: u64) -> String {
    let (hours, minutes) = (secs / 3600, secs % 3600 / 60);
    match (hours, minutes) {
        (0, 0) => "Less than a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// The time played: [`duration_text`], or empty when the game never ran.
fn played_text(secs: u64) -> String {
    if secs > 0 { duration_text(secs) } else { String::new() }
}

/// `2 hr 5 min left`, `12 min left`, or `under a minute left`.
fn time_left_text(secs: u64) -> String {
    if secs < 60 { "under a minute left".into() } else { format!("{} left", duration_text(secs)) }
}

/// `Today`, `Yesterday` or `3 days ago`; empty when the game never ran.
fn days_text(days: Option<u64>) -> String {
    match days {
        None => String::new(),
        Some(0) => "Today".into(),
        Some(1) => "Yesterday".into(),
        Some(n) => format!("{n} days ago"),
    }
}

/// `2024-03-12` as `Mar 12, 2024`; empty stays empty.
fn date_text(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok());
    let month = date.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    match (date.get(..4), month, day) {
        (Some(y), Some(m), Some(d)) => format!("{} {d}, {y}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// An availability in words: `good` is "Good", `unknown` is "Unknown"; empty stays empty.
fn availability_text(level: &str) -> String {
    match level {
        "good" => "Good",
        "low" => "Low",
        "none" => "None",
        "unknown" => "Unknown",
        _ => "",
    }
    .into()
}

/// `4.4 · 3.2K ratings`, or empty for none.
fn rating_text(rating: Option<launcher_api::Rating>) -> String {
    let Some(r) = rating else { return String::new() };
    let noun = if r.total == 1 { "rating" } else { "ratings" };
    format!("{:.1} · {} {noun}", r.score, short_count(r.total))
}

/// `950`, `3.2K`, `10.6K`, `2M`: one decimal, none when it is 0.
fn short_count(n: u64) -> String {
    let (value, unit) = match n {
        0..1_000 => return n.to_string(),
        // Up to where one decimal would round to 1000.0K.
        1_000..999_950 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

/// `12.5 MB of 40.0 MB`, or `12.5 MB` when the size is unknown.
pub(crate) fn progress_text(received: u64, total: Option<u64>) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    match total {
        Some(t) => format!("{} of {}", mb(received), mb(t)),
        None => mb(received),
    }
}

/// `3.1 MB/s`.
fn rate_text(bytes: u64) -> String {
    format!("{:.1} MB/s", bytes as f64 / 1e6)
}

/// One line about a download: its sizes, speed and peers, by its state.
fn torrent_detail(t: &TorrentVm) -> String {
    let peers = if t.peers == 1 { "1 peer".to_owned() } else { format!("{} peers", t.peers) };
    match t.state.as_str() {
        "resolving" => "Reading the torrent's files from its peers; nothing downloads yet".into(),
        "ready" => "Look at its files below: nothing downloads until you press Download".into(),
        "checking" => "Checking the pieces already on disk".into(),
        "downloading" => {
            let left =
                if t.seconds_left > 0 { format!(" · {}", time_left_text(t.seconds_left)) } else { String::new() };
            format!("{} of {} · {} · {peers}{left}", size_text(t.done), size_text(t.total), rate_text(t.download_rate))
        }
        "paused" if t.waiting_vpn && t.file_count == 0 => {
            "It reads the torrent's files once your VPN is connected".into()
        }
        "paused" if t.waiting_vpn => {
            format!("{} of {} · it goes on once your VPN is connected", size_text(t.done), size_text(t.total))
        }
        "paused" | "cancelled" => format!("{} of {}", size_text(t.done), size_text(t.total)),
        "complete" if t.install == "installing" => {
            format!("Installing · {}% · it runs in the background", (t.install_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "unpacking" => {
            format!("Unpacking its archives · {}%", (t.unpack_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "cancelled" => "Unpacking stopped: Install unpacks the archives again".into(),
        "complete" if t.verifying => format!("{} · checking the files before sharing them", size_text(t.total)),
        "complete" if t.seeding => {
            format!("{} · sharing at {} · {peers}", size_text(t.total), rate_text(t.upload_rate))
        }
        "complete" => size_text(t.total),
        _ => String::new(),
    }
}

/// A download's state in words, for its card: the first that holds, in order, of its install,
/// its queue, its archives, then its state. A store's adds where: "Queued in Steam".
fn torrent_state_text(t: &TorrentVm) -> String {
    // An install that didn't start: "Download and install", or the install sheet.
    let not_started = t.state == "complete" && t.install.is_empty() && !t.install_error.is_empty();
    let words = match (t.install.as_str(), t.unpack.as_str(), t.state.as_str()) {
        ("installing", _, _) => "Installing",
        ("done", _, _) => "Installed",
        ("failed", _, _) => "Install failed",
        _ if not_started => "Install didn't start",
        (_, _, "queued") => "Queued",
        (_, "unpacking", _) => "Unpacking",
        (_, "failed", _) => "Unpacking failed",
        (_, _, "resolving") => "Reading its files",
        (_, _, "ready") if t.install_after => "Ready",
        (_, _, "ready") => "Ready: choose the files, then download",
        (_, _, "checking") => "Checking",
        (_, _, "downloading") => "Downloading",
        (_, _, "paused") if t.waiting_vpn => "Paused · waiting for your VPN",
        (_, _, "paused") => "Paused",
        (_, _, "complete") if t.seeding => "Complete · sharing",
        (_, _, "complete") if t.verifying => "Complete · checking",
        (_, _, "complete") => "Complete",
        (_, _, "failed") => "Failed",
        _ => "Cancelled · the files stay",
    };
    if t.store.is_empty() { words.to_owned() } else { format!("{words} in {}", t.store) }
}

/// What a release's Download button says, by its store, or its download's state and progress.
fn download_text(store: &str, download: &str, progress: f32) -> String {
    match download {
        _ if !store.is_empty() => format!("Install in {store}"),
        "" => "Download".into(),
        // As the kit worked it out: in f64, so a percent on a half rounds as it did.
        "downloading" => format!("Downloading {}%", (f64::from(progress) * 100.0).round() as u32),
        "complete" => "Downloaded".into(),
        "paused" => "Paused".into(),
        "failed" => "Failed".into(),
        _ => "In Downloads".into(),
    }
}

/// A release's title: "Version 1.02 · Europe · Linux", or "Release" when it has no version.
fn release_title(version: &str, region: &str, system: &str) -> String {
    let mut title = if version.is_empty() { "Release".to_owned() } else { format!("Version {version}") };
    for part in [region, system].into_iter().filter(|p| !p.is_empty()) {
        title = format!("{title} · {part}");
    }
    title
}

/// The catalog's counts: "5 seeders · 1 leecher". Empty when it has no count of seeders.
fn peers_text(seeders: Option<u64>, leechers: Option<u64>) -> String {
    let count = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (seeders, leechers) {
        (None, _) => String::new(),
        (Some(s), None) => count(s, "seeder", "seeders"),
        (Some(s), Some(l)) => format!("{} · {}", count(s, "seeder", "seeders"), count(l, "leecher", "leechers")),
    }
}

/// What the download sheet says of a release: "Version 1.02 · Europe · 7.6 GB · runs with
/// KytyPS5".
fn sheet_text(version: &str, region: &str, size: &str, emulator: &str) -> String {
    let version = if version.is_empty() { String::new() } else { format!("Version {version} · ") };
    let region = if region.is_empty() { String::new() } else { format!("{region} · ") };
    let size = if size.is_empty() { "Size unknown" } else { size };
    format!("{version}{region}{size} · runs with {emulator}")
}

/// A release row's second line: "Version 3 of 3 (newest) · 7.6 GB · added Oct 15, 2026".
fn release_line(version_label: &str, size: &str, added: &str) -> String {
    let place = if version_label.is_empty() { String::new() } else { format!("{version_label} · ") };
    let size = if size.is_empty() { "Size unknown" } else { size };
    let added = if added.is_empty() { String::new() } else { format!(" · added {added}") };
    format!("{place}{size}{added}")
}

/// What the download sheet says of a release this computer can't start: "This is a Linux
/// release: it can't run on Windows. It installs, but it won't start on this computer." Empty
/// when it can start it.
fn not_here_text(not_here: &str) -> String {
    if not_here.is_empty() {
        return String::new();
    }
    format!("{not_here}. It installs, but it won't start on this computer.")
}

/// What the file list shows: `120 files · 1.5 GB · first 100 shown`.
fn file_summary(t: &TorrentVm) -> String {
    let files = if t.file_count == 1 { "1 file".to_owned() } else { format!("{} files", t.file_count) };
    let shown = if t.files.len() < t.file_count { format!(" · first {} shown", t.files.len()) } else { String::new() };
    format!("{files} · {}{shown}", size_text(t.total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_availability_reads_in_words() {
        let words: Vec<String> = ["good", "low", "none", "unknown", ""].into_iter().map(availability_text).collect();
        assert_eq!(words, ["Good", "Low", "None", "Unknown", ""]);
    }

    #[test]
    fn progress_text_reads_naturally() {
        assert_eq!(progress_text(12_500_000, Some(40_000_000)), "12.5 MB of 40.0 MB");
        assert_eq!(progress_text(0, None), "0.0 MB");
    }

    #[test]
    fn a_date_reads_as_month_day_and_year() {
        assert_eq!(date_text("2024-03-12"), "Mar 12, 2024");
        assert_eq!(date_text(""), "");
    }

    #[test]
    fn a_rating_says_how_many_players_gave_it() {
        assert_eq!(rating_text(Some(launcher_api::Rating { score: 4.42, total: 3_200 })), "4.4 · 3.2K ratings");
        assert_eq!(rating_text(Some(launcher_api::Rating { score: 5.0, total: 1 })), "5.0 · 1 rating");
        assert_eq!(rating_text(None), "");
    }

    #[test]
    fn a_download_s_line_says_its_install_s_percent_while_it_installs() {
        let t = TorrentVm {
            state: "complete".into(),
            seeding: true,
            install: "installing".into(),
            install_progress: 0.43,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "Installing · 43% · it runs in the background");
    }

    #[test]
    fn a_download_s_state_reads_in_words() {
        let t = |state: &str| TorrentVm { state: state.into(), ..Default::default() };
        let rows: Vec<(&str, TorrentVm, &str)> = vec![
            ("installing", TorrentVm { install: "installing".into(), ..t("complete") }, "Installing"),
            ("installed", TorrentVm { install: "done".into(), ..t("complete") }, "Installed"),
            ("install failed", TorrentVm { install: "failed".into(), ..t("complete") }, "Install failed"),
            (
                "install didn't start",
                TorrentVm { install_error: "No room".into(), ..t("complete") },
                "Install didn't start",
            ),
            ("queued", t("queued"), "Queued"),
            ("unpacking", TorrentVm { unpack: "unpacking".into(), ..t("complete") }, "Unpacking"),
            ("unpacking failed", TorrentVm { unpack: "failed".into(), ..t("complete") }, "Unpacking failed"),
            ("resolving", t("resolving"), "Reading its files"),
            ("ready to install", TorrentVm { install_after: true, ..t("ready") }, "Ready"),
            ("ready", t("ready"), "Ready: choose the files, then download"),
            ("checking", t("checking"), "Checking"),
            ("downloading", t("downloading"), "Downloading"),
            ("waiting for a VPN", TorrentVm { waiting_vpn: true, ..t("paused") }, "Paused · waiting for your VPN"),
            ("paused", t("paused"), "Paused"),
            ("sharing", TorrentVm { seeding: true, ..t("complete") }, "Complete · sharing"),
            ("checking before sharing", TorrentVm { verifying: true, ..t("complete") }, "Complete · checking"),
            ("complete", t("complete"), "Complete"),
            ("failed", t("failed"), "Failed"),
            ("cancelled", t("cancelled"), "Cancelled · the files stay"),
            ("no state", t(""), "Cancelled · the files stay"),
            // Two rules hold: the first one wins.
            (
                "installing and unpacking",
                TorrentVm { install: "installing".into(), unpack: "unpacking".into(), ..t("complete") },
                "Installing",
            ),
            ("installed, download failed", TorrentVm { install: "done".into(), ..t("failed") }, "Installed"),
            ("queued and unpacking", TorrentVm { unpack: "unpacking".into(), ..t("queued") }, "Queued"),
            (
                "sharing and checking",
                TorrentVm { seeding: true, verifying: true, ..t("complete") },
                "Complete · sharing",
            ),
            ("install cancelled", TorrentVm { install: "cancelled".into(), ..t("complete") }, "Complete"),
            ("paused with an install error", TorrentVm { install_error: "No room".into(), ..t("paused") }, "Paused"),
            (
                "install didn't start, unpacking failed",
                TorrentVm { install_error: "No room".into(), unpack: "failed".into(), ..t("complete") },
                "Install didn't start",
            ),
            (
                "install cancelled with an install error",
                TorrentVm { install: "cancelled".into(), install_error: "No room".into(), ..t("complete") },
                "Complete",
            ),
            // A store's install says where it is.
            ("in a store", TorrentVm { store: "Steam".into(), ..t("queued") }, "Queued in Steam"),
            (
                "installing in a store",
                TorrentVm { store: "Steam".into(), install: "installing".into(), ..t("downloading") },
                "Installing in Steam",
            ),
        ];
        for (name, vm, words) in rows {
            assert_eq!(torrent_state_text(&vm), words, "{name}");
        }
    }

    #[test]
    fn a_release_s_download_button_says_its_download_s_state() {
        let rows = [
            ("in a store", ("Steam", "downloading", 0.5), "Install in Steam"),
            ("in a store, no download", ("Steam", "", 0.0), "Install in Steam"),
            ("no download", ("", "", 0.0), "Download"),
            ("downloading", ("", "downloading", 0.43), "Downloading 43%"),
            ("just started", ("", "downloading", 0.0), "Downloading 0%"),
            ("all there", ("", "downloading", 1.0), "Downloading 100%"),
            // Slint works out the percent in f64: 0.415 as f32 is just under 41.5%.
            ("on a half", ("", "downloading", 0.415), "Downloading 41%"),
            ("complete", ("", "complete", 1.0), "Downloaded"),
            ("paused", ("", "paused", 0.5), "Paused"),
            ("failed", ("", "failed", 0.5), "Failed"),
            ("resolving", ("", "resolving", 0.0), "In Downloads"),
            ("ready", ("", "ready", 0.0), "In Downloads"),
            ("cancelled", ("", "cancelled", 0.5), "In Downloads"),
        ];
        for (name, (store, download, progress), words) in rows {
            assert_eq!(download_text(store, download, progress), words, "{name}");
        }
    }

    #[test]
    fn a_release_s_title_says_its_version_region_and_system() {
        let rows = [
            ("all three", ("1.02", "Europe", "Linux"), "Version 1.02 · Europe · Linux"),
            ("no system", ("1.02", "Europe", ""), "Version 1.02 · Europe"),
            ("no region", ("1.02", "", "Linux"), "Version 1.02 · Linux"),
            ("a version only", ("1.02", "", ""), "Version 1.02"),
            ("no version", ("", "Europe", "Linux"), "Release · Europe · Linux"),
            ("a region only", ("", "Europe", ""), "Release · Europe"),
            ("a system only", ("", "", "Windows"), "Release · Windows"),
            ("nothing", ("", "", ""), "Release"),
        ];
        for (name, (version, region, system), words) in rows {
            assert_eq!(release_title(version, region, system), words, "{name}");
        }
    }

    #[test]
    fn a_release_s_peers_read_in_words() {
        let rows = [
            ("no count", (None, None), ""),
            // Leechers alone say nothing: the row shows the counts with the seeders only.
            ("leechers only", (None, Some(4)), ""),
            ("no seeders", (Some(0), None), "0 seeders"),
            ("one seeder", (Some(1), None), "1 seeder"),
            ("seeders", (Some(5), None), "5 seeders"),
            ("no leechers", (Some(5), Some(0)), "5 seeders · 0 leechers"),
            ("one leecher", (Some(5), Some(1)), "5 seeders · 1 leecher"),
            ("leechers", (Some(1), Some(2)), "1 seeder · 2 leechers"),
        ];
        for (name, (seeders, leechers), words) in rows {
            assert_eq!(peers_text(seeders, leechers), words, "{name}");
        }
    }

    #[test]
    fn a_release_s_download_sheet_says_what_it_gets() {
        let rows = [
            ("all of it", ("1.02", "Europe", "7.6 GB"), "Version 1.02 · Europe · 7.6 GB · runs with KytyPS5"),
            ("no version", ("", "Europe", "7.6 GB"), "Europe · 7.6 GB · runs with KytyPS5"),
            ("no region", ("1.02", "", "7.6 GB"), "Version 1.02 · 7.6 GB · runs with KytyPS5"),
            ("no size", ("1.02", "Europe", ""), "Version 1.02 · Europe · Size unknown · runs with KytyPS5"),
            ("a size only", ("", "", "7.6 GB"), "7.6 GB · runs with KytyPS5"),
            ("nothing", ("", "", ""), "Size unknown · runs with KytyPS5"),
        ];
        for (name, (version, region, size), words) in rows {
            assert_eq!(sheet_text(version, region, size, "KytyPS5"), words, "{name}");
        }
    }

    #[test]
    fn a_release_s_row_says_its_place_size_and_day() {
        let rows = [
            (
                "all of it",
                ("Version 3 of 3 (newest)", "7.6 GB", "Oct 15, 2026"),
                "Version 3 of 3 (newest) · 7.6 GB · added Oct 15, 2026",
            ),
            ("no place", ("", "7.6 GB", "Oct 15, 2026"), "7.6 GB · added Oct 15, 2026"),
            ("no size", ("Version 1 of 2", "", "Oct 15, 2026"), "Version 1 of 2 · Size unknown · added Oct 15, 2026"),
            ("no day", ("Version 1 of 2", "7.6 GB", ""), "Version 1 of 2 · 7.6 GB"),
            ("a size only", ("", "7.6 GB", ""), "7.6 GB"),
            ("nothing", ("", "", ""), "Size unknown"),
        ];
        for (name, (place, size, added), words) in rows {
            assert_eq!(release_line(place, size, added), words, "{name}");
        }
    }

    #[test]
    fn a_release_this_computer_can_t_start_says_so_on_its_sheet() {
        let rows = [
            ("it starts here", "", ""),
            (
                "it doesn't",
                "This is a Linux release: it can't run on Windows",
                "This is a Linux release: it can't run on Windows. It installs, but it won't start on this computer.",
            ),
        ];
        for (name, not_here, words) in rows {
            assert_eq!(not_here_text(not_here), words, "{name}");
        }
    }

    #[test]
    fn a_download_s_file_list_says_how_many_files_it_shows() {
        let files = vec!["a · 1 MB".to_owned(); 100];
        let t = TorrentVm { files, file_count: 120, total: 1_500_000_000, ..Default::default() };
        assert_eq!(file_summary(&t), "120 files · 1.5 GB · first 100 shown");
        let one = TorrentVm { files: vec!["a".into()], file_count: 1, total: 2_000_000, ..t };
        assert_eq!(file_summary(&one), "1 file · 2 MB");
    }

    #[test]
    fn a_download_s_line_says_its_time_left_only_while_bytes_arrive() {
        let t = TorrentVm {
            state: "downloading".into(),
            done: 1_000_000_000,
            total: 3_000_000_000,
            download_rate: 2_000_000,
            seconds_left: 7_500,
            peers: 3,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "1.0 GB of 3.0 GB · 2.0 MB/s · 3 peers · 2 hr 5 min left");
        assert!(torrent_detail(&TorrentVm { seconds_left: 30, ..t.clone() }).ends_with("under a minute left"));
        let stalled = TorrentVm { download_rate: 0, seconds_left: 0, ..t };
        assert_eq!(torrent_detail(&stalled), "1.0 GB of 3.0 GB · 0.0 MB/s · 3 peers");
    }
}
