//! The host's BitTorrent: [`launcher_core::Torrents`] on librqbit 9.1.0.
//!
//! One session, made at the first use, with no local peer discovery, at most 50 peers, and
//! uploads capped at [`SEED_UPLOAD_LIMIT`]. A torrent's metadata is kept as `<info hash>.torrent`
//! in the data folder's `torrents/`, so a start after a restart doesn't ask the swarm for it
//! again. The core saves the downloads; librqbit keeps only which pieces each one was checked
//! to have, in `torrents/resume/`, so a launch reads one piece of each file instead of every
//! byte (fastresume). Stopping a torrent forgets its pieces, and its next start checks it all.
//!
//! A route binds every socket of the session to one network interface
//! (`SessionOptions::bind_device_name`): peers over TCP and uTP, the listener, DHT, UDP
//! trackers and UPnP. HTTP trackers go through reqwest's `interface`. On Linux that is
//! SO_BINDTODEVICE, on macOS IP_BOUND_IF. A new route, and [`Torrents::stop_all`], stop the
//! session; the next start makes a new one.
//!
//! A download with files left out stores through [`SkipFiles`]: no file for a skipped one in
//! the download folder, and its bytes that a piece shares with a chosen file in
//! `torrents/kept/<info hash>/`, so the download can seed. [`Torrents::forget`] deletes them.
//!
//! A paused start adds the torrent paused, which checks the pieces on disk without
//! downloading. The adapter only reports what librqbit says ([`Torrents::stats`]): the core
//! decides what a finished or a paused torrent means, and unpauses or stops it.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use launcher_api::{PluginError, PluginResult};
use launcher_core::{SEED_UPLOAD_LIMIT, TorrentFile, TorrentInfo, TorrentStats, Torrents, TransferState};
use librqbit::storage::StorageFactoryExt;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions, TorrentStatsState,
};

use crate::torrent_storage::SkipFiles;

type ManagedTorrentHandle = Arc<ManagedTorrent>;

/// How long reading a torrent's files from the swarm may take.
const RESOLVE_LIMIT: Duration = Duration::from_secs(180);
/// How long a stopped session's torrents get to write down which pieces they have.
const FLUSH_TIME: Duration = Duration::from_millis(200);

struct Running {
    handle: ManagedTorrentHandle,
}

pub struct RqbitTorrents {
    dir: PathBuf,
    /// Extra session settings: tests run without DHT, trackers or the network.
    local: Option<Local>,
    session: Mutex<Option<Arc<Session>>>,
    /// Held while a session is made or stopped, so there is one at a time.
    making: tokio::sync::Mutex<()>,
    /// The interface every socket binds to; `None` for any.
    bind: Mutex<Option<String>>,
    running: Mutex<HashMap<String, Running>>,
}

/// A session that only talks to `peers` on this computer, for tests.
#[derive(Clone)]
pub struct Local {
    pub peers: Vec<std::net::SocketAddr>,
}

fn error(e: impl std::fmt::Display) -> PluginError {
    PluginError::Network(e.to_string())
}

