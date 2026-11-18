//! "Send ratings": the player's own results go to the community compatibility lists that take
//! reports through a form, such as shadPS4's. A result waits on this computer until the player
//! sends it. For each one the player picks, the launcher shows the whole report and where it
//! goes; on their yes it opens the list's form, filled in, in the browser, and the player
//! submits it there. The launcher holds no account and sends nothing itself.
//!
//! The end of the emulator's log goes with a result. It is kept when the player rates the
//! game, as the next run writes over the log, and personal data comes out of it first
//! ([`crate::redact`]).

use launcher_api::{CompatLevel, GameKey, ReportForm, ResultReport};
use launcher_ui_api::{RatingItemVm, RatingsVm, ReportPreviewVm};
use serde::{Deserialize, Serialize};

use crate::app::Core;
use crate::changes::Area;
use crate::redact::{Personal, redact};
use crate::{CoreError, CoreResult};

/// What the player does in "Send ratings".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RatingsCmd {
    /// Opens it, with every result that waits and whose list takes reports, all picked.
    Open,
    /// Picks a result to send, or leaves it out.
    Pick(GameKey),
    /// Shows the first picked report: what it says and where it goes.
    Next,
    /// The player says yes to the report on screen: its form opens in the browser, and the
    /// result is sent. Then the next picked report shows.
    Send,
    /// The player says no to the report on screen: nothing is sent. Then the next one shows.
    Skip,
    /// Closes "Send ratings". What wasn't sent waits.
    Close,
}

/// The most of a log a report carries, as it is in the form's link: GitHub refuses a link
/// much over 8 KB.
const LOG_BUDGET: usize = 3_000;
/// The most of the log file read: its end.
const LOG_READ: u64 = 64 * 1024;

/// "Send ratings" while it is open.
#[derive(Clone, Debug, Default)]
pub(crate) struct SendRatings {
    pub reports: Vec<Prepared>,
    /// The report on screen, by its place in `reports`; `None` while the player picks.
    pub shown: Option<usize>,
}

/// A result's report, made when "Send ratings" opens. What the player sees is what opens.
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub game: GameKey,
    pub level: CompatLevel,
    pub picked: bool,
    /// Its form opened this time.
    pub sent: bool,
    pub list: String,
    pub title: String,
    pub body: String,
    pub note: String,
    /// The form with the report in its query.
    pub url: String,
}

pub(crate) async fn handle(core: &Core, command: RatingsCmd) -> CoreResult<()> {
    match command {
        RatingsCmd::Open => open(core).await?,
        RatingsCmd::Pick(game) => {
            if let Some(r) = core.state().ratings.as_mut().and_then(|s| s.reports.iter_mut().find(|r| r.game == game)) {
                r.picked = !r.picked;
            }
        }
        RatingsCmd::Next => {
            if let Some(s) = core.state().ratings.as_mut() {
                s.shown = s.reports.iter().position(|r| r.picked && !r.sent);
            }
        }
        RatingsCmd::Send => send(core).await?,
        RatingsCmd::Skip => next(core),
        RatingsCmd::Close => core.state().ratings = None,
    }
    core.changed(Area::Library);
    Ok(())
}

/// What is personal on this computer, to take out of reports.
fn personal(core: &Core) -> Personal {
    Personal::new(core.kernel().host().home.as_deref(), &core.ports().hardware.computer_name())
}

