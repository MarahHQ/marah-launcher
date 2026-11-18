//! The storage of a download with files left out. librqbit 9.1.0 makes every file of a torrent
//! when it adds one, so a seed of such a download would bring back the files the player
//! skipped. [`SkipFiles`] wraps librqbit's own storage: the chosen files go to the download
//! folder as before, and a skipped one gets no file there.
//!
//! A piece can hold the end of one file and the start of the next. When one of the two is
//! skipped, librqbit still downloads the whole piece, writes the skipped file's share of it,
//! and reads that back to check the piece's hash, at each check after a restart too. So each
//! skipped file keeps its share of its first and its last piece, at most two pieces, in
//! `<kept folder>/<file index>`, out of the download folder. Every other byte of a skipped file
//! reads as zeros and is never kept. Those pieces fail their hash, so librqbit never has them
//! and never sends them to a peer: it sends only pieces whose hash it checked.

use std::any::TypeId;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{IoSlice, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use librqbit::storage::filesystem::FilesystemStorageFactory;
use librqbit::storage::{BoxStorageFactory, StorageFactory, StorageFactoryExt, TorrentStorage};
use librqbit::{ManagedTorrentShared, TorrentMetadata};

/// Makes the storage of a torrent of which the player chose the files `chosen`, by their place
/// in it. What it keeps of the others goes into the folder `kept`.
#[derive(Clone)]
pub struct SkipFiles {
    chosen: Arc<BTreeSet<usize>>,
    kept: PathBuf,
}

impl SkipFiles {
    pub fn new(chosen: impl IntoIterator<Item = usize>, kept: PathBuf) -> Self {
        Self { chosen: Arc::new(chosen.into_iter().collect()), kept }
    }
}

impl StorageFactory for SkipFiles {
    type Storage = SkipFilesStorage;

    fn create(&self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<SkipFilesStorage> {
        let files = FilesystemStorageFactory::default().create(shared, metadata)?;
        Ok(SkipFilesStorage {
            files: Box::new(files),
            chosen: self.chosen.clone(),
            folder: self.kept.clone(),
            kept: Vec::new(),
        })
    }

    /// It stores on the file system, as librqbit's own does: librqbit saves the session's
    /// torrents only for that one.
    fn is_type_id(&self, type_id: TypeId) -> bool {
        type_id == TypeId::of::<Self>() || type_id == TypeId::of::<FilesystemStorageFactory>()
    }

    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

pub struct SkipFilesStorage {
    /// librqbit's own storage, for the chosen files.
    files: Box<dyn TorrentStorage>,
    chosen: Arc<BTreeSet<usize>>,
    folder: PathBuf,
    /// By file index: what a skipped file keeps; `None` for a chosen one.
    kept: Vec<Option<Kept>>,
}

impl SkipFilesStorage {
    fn kept(&self, file_id: usize) -> Option<&Kept> {
        self.kept.get(file_id).and_then(Option::as_ref)
    }
}

impl TorrentStorage for SkipFilesStorage {
    fn init(&mut self, shared: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<()> {
        // librqbit's storage makes no file for a padding file: to it, the skipped ones are.
        let mut file_infos = metadata.file_infos.clone();
        for (i, file) in file_infos.iter_mut().enumerate() {
            file.attrs.padding |= !self.chosen.contains(&i);
        }
        let only_chosen = TorrentMetadata {
            info: metadata.info.clone(),
            torrent_bytes: metadata.torrent_bytes.clone(),
            info_bytes: metadata.info_bytes.clone(),
            file_infos,
        };
        self.files.init(shared, &only_chosen)?;

        let piece = u64::from(metadata.lengths().default_piece_length());
        let mut kept = Vec::with_capacity(metadata.file_infos.len());
        for (i, file) in metadata.file_infos.iter().enumerate() {
            if self.chosen.contains(&i) || file.attrs.padding {
                kept.push(None);
                continue;
            }
            let regions = Regions::of(file.offset_in_torrent, file.len, piece);
            let store = if regions.size() > 0 {
                std::fs::create_dir_all(&self.folder)?;
                let path = self.folder.join(i.to_string());
                let store = File::options().create(true).truncate(false).read(true).write(true).open(&path)?;
                if store.metadata()?.len() != regions.size() {
                    store.set_len(regions.size())?;
                }
                Some(store)
            } else {
                None
            };
            kept.push(Some(Kept { regions, store: Mutex::new(store) }));
        }
        self.kept = kept;
        Ok(())
    }

    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        match self.kept(file_id) {
            Some(kept) => {
                kept.read(offset, buf);
                Ok(())
            }
            None => self.files.pread_exact(file_id, offset, buf),
        }
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        match self.kept(file_id) {
            Some(kept) => kept.write(offset, buf),
            None => self.files.pwrite_all(file_id, offset, buf),
        }
    }

    fn pwrite_all_vectored(&self, file_id: usize, offset: u64, bufs: [IoSlice<'_>; 2]) -> anyhow::Result<usize> {
        let Some(kept) = self.kept(file_id) else { return self.files.pwrite_all_vectored(file_id, offset, bufs) };
        kept.write(offset, &bufs[0])?;
        kept.write(offset + bufs[0].len() as u64, &bufs[1])?;
        Ok(bufs[0].len() + bufs[1].len())
    }

    fn remove_file(&self, file_id: usize, filename: &Path) -> anyhow::Result<()> {
        if self.kept(file_id).is_some() {
            return Ok(());
        }
        self.files.remove_file(file_id, filename)
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.files.remove_directory_if_empty(path)
    }

    fn ensure_file_length(&self, file_id: usize, length: u64) -> anyhow::Result<()> {
        if self.kept(file_id).is_some() {
            return Ok(());
        }
        self.files.ensure_file_length(file_id, length)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        let kept = self
            .kept
            .iter()
            .map(|k| {
                k.as_ref().map(|k| Kept {
                    regions: k.regions,
                    store: Mutex::new(k.store.lock().unwrap_or_else(|p| p.into_inner()).take()),
                })
            })
            .collect();
        Ok(Box::new(Self { files: self.files.take()?, chosen: self.chosen.clone(), folder: self.folder.clone(), kept }))
    }
}

/// The bytes of a skipped file that a piece shares with another file: its share of its first
/// piece (`head`) and of its last (`tail`). They are kept one after the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Regions {
    len: u64,
    head: u64,
    tail: u64,
}

impl Regions {
    /// For a file of `len` bytes at `offset` in a torrent of `piece`-byte pieces. A file that
    /// starts on a piece's start shares its first piece only when it ends inside it, and then
    /// its tail holds it all; the same goes for its end.
    fn of(offset: u64, len: u64, piece: u64) -> Self {
        let into_first = offset % piece;
        let head = if into_first == 0 { 0 } else { (piece - into_first).min(len) };
        let into_last = (offset + len) % piece;
        let tail = if into_last == 0 { 0 } else { into_last.min(len) };
        if head + tail >= len { Self { len, head: len, tail: 0 } } else { Self { len, head, tail } }
    }

    /// The bytes kept.
    fn size(&self) -> u64 {
        self.head + self.tail
    }

    /// Each part of `len` bytes at `offset` in the file that is kept: where it starts in the
    /// file, where in the store, and how long it is.
    fn parts(&self, offset: u64, len: u64) -> impl Iterator<Item = (u64, u64, u64)> {
        let end = offset + len;
        [(0, self.head, 0), (self.len - self.tail, self.len, self.head)].into_iter().filter_map(
            move |(from, to, at)| {
                let (start, stop) = (offset.max(from), end.min(to));
                (start < stop).then(|| (start, at + start - from, stop - start))
            },
        )
    }
}

/// What a skipped file keeps.
struct Kept {
    regions: Regions,
    /// `None` when it keeps nothing, or once the torrent pauses.
    store: Mutex<Option<File>>,
}

impl Kept {
    /// Zeros where nothing is kept, or the store doesn't read: the piece's hash then fails.
    fn read(&self, offset: u64, buf: &mut [u8]) {
        buf.fill(0);
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let Some(store) = store.as_mut() else { return };
        for (start, at, len) in self.regions.parts(offset, buf.len() as u64) {
            let from = (start - offset) as usize;
            let part = &mut buf[from..from + len as usize];
            let read = store.seek(SeekFrom::Start(at)).and_then(|_| read_up_to(store, part));
            if let Err(e) = read {
                tracing::debug!(error = %e, "a skipped file's kept bytes didn't read");
                part.fill(0);
            }
        }
    }

    /// Keeps the parts of `buf` that are kept, and drops the rest.
    fn write(&self, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let Some(store) = store.as_mut() else { return Ok(()) };
        for (start, at, len) in self.regions.parts(offset, buf.len() as u64) {
            let from = (start - offset) as usize;
            store.seek(SeekFrom::Start(at))?;
            store.write_all(&buf[from..from + len as usize])?;
        }
        Ok(())
    }
}

/// Reads into `buf` until it is full or the file ends; what is past the end stays as it was.
fn read_up_to(file: &mut File, mut buf: &mut [u8]) -> std::io::Result<()> {
    while !buf.is_empty() {
        match file.read(buf) {
            Ok(0) => break,
            Ok(n) => buf = &mut buf[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_file_keeps_only_its_share_of_the_pieces_it_shares() {
        // Pieces of 10 bytes. A file from 25 to 57: 5 bytes in piece 2, 7 in piece 5.
        assert_eq!(Regions::of(25, 32, 10), Regions { len: 32, head: 5, tail: 7 });
        // Starts and ends on a piece's edge: it shares none.
        assert_eq!(Regions::of(20, 30, 10).size(), 0);
        // Inside one piece, or across two: all of it.
        assert_eq!(Regions::of(21, 5, 10), Regions { len: 5, head: 5, tail: 0 });
        assert_eq!(Regions::of(25, 10, 10), Regions { len: 10, head: 10, tail: 0 });
        assert_eq!(Regions::of(20, 5, 10), Regions { len: 5, head: 5, tail: 0 });
        assert_eq!(Regions::of(0, 0, 10).size(), 0);
    }

    #[test]
    fn kept_bytes_read_back_and_the_rest_reads_as_zeros() {
        let path = std::env::temp_dir().join(format!("torrent-kept-bytes-test-{}", std::process::id()));
        let store = File::options().create(true).truncate(true).read(true).write(true).open(&path).unwrap();
        let regions = Regions::of(25, 32, 10);
        store.set_len(regions.size()).unwrap();
        let kept = Kept { regions, store: Mutex::new(Some(store)) };
        let bytes: Vec<u8> = (1..=32).collect();
        kept.write(0, &bytes).unwrap();

        let mut back = vec![0xff; 32];
        kept.read(0, &mut back);
        let mut expected = vec![0; 32];
        expected[..5].copy_from_slice(&bytes[..5]);
        expected[25..].copy_from_slice(&bytes[25..]);
        assert_eq!(back, expected, "the head and the tail, zeros between");
        let mut tail = [0xff; 4];
        kept.read(27, &mut tail);
        assert_eq!(tail, [28, 29, 30, 31]);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 12, "the store holds only those");

        *kept.store.lock().unwrap() = None;
        kept.read(0, &mut back);
        assert!(back.iter().all(|b| *b == 0), "a paused torrent's store reads as zeros");
        let _ = std::fs::remove_file(&path);
    }
}