impl RqbitTorrents {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            local: None,
            session: Default::default(),
            making: Default::default(),
            bind: Default::default(),
            running: Default::default(),
        }
    }

    #[cfg(test)]
    pub fn local(dir: PathBuf, peers: Vec<std::net::SocketAddr>) -> Self {
        Self { local: Some(Local { peers }), ..Self::new(dir) }
    }

    /// The session now, if there is one.
    fn current(&self) -> Option<Arc<Session>> {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    async fn session(&self) -> PluginResult<Arc<Session>> {
        let _making = self.making.lock().await;
        if let Some(session) = self.current() {
            return Ok(session);
        }
        // librqbit keeps the pieces each torrent was checked to have, by its info hash, so a
        // launch doesn't read every finished download again. Its list of torrents goes first:
        // the core starts the ones it wants.
        let resume = self.dir.join("resume");
        if let Err(e) = tokio::fs::remove_file(resume.join("session.json")).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "could not clear the torrent session's list");
        }
        let mut opts = SessionOptions {
            persistence: Some(librqbit::SessionPersistenceConfig::Json { folder: Some(resume) }),
            fastresume: true,
            disable_local_service_discovery: true,
            peer_limit: Some(50),
            concurrent_init_limit: Some(2),
            ratelimits: librqbit::limits::LimitsConfig {
                upload_bps: NonZeroU32::new(SEED_UPLOAD_LIMIT as u32),
                download_bps: None,
            },
            bind_device_name: self.bind.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            ..Default::default()
        };
        if self.local.is_some() {
            opts.dht = None;
            opts.disable_trackers = true;
            opts.listen = Some(librqbit::ListenerOptions {
                mode: librqbit::ListenerMode::TcpOnly,
                listen_addr: "127.0.0.1:0".parse().expect("an address"),
                enable_upnp_port_forwarding: false,
                ..Default::default()
            });
        }
        let session = Session::new_with_opts(self.dir.join("unused-default-output"), opts).await.map_err(error)?;
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = Some(session.clone());
        Ok(session)
    }

    fn metadata_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.torrent"))
    }

    /// Where a download with files left out keeps their bytes that a piece shares with a
    /// chosen file.
    fn kept_path(&self, id: &str) -> PathBuf {
        self.dir.join("kept").join(id)
    }

    fn peers(&self) -> Option<Vec<std::net::SocketAddr>> {
        self.local.as_ref().map(|l| l.peers.clone())
    }

    fn handle(&self, id: &str) -> Option<ManagedTorrentHandle> {
        self.running.lock().unwrap_or_else(|p| p.into_inner()).get(id).map(|r| r.handle.clone())
    }
}

#[async_trait]
impl Torrents for RqbitTorrents {
    async fn resolve(&self, magnet: &str) -> PluginResult<TorrentInfo> {
        let session = self.session().await?;
        let options = AddTorrentOptions { list_only: true, initial_peers: self.peers(), ..Default::default() };
        let answer =
            tokio::time::timeout(RESOLVE_LIMIT, session.add_torrent(AddTorrent::from_url(magnet), Some(options)))
                .await
                .map_err(|_| PluginError::Network("no peer answered in 3 minutes: try again later".into()))?
                .map_err(error)?;
        let AddTorrentResponse::ListOnly(listed) = answer else {
            return Err(PluginError::Other("the torrent engine didn't list the files".into()));
        };
        let id = listed.info_hash.as_string();
        if let Err(e) = tokio::fs::create_dir_all(&self.dir).await {
            tracing::warn!(error = %e, "could not make the torrents folder");
        }
        if let Err(e) = tokio::fs::write(self.metadata_path(&id), &listed.torrent_bytes).await {
            tracing::warn!(error = %e, "could not keep a torrent's metadata");
        }
        let files = listed
            .info
            .iter_file_details()
            .map(|f| TorrentFile { path: f.filename.to_vec().join("/"), size: f.len })
            .collect();
        let name = listed.info.info().name.as_ref().map(|n| String::from_utf8_lossy(n.as_ref()).into_owned());
        Ok(TorrentInfo { name: name.unwrap_or_default(), files })
    }

    async fn start(
        &self,
        id: &str,
        magnet: &str,
        folder: &Path,
        paused: bool,
        only: Option<&[usize]>,
    ) -> PluginResult<()> {
        let session = self.session().await?;
        let add = match tokio::fs::read(self.metadata_path(id)).await {
            Ok(bytes) => AddTorrent::from_bytes(bytes),
            Err(_) => AddTorrent::from_url(magnet),
        };
        let trackers = librqbit::Magnet::parse(magnet).map(|m| m.trackers).unwrap_or_default();
        let options = AddTorrentOptions {
            paused,
            overwrite: true,
            output_folder: Some(folder.to_string_lossy().into_owned()),
            only_files: only.map(<[usize]>::to_vec),
            // librqbit's own storage would make every file.
            storage_factory: only.map(|o| SkipFiles::new(o.iter().copied(), self.kept_path(id)).boxed()),
            trackers: Some(trackers),
            initial_peers: self.peers(),
            ..Default::default()
        };
        let handle = session
            .add_torrent(add, Some(options))
            .await
            .map_err(error)?
            .into_handle()
            .ok_or_else(|| PluginError::Other("the torrent engine gave no handle".into()))?;
        // A torrent the session already has: a resume.
        if !paused && matches!(handle.stats().state, TorrentStatsState::Paused) {
            session.unpause(&handle).await.map_err(error)?;
        }
        self.running.lock().unwrap_or_else(|p| p.into_inner()).insert(id.to_owned(), Running { handle });
        Ok(())
    }