/// The player rated `game` just now: keeps the end of its last run's log, without personal
/// data, with the result, unsent.
pub(crate) fn queue(core: &Core, game: &GameKey) -> CoreResult<()> {
    let log = std::fs::File::open(crate::sessions::log_path(&core.kernel().host().data_dir, game))
        .ok()
        .map(|mut file| {
            use std::io::{Read, Seek, SeekFrom};
            let size = file.metadata().map(|m| m.len()).unwrap_or(0);
            let _ = file.seek(SeekFrom::Start(size.saturating_sub(LOG_READ)));
            let mut bytes = Vec::new();
            if let Err(e) = file.read_to_end(&mut bytes) {
                tracing::warn!(error = %e, "could not read the game's log for its result");
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
        .unwrap_or_default();
    let excerpt = excerpt(&redact(&log, &personal(core)));
    core.library().queue_report(game, &excerpt)
}

/// The last lines of `log` that fit in [`LOG_BUDGET`] once in a link. Blank lines go.
fn excerpt(log: &str) -> String {
    let mut used = 0;
    let mut lines: Vec<&str> = Vec::new();
    for line in log.lines().rev().map(str::trim_end).filter(|l| !l.trim().is_empty()) {
        used += url::form_urlencoded::byte_serialize(line.as_bytes()).map(str::len).sum::<usize>() + 3;
        if used > LOG_BUDGET {
            break;
        }
        lines.push(line);
    }
    lines.reverse();
    if lines.is_empty() {
        // The last line alone is too long: its end.
        let last = log.lines().rev().map(str::trim_end).find(|l| !l.trim().is_empty()).unwrap_or_default();
        return cut_to_fit(last);
    }
    lines.join("\n")
}

/// The end of `line` that fits in [`LOG_BUDGET`] once in a link.
fn cut_to_fit(line: &str) -> String {
    let mut used = 0;
    let mut start = line.len();
    for (at, c) in line.char_indices().rev() {
        used +=
            url::form_urlencoded::byte_serialize(c.encode_utf8(&mut [0; 4]).as_bytes()).map(str::len).sum::<usize>();
        if used > LOG_BUDGET {
            break;
        }
        start = at;
    }
    line[start..].to_owned()
}

/// The words of a result, as "How far did it get?" says them.
fn level_words(level: CompatLevel) -> &'static str {
    match level {
        CompatLevel::Ingame | CompatLevel::Playable => "In game",
        CompatLevel::Menus => "Main menu",
        CompatLevel::Boots => "Intro logos",
        CompatLevel::Nothing | CompatLevel::Unknown => "Doesn't start",
    }
}

/// Opens "Send ratings": a report for each result not sent yet, from its plugin. A result
/// whose plugin has no form isn't there.
async fn open(core: &Core) -> CoreResult<()> {
    let who = personal(core);
    let host = core.kernel().host();
    let specs = core.state().hardware.clone();
    let cpu = specs.as_ref().map(|s| s.cpu.clone()).unwrap_or_default();
    let gpu = specs
        .as_ref()
        .and_then(|s| s.gpus.as_ref())
        .and_then(|g| g.iter().find(|g| g.discrete).or(g.first()))
        .map(|g| g.name.clone())
        .unwrap_or_default();
    let memory =
        specs.as_ref().filter(|s| s.memory_gb > 0.0).map(|s| format!("{:.0} GB", s.memory_gb)).unwrap_or_default();
    let system = match host.platform {
        Some(launcher_api::Platform::Macos) => "macOS",
        Some(launcher_api::Platform::Windows) => "Windows",
        _ => "Linux",
    };
    let mut reports = Vec::new();
    // TODO: on Windows the app's opener runs `cmd /C start "" <link>`, and cmd splits a link at
    // each `&`, as a report's query has. Until the opener passes the link whole, nothing is
    // listed there.
    let rated =
        if host.platform == Some(launcher_api::Platform::Windows) { Vec::new() } else { core.library().rated() };
    for rated in rated.into_iter().filter(|r| !r.sent) {
        let plugin = &rated.game.plugin;
        let Ok(provided) = core.kernel().provided(plugin).await else { continue };
        let Some(compat) = provided.compatibility.clone() else { continue };
        let report = ResultReport {
            level: Some(rated.level),
            build: rated.build.clone().unwrap_or_default(),
            system: system.into(),
            cpu: cpu.clone(),
            gpu: gpu.clone(),
            memory: memory.clone(),
            log: rated.log.clone(),
        };
        let form = match core
            .kernel()
            .guard(plugin, "compatibility.report", compat.report(&rated.game.local, &report))
            .await
        {
            Ok(Some(form)) => form,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(error = %e, game = %rated.game, "no report form");
                continue;
            }
        };
        if !core.kernel().manifest(plugin)?.permissions.allows_link(&form.form) {
            tracing::warn!(game = %rated.game, form = %form.form, "the plugin's links don't allow its report form");
            continue;
        }
        let Some(prepared) = prepare(&rated.game, rated.level, &form, &who) else {
            tracing::warn!(game = %rated.game, form = %form.form, "the report form isn't a link");
            continue;
        };
        reports.push(prepared);
    }
    core.state().ratings = Some(SendRatings { reports, shown: None });
    Ok(())
}

/// The report the player sees and the link that opens, from the same text, with personal data
/// taken out of every field.
fn prepare(game: &GameKey, level: CompatLevel, form: &ReportForm, who: &Personal) -> Option<Prepared> {
    let mut url = url::Url::parse(&form.form).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let title = redact(&form.title, who);
    let mut body = Vec::new();
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("title", &title);
        for field in &form.fields {
            let value = redact(&field.value, who);
            if value.is_empty() {
                continue;
            }
            query.append_pair(&field.id, &value);
            body.push(if value.contains('\n') {
                format!("{}:\n{value}", field.label)
            } else {
                format!("{}: {value}", field.label)
            });
        }
    }
    Some(Prepared {
        game: game.clone(),
        level,
        picked: true,
        sent: false,
        list: form.list.clone(),
        title,
        body: body.join("\n\n"),
        note: form.note.clone(),
        url: url.into(),
    })
}