    async fn unpause(&self, id: &str) -> PluginResult<()> {
        let (Some(handle), Some(session)) = (self.handle(id), self.current()) else { return Ok(()) };
        // Already live: librqbit can start a torrent on its own between the stats read and
        // this, and then says "already live", which is no fault.
        if !handle.is_paused() {
            return Ok(());
        }
        match session.unpause(&handle).await {
            Err(e) if !handle.is_paused() => {
                tracing::debug!(error = %e, "a torrent was already live");
                Ok(())
            }
            done => done.map_err(error),
        }
    }

    async fn stop(&self, id: &str) -> PluginResult<()> {
        let Some(running) = self.running.lock().unwrap_or_else(|p| p.into_inner()).remove(id) else { return Ok(()) };
        let Some(session) = self.current() else { return Ok(()) };
        if matches!(running.handle.stats().state, TorrentStatsState::Live | TorrentStatsState::Initializing { .. }) {
            let _ = session.pause(&running.handle).await;
        }
        // The files stay: only the session forgets it.
        session.delete(running.handle.id().into(), false).await.map_err(error)
    }

    async fn forget(&self, id: &str) -> PluginResult<()> {
        match tokio::fs::remove_dir_all(self.kept_path(id)).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(error(e)),
            _ => Ok(()),
        }
    }

    fn stats(&self, id: &str) -> Option<TorrentStats> {
        let stats = self.handle(id)?.stats();
        let live = stats.live.as_ref();
        let rate = |mbps: f64| (mbps * 1024.0 * 1024.0) as u64;
        Some(TorrentStats {
            state: match stats.state {
                TorrentStatsState::Initializing { .. } => TransferState::Checking,
                TorrentStatsState::Live => TransferState::Live,
                TorrentStatsState::Paused => TransferState::Paused,
                TorrentStatsState::Error => TransferState::Error,
            },
            finished: stats.finished,
            done: stats.progress_bytes,
            total: stats.total_bytes,
            download_rate: live.map(|l| rate(l.download_speed.mbps)).unwrap_or(0),
            upload_rate: live.map(|l| rate(l.upload_speed.mbps)).unwrap_or(0),
            peers: live.map(|l| l.snapshot.peer_stats.live).unwrap_or(0),
            error: stats.error.clone().unwrap_or_default(),
        })
    }

    async fn route(&self, interface: Option<String>) -> PluginResult<()> {
        {
            let mut bind = self.bind.lock().unwrap_or_else(|p| p.into_inner());
            if *bind == interface {
                return Ok(());
            }
            *bind = interface;
        }
        self.stop_all().await
    }

    async fn stop_all(&self) -> PluginResult<()> {
        let _making = self.making.lock().await;
        self.running.lock().unwrap_or_else(|p| p.into_inner()).clear();
        let session = self.session.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(session) = session {
            // It pauses every torrent and closes the listener, DHT and trackers. It waits a
            // second by itself.
            if tokio::time::timeout(Duration::from_secs(5), session.stop()).await.is_err() {
                tracing::warn!("the torrent session took more than 5 s to stop");
            }
            // Its torrents go with it, and each hands its pieces to a task that writes them
            // down: a moment for that before the caller goes on, maybe to quit.
            drop(session);
            tokio::time::sleep(FLUSH_TIME).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in, `MARAH_TORRENT_TESTS=1`: a real transfer between two librqbit sessions on this
    /// computer, with no DHT, trackers or other network. The payload is made-up bytes.
    #[test]
    fn a_torrent_downloads_finishes_and_seeds_again_only_when_its_files_are_whole() {
        if std::env::var_os("MARAH_TORRENT_TESTS").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("marah-launcher-torrent-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let mut x = 0x1234_5678u32;
            let payload: Vec<u8> = (0..512 * 1024)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect();
            let source = root.join("seed/fixture.bin");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(&source, &payload).unwrap();
            let seeder = Session::new_with_opts(
                root.join("seed"),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    persistence: None,
                    fastresume: false,
                    listen: Some(librqbit::ListenerOptions {
                        mode: librqbit::ListenerMode::TcpOnly,
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let (made, _served) = seeder
                .create_and_serve_torrent(
                    &source,
                    librqbit::CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(65536) },
                )
                .await
                .unwrap();
            let magnet = made.as_magnet().to_string();
            let id = made.info_hash().as_string();
            let torrents = RqbitTorrents::local(root.join("leech/torrents"), vec![seeder.listen_addr().unwrap()]);

            let info = torrents.resolve(&magnet).await.unwrap();
            assert_eq!(info.files, [TorrentFile { path: "fixture.bin".into(), size: payload.len() as u64 }]);

            let out = root.join("leech/out");
            async fn until(
                torrents: &RqbitTorrents,
                id: &str,
                what: &str,
                done: fn(&TorrentStats) -> bool,
            ) -> TorrentStats {
                for _ in 0..600 {
                    if let Some(stats) = torrents.stats(id).filter(done) {
                        return stats;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                panic!("never {what}: {:?}", torrents.stats(id));
            }
            let checked = |s: &TorrentStats| s.state != TransferState::Checking;
            torrents.start(&id, &magnet, &out, false, None).await.unwrap();
            until(&torrents, &id, "finished", |s| s.finished && s.state == TransferState::Live).await;
            assert_eq!(std::fs::read(out.join("fixture.bin")).unwrap(), payload);
            torrents.stop(&id).await.unwrap();

            // Again, paused: the files are whole, so its check finishes it, and it connects
            // only when the core unpauses it.
            torrents.start(&id, &magnet, &out, true, None).await.unwrap();
            let whole = until(&torrents, &id, "checked", checked).await;
            assert_eq!((whole.state, whole.finished), (TransferState::Paused, true));
            torrents.unpause(&id).await.unwrap();
            until(&torrents, &id, "live", |s| s.state == TransferState::Live).await;
            torrents.stop(&id).await.unwrap();

            // A file changed: the check finds a piece missing, and nothing downloads.
            let mut broken = payload.clone();
            broken[1000] ^= 0xff;
            std::fs::write(out.join("fixture.bin"), &broken).unwrap();
            torrents.start(&id, &magnet, &out, true, None).await.unwrap();
            let missing = until(&torrents, &id, "checked", checked).await;
            assert_eq!((missing.state, missing.finished), (TransferState::Paused, false));
            torrents.stop(&id).await.unwrap();
            assert_eq!(std::fs::read(out.join("fixture.bin")).unwrap(), broken, "nothing was downloaded");

            // Whole again, and the launcher closed: the next launch trusts the pieces it checked
            // before instead of reading every file again. It reads one piece of each file, so a
            // change in another piece shows only here, as a check that finds it whole.
            std::fs::write(out.join("fixture.bin"), &payload).unwrap();
            torrents.start(&id, &magnet, &out, true, None).await.unwrap();
            until(&torrents, &id, "checked", checked).await;
            torrents.stop_all().await.unwrap();
            let mut last_piece_changed = payload.clone();
            last_piece_changed[payload.len() - 1000] ^= 0xff;
            std::fs::write(out.join("fixture.bin"), &last_piece_changed).unwrap();
            let relaunched = RqbitTorrents::local(root.join("leech/torrents"), vec![seeder.listen_addr().unwrap()]);
            relaunched.start(&id, &magnet, &out, true, None).await.unwrap();
            let trusted = until(&relaunched, &id, "checked", checked).await;
            assert_eq!((trusted.state, trusted.finished), (TransferState::Paused, true), "not checked again");
            relaunched.stop_all().await.unwrap();
            let _ = std::fs::remove_dir_all(&root);
        });
    }
    /// Opt-in, `MARAH_TORRENT_TESTS=1`: a torrent of three files, the middle one skipped. No
    /// piece starts or ends on a file's edge, so the skipped file shares a piece with each of
    /// the others. librqbit downloads the chosen two and makes no file for the skipped one,
    /// after a check and a restart too. A third session that has only this one to download
    /// from gets the chosen files whole: every piece it was sent, the shared ones too, passed
    /// its hash.
    #[test]
    fn a_download_with_a_file_left_out_makes_no_file_for_it_and_seeds_the_others() {
        if std::env::var_os("MARAH_TORRENT_TESTS").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("marah-launcher-torrent-skip-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let source = root.join("seed/Game");
            std::fs::create_dir_all(&source).unwrap();
            let bytes = |len: u32, step: u32| (0..len).map(|i| (i * step % 251) as u8).collect::<Vec<u8>>();
            let named =
                [("game.bin", bytes(300_000, 7)), ("update.bin", bytes(200_000, 13)), ("dlc.bin", bytes(150_000, 17))];
            for (name, data) in &named {
                std::fs::write(source.join(name), data).unwrap();
            }
            let local = || SessionOptions {
                dht: None,
                disable_trackers: true,
                disable_local_service_discovery: true,
                persistence: None,
                fastresume: false,
                listen: Some(librqbit::ListenerOptions {
                    mode: librqbit::ListenerMode::TcpOnly,
                    listen_addr: "127.0.0.1:0".parse().unwrap(),
                    enable_upnp_port_forwarding: false,
                    ..Default::default()
                }),
                ..Default::default()
            };
            let seeder = Session::new_with_opts(root.join("seed"), local()).await.unwrap();
            let (made, _served) = seeder
                .create_and_serve_torrent(
                    &source,
                    librqbit::CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(65536) },
                )
                .await
                .unwrap();
            let magnet = made.as_magnet().to_string();
            let id = made.info_hash().as_string();
            let dir = root.join("leech/torrents");
            let torrents = RqbitTorrents::local(dir.clone(), vec![seeder.listen_addr().unwrap()]);
            let info = torrents.resolve(&magnet).await.unwrap();
            let data = |i: usize| &named.iter().find(|(n, _)| info.files[i].path.ends_with(n)).unwrap().1;
            let out = root.join("leech/out");
            let path = |i: usize| out.join(&info.files[i].path);

            async fn until(torrents: &RqbitTorrents, id: &str, what: &str, done: fn(&TorrentStats) -> bool) {
                for _ in 0..600 {
                    if torrents.stats(id).as_ref().is_some_and(done) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                panic!("never {what}: {:?}", torrents.stats(id));
            }
            let checked_whole = |s: &TorrentStats| s.state == TransferState::Paused && s.finished;

            torrents.start(&id, &magnet, &out, false, Some(&[0, 2])).await.unwrap();
            until(&torrents, &id, "finished", |s| s.finished && s.state == TransferState::Live).await;
            assert_eq!(&std::fs::read(path(0)).unwrap(), data(0));
            assert_eq!(&std::fs::read(path(2)).unwrap(), data(2));
            assert!(!path(1).exists(), "no file for the skipped one");
            let kept = std::fs::metadata(dir.join("kept").join(&id).join("1")).unwrap().len();
            assert!(kept > 0 && kept <= 2 * 65536, "it keeps its share of two pieces: {kept}");

            // The first seeder goes: the third session can download only from this one.
            seeder.stop().await;
            drop(seeder);
            let third = Session::new_with_opts(root.join("third"), local()).await.unwrap();
            let metadata = std::fs::read(dir.join(format!("{id}.torrent"))).unwrap();
            let options = AddTorrentOptions {
                only_files: Some(vec![0, 2]),
                output_folder: Some(root.join("third").to_string_lossy().into_owned()),
                initial_peers: Some(vec![torrents.current().unwrap().listen_addr().unwrap()]),
                ..Default::default()
            };
            let handle = third
                .add_torrent(AddTorrent::from_bytes(metadata), Some(options))
                .await
                .unwrap()
                .into_handle()
                .unwrap();
            for _ in 0..600 {
                if handle.stats().finished {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            assert!(handle.stats().finished, "{:?}", handle.stats());
            let third_path = |i: usize| root.join("third").join(&info.files[i].path);
            assert_eq!(&std::fs::read(third_path(0)).unwrap(), data(0), "seeded whole, with the shared pieces");
            assert_eq!(&std::fs::read(third_path(2)).unwrap(), data(2));
            third.stop().await;

            // Started again, it checks every piece: the chosen files are whole, and the skipped
            // one stays away.
            torrents.stop(&id).await.unwrap();
            torrents.start(&id, &magnet, &out, true, Some(&[0, 2])).await.unwrap();
            until(&torrents, &id, "checked whole", checked_whole).await;
            assert!(!path(1).exists(), "no file after a check");

            // After a restart, it trusts the pieces it checked before.
            torrents.stop_all().await.unwrap();
            let relaunched = RqbitTorrents::local(dir.clone(), vec![]);
            relaunched.start(&id, &magnet, &out, true, Some(&[0, 2])).await.unwrap();
            until(&relaunched, &id, "checked whole", checked_whole).await;
            assert!(!path(1).exists(), "no file after a restart");
            relaunched.stop(&id).await.unwrap();
            relaunched.forget(&id).await.unwrap();
            assert!(!dir.join("kept").join(&id).exists(), "forgotten, its kept bytes go");
            relaunched.stop_all().await.unwrap();
            let _ = std::fs::remove_dir_all(&root);
        });
    }

    /// Opt-in, `MARAH_TORRENT_TESTS=1`: a download stopped about half done, by a launcher that
    /// closes, goes on after the next launch from the pieces it has, and downloads only the
    /// rest. Once finished, it seeds after a launch without downloading anything.
    #[test]
    fn an_unfinished_download_goes_on_after_a_restart_from_the_pieces_it_has() {
        resumes_after_a_restart("whole", None);
    }

    /// The same for a download with the middle file left out: no file for it appears, and
    /// the pieces it shares with the chosen files count after the restart.
    #[test]
    fn an_unfinished_download_with_a_file_left_out_goes_on_after_a_restart() {
        resumes_after_a_restart("partial", Some(&[0, 2]));
    }

    /// A librqbit session on this computer that serves the contract's torrent, and the
    /// adapter's data folder, which outlives each adapter.
    struct Seeder {
        root: PathBuf,
        session: Arc<Session>,
    }

    #[async_trait]
    impl launcher_testkit::contract::TorrentFixture for Seeder {
        async fn torrent(&mut self, name: &str, files: &[(&str, Vec<u8>)]) -> (String, String) {
            let source = self.root.join("seed").join(name);
            for (path, bytes) in files {
                let to = source.join(path);
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::write(to, bytes).unwrap();
            }
            let (made, _served) = self
                .session
                .create_and_serve_torrent(
                    &source,
                    librqbit::CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(16 * 1024) },
                )
                .await
                .unwrap();
            (made.info_hash().as_string(), made.as_magnet().to_string())
        }

        fn adapter(&mut self) -> Arc<dyn Torrents> {
            let peers = vec![self.session.listen_addr().unwrap()];
            Arc::new(RqbitTorrents::local(self.root.join("leech/torrents"), peers))
        }

        fn send(&mut self, _id: &str) {}
    }

    /// Opt-in, `MARAH_TORRENT_TESTS=1`: the torrents contract, with a librqbit session on this
    /// computer as the swarm. No DHT, trackers or other network.
    #[test]
    fn the_engine_keeps_the_torrents_contract() {
        if std::env::var_os("MARAH_TORRENT_TESTS").is_none() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("marah-launcher-torrent-contract-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let session = Session::new_with_opts(
                root.join("seed"),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    persistence: None,
                    fastresume: false,
                    listen: Some(librqbit::ListenerOptions {
                        mode: librqbit::ListenerMode::TcpOnly,
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let mut seeder = Seeder { root: root.clone(), session: session.clone() };
            launcher_testkit::contract::torrents(&mut seeder).await;
            session.stop().await;
            let _ = std::fs::remove_dir_all(&root);
        });
    }

    async fn wait_for(
        torrents: &RqbitTorrents,
        id: &str,
        what: &str,
        done: impl Fn(&TorrentStats) -> bool,
    ) -> TorrentStats {
        for _ in 0..1200 {
            if let Some(stats) = torrents.stats(id).filter(|s| done(s)) {
                return stats;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("never {what}: {:?}", torrents.stats(id));
    }

    /// The bytes the torrent fetched from peers since it last went live.
    fn fetched(torrents: &RqbitTorrents, id: &str) -> u64 {
        torrents.handle(id).and_then(|h| h.live()).map(|l| l.stats_snapshot().fetched_bytes).unwrap_or(0)
    }

    fn resumes_after_a_restart(case: &str, only: Option<&[usize]>) {
        if std::env::var_os("MARAH_TORRENT_TESTS").is_none() {
            return;
        }
        const PIECE: u64 = 32 * 1024;
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("marah-launcher-torrent-resume-{case}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let source = root.join("seed/Game");
            std::fs::create_dir_all(&source).unwrap();
            let bytes = |len: u32, step: u32| (0..len).map(|i| (i.wrapping_mul(step) % 251) as u8).collect::<Vec<u8>>();
            // No file starts or ends on a piece's edge.
            let named = [
                ("game.bin", bytes(1_500_001, 7)),
                ("update.bin", bytes(1_000_003, 13)),
                ("dlc.bin", bytes(1_500_007, 17)),
            ];
            for (name, data) in &named {
                std::fs::write(source.join(name), data).unwrap();
            }
            // The seeder sends 1 MiB a second, so the download can stop about half done.
            let seeder = Session::new_with_opts(
                root.join("seed"),
                SessionOptions {
                    dht: None,
                    disable_trackers: true,
                    disable_local_service_discovery: true,
                    persistence: None,
                    fastresume: false,
                    ratelimits: librqbit::limits::LimitsConfig { upload_bps: NonZeroU32::new(1024 * 1024), download_bps: None },
                    listen: Some(librqbit::ListenerOptions {
                        mode: librqbit::ListenerMode::TcpOnly,
                        listen_addr: "127.0.0.1:0".parse().unwrap(),
                        enable_upnp_port_forwarding: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let (made, served) = seeder
                .create_and_serve_torrent(
                    &source,
                    librqbit::CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(PIECE as u32) },
                )
                .await
                .unwrap();
            let uploaded = || served.live().map(|l| l.stats_snapshot().uploaded_bytes).unwrap_or(0);
            let magnet = made.as_magnet().to_string();
            let id = made.info_hash().as_string();
            let peers = vec![seeder.listen_addr().unwrap()];
            let dir = root.join("leech/torrents");
            let out = root.join("leech/out");

            let torrents = RqbitTorrents::local(dir.clone(), peers.clone());
            let info = torrents.resolve(&magnet).await.unwrap();
            let data = |i: usize| &named.iter().find(|(n, _)| info.files[i].path.ends_with(n)).unwrap().1;
            let path = |i: usize| out.join(&info.files[i].path);
            let chosen = |i: usize| only.is_none_or(|o| o.contains(&i));
            torrents.start(&id, &magnet, &out, false, only).await.unwrap();
            let before = wait_for(&torrents, &id, "about half done", |s| s.total > 0 && s.done * 10 >= s.total * 4).await;
            assert!(!before.finished && before.done * 10 <= before.total * 8, "stopped too late: {before:?}");

            // The launcher closes.
            torrents.stop_all().await.unwrap();
            drop(torrents);
            let bits = std::fs::read(dir.join("resume").join(format!("{id}.bitv"))).unwrap();
            let pieces_kept = bits.iter().map(|b| u64::from(b.count_ones())).sum::<u64>();
            assert!(pieces_kept * PIECE >= before.done, "the pieces it had are written down: {pieces_kept} pieces, {before:?}");
            for i in (0..info.files.len()).filter(|&i| !chosen(i)) {
                assert!(!path(i).exists(), "no file for a skipped one");
            }

            // The next launch: the core adds it again; its check finds the pieces it had.
            let relaunched = RqbitTorrents::local(dir.clone(), peers.clone());
            let started = std::time::Instant::now();
            relaunched.start(&id, &magnet, &out, true, only).await.unwrap();
            let after = wait_for(&relaunched, &id, "checked", |s| s.state != TransferState::Checking).await;
            let check_time = started.elapsed();
            assert_eq!(after.state, TransferState::Paused);
            assert_eq!(after.total, before.total);
            assert!(after.done >= before.done, "progress lost: {} before, {} after", before.done, after.done);
            let uploaded_before = uploaded();
            relaunched.unpause(&id).await.unwrap();
            wait_for(&relaunched, &id, "finished", |s| s.finished && s.state == TransferState::Live).await;
            let fetched_after = fetched(&relaunched, &id);
            let sent_after = uploaded() - uploaded_before;
            let rest = after.total - after.done;
            eprintln!(
                "{case}: total {} B, done before the stop {} B ({}%), after the re-add {} B ({}%, checked in {check_time:?}), \
                 fetched after the restart {fetched_after} B (seeder sent {sent_after} B), the rest was {rest} B",
                after.total,
                before.done,
                before.done * 100 / before.total,
                after.done,
                after.done * 100 / after.total,
            );
            assert!(fetched_after >= rest, "it fetched what it lacked: {fetched_after} of {rest}");
            assert!(fetched_after <= rest + 4 * PIECE, "it fetched only the rest: {fetched_after}, the rest was {rest}");
            assert!(sent_after <= rest + 4 * PIECE, "the seeder sent only the rest: {sent_after}, the rest was {rest}");
            for i in 0..info.files.len() {
                if chosen(i) {
                    assert_eq!(&std::fs::read(path(i)).unwrap(), data(i), "file {i} whole");
                } else {
                    assert!(!path(i).exists(), "no file for a skipped one after the restart");
                    let kept = std::fs::metadata(dir.join("kept").join(&id).join(i.to_string())).unwrap().len();
                    assert!(kept > 0 && kept <= 2 * PIECE, "it keeps its share of two pieces: {kept}");
                }
            }

            // Finished and seeding, the launcher closes; the next launch seeds without
            // downloading.
            relaunched.stop_all().await.unwrap();
            drop(relaunched);
            let seeding = RqbitTorrents::local(dir.clone(), peers.clone());
            seeding.start(&id, &magnet, &out, true, only).await.unwrap();
            let checked = wait_for(&seeding, &id, "checked", |s| s.state != TransferState::Checking).await;
            assert_eq!((checked.state, checked.finished), (TransferState::Paused, true));
            seeding.unpause(&id).await.unwrap();
            wait_for(&seeding, &id, "live", |s| s.state == TransferState::Live).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert_eq!(fetched(&seeding, &id), 0, "a finished download fetches nothing");
            assert!(seeding.stats(&id).unwrap().finished);
            seeding.stop_all().await.unwrap();
            seeder.stop().await;
            let _ = std::fs::remove_dir_all(&root);
        });
    }
}