/// Opens the report on screen, marks it sent, and shows the next.
async fn send(core: &Core) -> CoreResult<()> {
    let shown = core.state().ratings.as_ref().and_then(|s| Some(s.reports.get(s.shown?)?.clone()));
    let Some(report) = shown else { return Ok(()) };
    let plugin = &report.game.plugin;
    if !core.kernel().manifest(plugin)?.permissions.allows_link(&report.url) {
        return Err(CoreError::PermissionDenied { plugin: plugin.clone(), action: format!("open {}", report.url) });
    }
    if let Err(e) = crate::running::open_link(core, report.url.clone()).await {
        core.notices().problem(format!("Couldn't open the report in your browser: {e}"));
        return Ok(());
    }
    core.library().mark_sent(&report.game)?;
    core.notices().info(format!(
        "The report for {} is open in your browser: submit it there",
        core.library().snapshot().title(&report.game)
    ));
    if let Some(r) = core.state().ratings.as_mut().and_then(|s| s.reports.iter_mut().find(|r| r.game == report.game)) {
        r.sent = true;
    }
    next(core);
    Ok(())
}

/// Shows the next picked report after the one on screen, or closes when there is none.
fn next(core: &Core) {
    let mut state = core.state();
    let Some(s) = state.ratings.as_mut() else { return };
    let from = s.shown.map_or(0, |i| i + 1);
    s.shown = s.reports.iter().enumerate().skip(from).find(|(_, r)| r.picked && !r.sent).map(|(i, _)| i);
    if s.shown.is_none() {
        state.ratings = None;
    }
}

/// "Send ratings" for the view.
pub(crate) fn view(ratings: Option<&SendRatings>, title: impl Fn(&GameKey) -> String) -> RatingsVm {
    let Some(s) = ratings else { return RatingsVm::default() };
    let items = s
        .reports
        .iter()
        .filter(|r| !r.sent)
        .map(|r| RatingItemVm {
            game: r.game.to_string(),
            title: title(&r.game),
            result: level_words(r.level).into(),
            list: r.list.clone(),
            picked: r.picked,
        })
        .collect();
    let preview = s.shown.and_then(|i| s.reports.get(i)).map(|r| {
        let picked: Vec<usize> = s.reports.iter().enumerate().filter(|(_, r)| r.picked).map(|(i, _)| i).collect();
        let place = picked.iter().position(|i| Some(*i) == s.shown).map_or(1, |p| p + 1);
        let destination = url::Url::parse(&r.url)
            .map(|u| format!("{}{}", u.host_str().unwrap_or_default(), u.path()))
            .unwrap_or_default();
        ReportPreviewVm {
            game: r.game.to_string(),
            game_title: title(&r.game),
            list: r.list.clone(),
            destination,
            title: r.title.clone(),
            body: r.body.clone(),
            note: r.note.clone(),
            step: format!("{place} of {}", picked.len()),
        }
    });
    RatingsVm { open: true, items, previewing: preview.is_some(), preview: preview.unwrap_or_default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_excerpt_is_the_end_of_the_log_and_fits_a_link() {
        let log = format!("{}\n\nthe error\n", "a line of the log\n".repeat(1_000));
        let e = excerpt(&log);
        assert!(e.ends_with("a line of the log\nthe error"), "the end, without blank lines");
        let encoded: usize = url::form_urlencoded::byte_serialize(e.as_bytes()).map(str::len).sum();
        assert!(encoded <= LOG_BUDGET, "{encoded} bytes in a link");
        assert_eq!(excerpt(""), "");
        let long = format!("start{}", "é".repeat(5_000));
        let cut = excerpt(&long);
        assert!(!cut.is_empty() && long.ends_with(&cut), "the end of a line too long for the link");
        assert!(url::form_urlencoded::byte_serialize(cut.as_bytes()).map(str::len).sum::<usize>() <= LOG_BUDGET);
    }

    #[test]
    fn the_link_holds_the_text_the_player_sees() {
        let form = ReportForm {
            list: "A list".into(),
            form: "https://github.com/o/r/issues/new?template=t.yml".into(),
            title: "G1 - A game".into(),
            fields: vec![
                launcher_api::ReportField { id: "status".into(), label: "Status".into(), value: "Menus".into() },
                launcher_api::ReportField { id: "empty".into(), label: "Empty".into(), value: String::new() },
                launcher_api::ReportField {
                    id: "log".into(),
                    label: "Log".into(),
                    value: "open /home/alice/a & b\nok".into(),
                },
            ],
            note: String::new(),
        };
        let who = Personal::new(Some(std::path::Path::new("/home/alice")), "");
        let p = prepare(&"emu:g".parse().unwrap(), CompatLevel::Menus, &form, &who).unwrap();
        assert_eq!(p.body, "Status: Menus\n\nLog:\nopen ~/a & b\nok");
        let url = url::Url::parse(&p.url).unwrap();
        let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let pairs =
            |list: &[(&str, &str)]| list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(
            query,
            pairs(&[("template", "t.yml"), ("title", "G1 - A game"), ("status", "Menus"), ("log", "open ~/a & b\nok")])
        );
        let http = ReportForm { form: "http://example.com/new".into(), ..form };
        assert!(prepare(&"emu:g".parse().unwrap(), CompatLevel::Menus, &http, &who).is_none(), "only https");
    }
}
