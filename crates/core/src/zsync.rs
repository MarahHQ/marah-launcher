//! A zsync download: build a new file from the blocks of an old one, and download only the
//! blocks the old one doesn't have. The release publishes a control file (`<file>.zsync`, made by
//! `zsyncmake`) beside the file. Its header says the file's length and its block size; then, for
//! each block, a weak checksum (rsync's rolling sum) and a strong one (MD4), both cut short.
//! [`Control::plan`] rolls the weak sum over every offset of the old file, checks each hit with
//! MD4, and lists the byte ranges no old block covers. The caller downloads those ranges and
//! [`Plan::assemble`] joins them with the old blocks.
//!
//! The control file's whole-file SHA-1 isn't checked here: the caller checks the result's SHA-256.

use std::collections::HashMap;

/// The biggest file a control file may describe, so a broken one can't ask for all the memory.
const MAX_LENGTH: u64 = 4 << 30;
/// The biggest block size `zsyncmake` makes is far below this.
const MAX_BLOCK: usize = 1 << 20;
/// Two missing ranges closer than this are asked for as one: fewer requests for a few more bytes.
const MERGE_GAP: u64 = 16 * 1024;
/// The bits of [`Control::plan`]'s quick test: 2 MB for a few blocks in each 1000 bits.
const SEEN_BITS: u32 = 24;

/// What a control file says.
#[derive(Debug)]
pub(crate) struct Control {
    /// The new file's length in bytes.
    pub length: u64,
    pub block_size: usize,
    /// How many blocks in a row must match: 2 lets each block's checksums be shorter.
    seq_matches: usize,
    /// How many bytes of each weak checksum the file keeps: its last ones.
    rsum_bytes: usize,
    /// How many bytes of each MD4 the file keeps: its first ones.
    checksum_bytes: usize,
    blocks: Vec<Block>,
}

#[derive(Debug)]
struct Block {
    /// The weak checksum, masked to the bytes the file keeps.
    rsum: u32,
    strong: [u8; 16],
}

/// rsync's rolling checksum of one block: `a` sums the bytes, `b` weighs each by its distance
/// from the block's end. Both wrap at 16 bits, as zsync's `unsigned short`s do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rsum {
    a: u16,
    b: u16,
}

impl Rsum {
    pub(crate) fn of(block: &[u8]) -> Self {
        let (mut a, mut b) = (0u16, 0u16);
        let len = block.len();
        for (i, &byte) in block.iter().enumerate() {
            a = a.wrapping_add(u16::from(byte));
            b = b.wrapping_add(((len - i) as u16).wrapping_mul(u16::from(byte)));
        }
        Rsum { a, b }
    }

    /// The sum of the window one byte on: `old` leaves it, `new` comes in.
    pub(crate) fn roll(&mut self, old: u8, new: u8, block_size: usize) {
        self.a = self.a.wrapping_add(u16::from(new)).wrapping_sub(u16::from(old));
        self.b = self.b.wrapping_add(self.a).wrapping_sub((block_size as u16).wrapping_mul(u16::from(old)));
    }

    /// The four bytes zsync stores, `a` then `b`, in network order, as one number.
    pub(crate) fn value(self) -> u32 {
        (u32::from(self.a) << 16) | u32::from(self.b)
    }
}

/// Which ranges of the new file to download, and where the old file has the rest.
#[derive(Debug)]
pub(crate) struct Plan {
    /// For each block, where the old file has it.
    found: Vec<Option<usize>>,
    /// Byte ranges to download, `start..=end`, in order.
    missing: Vec<(u64, u64)>,
}

impl Plan {
    pub(crate) fn missing(&self) -> &[(u64, u64)] {
        &self.missing
    }

    /// The bytes to download.
    pub(crate) fn missing_bytes(&self) -> u64 {
        self.missing.iter().map(|(a, b)| b + 1 - a).sum()
    }

    /// How many of the new file's blocks the old file has.
    pub(crate) fn reused(&self) -> usize {
        self.found.iter().filter(|f| f.is_some()).count()
    }

    /// The new file: each found block from `old`, each missing range from `fetched` (one piece
    /// per range of [`Plan::missing`], in order, each of that range's length).
    pub(crate) fn assemble(&self, control: &Control, old: &[u8], fetched: &[Vec<u8>]) -> Result<Vec<u8>, String> {
        if fetched.len() != self.missing.len() {
            return Err(format!("{} pieces for {} ranges", fetched.len(), self.missing.len()));
        }
        let length = control.length as usize;
        let mut out = vec![0u8; length];
        for (index, at) in self.found.iter().enumerate() {
            let Some(at) = *at else { continue };
            let start = index * control.block_size;
            let len = control.block_size.min(length - start);
            let from = old.get(at..at + len).ok_or("an old block is past the old file's end")?;
            out[start..start + len].copy_from_slice(from);
        }
        for (&(start, end), piece) in self.missing.iter().zip(fetched) {
            if piece.len() as u64 != end + 1 - start {
                return Err(format!("the piece {start}-{end} has {} bytes", piece.len()));
            }
            out[start as usize..=end as usize].copy_from_slice(piece);
        }
        Ok(out)
    }
}

impl Control {
    /// Reads a control file: its header lines, a blank line, then each block's checksums.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let end = bytes.windows(2).position(|w| w == b"\n\n").ok_or("no end to the header")?;
        let header = std::str::from_utf8(&bytes[..end]).map_err(|_| "the header isn't text")?;
        let mut fields = HashMap::new();
        for line in header.lines() {
            let (key, value) = line.split_once(':').ok_or_else(|| format!("a header line without a colon: {line}"))?;
            fields.insert(key.trim(), value.trim());
        }
        let field = |key: &str| fields.get(key).copied().ok_or_else(|| format!("no {key} in the header"));
        let number = |key: &str| field(key)?.parse::<u64>().map_err(|_| format!("{key} isn't a number"));
        field("zsync")?;
        // A control file for a compressed file names the blocks of the file inside it.
        if fields.contains_key("Z-Map2") || fields.contains_key("Recompress") {
            return Err("a control file for a compressed file".into());
        }
        let length = number("Length")?;
        let block_size = number("Blocksize")? as usize;
        if length == 0 || length > MAX_LENGTH {
            return Err(format!("a length of {length}"));
        }
        if !block_size.is_power_of_two() || block_size > MAX_BLOCK {
            return Err(format!("a block size of {block_size}"));
        }
        let lengths: Vec<usize> = field("Hash-Lengths")?
            .split(',')
            .map(|n| n.trim().parse().map_err(|_| "Hash-Lengths isn't three numbers"))
            .collect::<Result<_, _>>()?;
        let [seq_matches, rsum_bytes, checksum_bytes] = lengths[..] else {
            return Err("Hash-Lengths isn't three numbers".into());
        };
        if !(1..=2).contains(&seq_matches) || !(1..=4).contains(&rsum_bytes) || !(3..=16).contains(&checksum_bytes) {
            return Err(format!("Hash-Lengths of {seq_matches},{rsum_bytes},{checksum_bytes}"));
        }
        let count = length.div_ceil(block_size as u64) as usize;
        let sums = &bytes[end + 2..];
        let each = rsum_bytes + checksum_bytes;
        if sums.len() != count * each {
            return Err(format!("{} bytes of checksums for {count} blocks of {each}", sums.len()));
        }
        let blocks = sums
            .chunks_exact(each)
            .map(|sum| {
                let mut rsum = [0u8; 4];
                rsum[4 - rsum_bytes..].copy_from_slice(&sum[..rsum_bytes]);
                let mut strong = [0u8; 16];
                strong[..checksum_bytes].copy_from_slice(&sum[rsum_bytes..]);
                Block { rsum: u32::from_be_bytes(rsum), strong }
            })
            .collect();
        Ok(Control { length, block_size, seq_matches, rsum_bytes, checksum_bytes, blocks })
    }

    fn rsum_mask(&self) -> u32 {
        if self.rsum_bytes == 4 { u32::MAX } else { (1 << (8 * self.rsum_bytes)) - 1 }
    }

    /// Whether the window's MD4 starts with block `index`'s.
    fn strong_matches(&self, index: usize, window: &[u8]) -> bool {
        md4(window)[..self.checksum_bytes] == self.blocks[index].strong[..self.checksum_bytes]
    }

    /// The key a block is found by: its weak checksum, and with two matches in a row, the next
    /// block's too (none after the last block: zero, the sum of the zeros past a file's end).
    fn key(&self, first: u32, second: u32) -> u64 {
        if self.seq_matches == 1 { u64::from(first) } else { (u64::from(first) << 32) | u64::from(second) }
    }

    /// Finds the new file's blocks in `old`. `old` ends in `seq_matches` blocks of zeros: the
    /// last block is padded with zeros for its checksums, and so is a window past the old
    /// file's end ([`Control::padding`]).
    pub(crate) fn plan(&self, old: &[u8]) -> Plan {
        let bs = self.block_size;
        let count = self.blocks.len();
        let mask = self.rsum_mask();
        let mut by_key: HashMap<u64, Vec<usize>> = HashMap::new();
        for index in 0..count {
            let next = self.blocks.get(index + 1).map_or(0, |b| b.rsum);
            by_key.entry(self.key(self.blocks[index].rsum, next)).or_default().push(index);
        }
        // A bit for each key's hash: most offsets match no block, and this tells them apart
        // without a lookup in the map.
        let mut seen = vec![0u64; 1 << (SEEN_BITS - 6)];
        let bit = |key: u64| (key.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - SEEN_BITS)) as usize;
        for &key in by_key.keys() {
            seen[bit(key) >> 6] |= 1 << (bit(key) & 63);
        }
        let mut found: Vec<Option<usize>> = vec![None; count];
        // Both sums are kept rolling, the second only read with two matches in a row.
        let span = 2 * bs;
        let sums_at = |at: usize| (Rsum::of(&old[at..at + bs]), Rsum::of(&old[at + bs..at + 2 * bs]));
        let mut at = 0;
        if old.len() >= span {
            let (mut first, mut second) = sums_at(at);
            while at + span <= old.len() {
                let second_value = if self.seq_matches == 1 { 0 } else { second.value() & mask };
                let mut matched = false;
                let key = self.key(first.value() & mask, second_value);
                let maybe = seen[bit(key) >> 6] & (1 << (bit(key) & 63)) != 0;
                if let Some(candidates) = by_key.get(&key).filter(|_| maybe) {
                    for &index in candidates {
                        let next_ok = self.seq_matches == 1
                            || index + 1 == count
                            || self.strong_matches(index + 1, &old[at + bs..at + 2 * bs]);
                        if next_ok && self.strong_matches(index, &old[at..at + bs]) {
                            found[index].get_or_insert(at);
                            if self.seq_matches == 2 && index + 1 < count {
                                found[index + 1].get_or_insert(at + bs);
                            }
                            matched = true;
                        }
                    }
                }
                // A match: the next block likely follows it.
                let step = if matched { bs } else { 1 };
                if at + step + span > old.len() {
                    break;
                }
                if matched {
                    at += bs;
                    (first, second) = sums_at(at);
                } else {
                    first.roll(old[at], old[at + bs], bs);
                    second.roll(old[at + bs], old[at + 2 * bs], bs);
                    at += 1;
                }
            }
        }
        let mut missing: Vec<(u64, u64)> = Vec::new();
        for (index, _) in found.iter().enumerate().filter(|(_, f)| f.is_none()) {
            let start = (index * bs) as u64;
            let end = (start + bs as u64).min(self.length) - 1;
            match missing.last_mut() {
                Some(last) if start <= last.1 + 1 + MERGE_GAP => last.1 = end,
                _ => missing.push((start, end)),
            }
        }
        Plan { found, missing }
    }

    /// The zeros [`Control::plan`] wants after the old file's bytes.
    pub(crate) fn padding(&self) -> usize {
        self.block_size * 2
    }

    /// The blocks' count.
    pub(crate) fn blocks(&self) -> usize {
        self.blocks.len()
    }
}

/// MD4 (RFC 1320), which zsync's control files use for each block. Not for security: the
/// whole file's SHA-256 is checked after.
pub(crate) fn md4(data: &[u8]) -> [u8; 16] {
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut message = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_le_bytes());
    for chunk in message.as_chunks::<64>().0 {
        let mut x = [0u32; 16];
        for (word, bytes) in x.iter_mut().zip(chunk.as_chunks::<4>().0) {
            *word = u32::from_le_bytes(*bytes);
        }
        let [mut a, mut b, mut c, mut d] = state;
        let f = |x: u32, y: u32, z: u32| (x & y) | (!x & z);
        let g = |x: u32, y: u32, z: u32| (x & y) | (x & z) | (y & z);
        let h = |x: u32, y: u32, z: u32| x ^ y ^ z;
        for &i in &[0, 4, 8, 12] {
            a = a.wrapping_add(f(b, c, d)).wrapping_add(x[i]).rotate_left(3);
            d = d.wrapping_add(f(a, b, c)).wrapping_add(x[i + 1]).rotate_left(7);
            c = c.wrapping_add(f(d, a, b)).wrapping_add(x[i + 2]).rotate_left(11);
            b = b.wrapping_add(f(c, d, a)).wrapping_add(x[i + 3]).rotate_left(19);
        }
        for &i in &[0, 1, 2, 3] {
            let k = 0x5a82_7999u32;
            a = a.wrapping_add(g(b, c, d)).wrapping_add(x[i]).wrapping_add(k).rotate_left(3);
            d = d.wrapping_add(g(a, b, c)).wrapping_add(x[i + 4]).wrapping_add(k).rotate_left(5);
            c = c.wrapping_add(g(d, a, b)).wrapping_add(x[i + 8]).wrapping_add(k).rotate_left(9);
            b = b.wrapping_add(g(c, d, a)).wrapping_add(x[i + 12]).wrapping_add(k).rotate_left(13);
        }
        for &i in &[0, 2, 1, 3] {
            let k = 0x6ed9_eba1u32;
            a = a.wrapping_add(h(b, c, d)).wrapping_add(x[i]).wrapping_add(k).rotate_left(3);
            d = d.wrapping_add(h(a, b, c)).wrapping_add(x[i + 8]).wrapping_add(k).rotate_left(9);
            c = c.wrapping_add(h(d, a, b)).wrapping_add(x[i + 4]).wrapping_add(k).rotate_left(11);
            b = b.wrapping_add(h(c, d, a)).wrapping_add(x[i + 12]).wrapping_add(k).rotate_left(15);
        }
        state =
            [state[0].wrapping_add(a), state[1].wrapping_add(b), state[2].wrapping_add(c), state[3].wrapping_add(d)];
    }
    let mut out = [0u8; 16];
    for (bytes, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by `zsyncmake -b 1024` 0.6.2 (Ubuntu 22.04's, as the release build has) from
    /// [`xorshift`]`(100_000)`: Hash-Lengths 2,2,4.
    const UBUNTU: &[u8] = include_bytes!("../../../tests/fixtures/zsync/xorshift-100000.zsync");
    /// The same, by Fedora's `zsyncmake` 0.6.5: Hash-Lengths 1,3,6.
    const FEDORA: &[u8] = include_bytes!("../../../tests/fixtures/zsync/xorshift-100000-fedora.zsync");

    /// Bytes from a xorshift32 generator, the same the fixtures were made from.
    fn xorshift(len: usize) -> Vec<u8> {
        let mut x: u32 = 0x2545_f491;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn padded(control: &Control, bytes: &[u8]) -> Vec<u8> {
        let mut old = bytes.to_vec();
        old.resize(bytes.len() + control.padding(), 0);
        old
    }

    #[test]
    fn md4_gives_rfc_1320_s_digests() {
        for (text, digest) in [
            ("", "31d6cfe0d16ae931b73c59d7e0c089c0"),
            ("a", "bde52cb31de33e46245e05fbdbd6fb24"),
            ("abc", "a448017aaf21d8525fc10ae87aa6729d"),
            ("message digest", "d9130a8164549fe818874806e1c7014b"),
            ("abcdefghijklmnopqrstuvwxyz", "d79e1c308aa5bbcdeea8ed63df412da9"),
            ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", "043f8582f241db351ce627e153e7f0e4"),
            (
                "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
                "e33b4ddc9c38f2199c3e7b164fcc0536",
            ),
        ] {
            assert_eq!(hex::encode(md4(text.as_bytes())), digest, "{text:?}");
        }
    }

    #[test]
    fn the_header_is_read() {
        let control = Control::parse(UBUNTU).unwrap();
        assert_eq!((control.length, control.block_size), (100_000, 1024));
        assert_eq!((control.seq_matches, control.rsum_bytes, control.checksum_bytes), (2, 2, 4));
        assert_eq!(control.blocks(), 98, "the last block is a short one");
        let control = Control::parse(FEDORA).unwrap();
        assert_eq!((control.seq_matches, control.rsum_bytes, control.checksum_bytes), (1, 3, 6));
    }

    /// Each block's sums, as this module makes them, are the ones `zsyncmake` wrote: the last
    /// block padded with zeros.
    #[test]
    fn the_checksums_are_zsyncmake_s() {
        let data = xorshift(100_000);
        for fixture in [UBUNTU, FEDORA] {
            let control = Control::parse(fixture).unwrap();
            for (index, block) in control.blocks.iter().enumerate() {
                let mut bytes = data[index * 1024..data.len().min((index + 1) * 1024)].to_vec();
                bytes.resize(1024, 0);
                assert_eq!(Rsum::of(&bytes).value() & control.rsum_mask(), block.rsum, "block {index}");
                assert!(control.strong_matches(index, &bytes), "block {index}");
            }
        }
    }

    #[test]
    fn a_rolled_sum_is_the_window_s_own() {
        let data = xorshift(5000);
        let mut sum = Rsum::of(&data[..1024]);
        for at in 1..=3000 {
            sum.roll(data[at - 1], data[at - 1 + 1024], 1024);
            assert_eq!(sum, Rsum::of(&data[at..at + 1024]), "at {at}");
        }
    }

    /// The old file: the new one with bytes 10 and 11 of block 5, one byte of block 40 and
    /// one of block 80 changed, and 300 bytes put in after block 60, which shifts the rest.
    fn old_file(new: &[u8]) -> Vec<u8> {
        let mut old = new.to_vec();
        old[5 * 1024 + 10] ^= 0xff;
        old[5 * 1024 + 11] ^= 0xff;
        old[40 * 1024 + 7] ^= 0x01;
        old[80 * 1024 + 1000] ^= 0x80;
        old.splice(61 * 1024..61 * 1024, std::iter::repeat_n(7u8, 300));
        old
    }

    #[test]
    fn only_the_changed_blocks_are_missing_and_the_rest_comes_from_the_old_file() {
        let new = xorshift(100_000);
        for fixture in [UBUNTU, FEDORA] {
            let control = Control::parse(fixture).unwrap();
            let old = padded(&control, &old_file(&new));
            let plan = control.plan(&old);
            let block = |i: u64| (i * 1024, i * 1024 + 1023);
            assert_eq!(plan.missing(), [block(5), block(40), block(80)], "an insert costs nothing: the sum rolls");
            assert_eq!(plan.missing_bytes(), 3 * 1024);
            assert_eq!(plan.reused(), 95);
            let fetched: Vec<Vec<u8>> =
                plan.missing().iter().map(|&(a, b)| new[a as usize..=b as usize].to_vec()).collect();
            assert_eq!(plan.assemble(&control, &old, &fetched).unwrap(), new);
        }
    }

    #[test]
    fn the_short_last_block_is_found_at_the_old_file_s_end() {
        let new = xorshift(100_000);
        let control = Control::parse(UBUNTU).unwrap();
        let mut changed = new.clone();
        changed[0] ^= 1;
        let plan = control.plan(&padded(&control, &changed));
        assert_eq!(plan.missing(), [(0, 1023)]);
    }

    #[test]
    fn missing_blocks_close_together_are_one_range_and_the_last_ends_at_the_file_s_end() {
        let new = xorshift(100_000);
        let control = Control::parse(UBUNTU).unwrap();
        let mut changed = new.clone();
        for block in [10, 12, 97] {
            changed[block * 1024] ^= 1;
        }
        let plan = control.plan(&padded(&control, &changed));
        assert_eq!(plan.missing(), [(10 * 1024, 13 * 1024 - 1), (97 * 1024, 99_999)]);
    }

    #[test]
    fn an_unrelated_old_file_reuses_nothing() {
        let control = Control::parse(UBUNTU).unwrap();
        let plan = control.plan(&padded(&control, &[3u8; 50_000]));
        assert_eq!(plan.reused(), 0);
        assert_eq!(plan.missing(), [(0, 99_999)]);
    }

    #[test]
    fn a_broken_control_file_is_refused() {
        let header_end = UBUNTU.windows(2).position(|w| w == b"\n\n").unwrap();
        let header = std::str::from_utf8(&UBUNTU[..header_end]).unwrap();
        let sums = &UBUNTU[header_end..];
        let with = |from: &str, to: &str| [header.replace(from, to).as_bytes(), sums].concat();
        for (why, bytes) in [
            ("cut short", UBUNTU[..UBUNTU.len() - 1].to_vec()),
            ("no header end", header.as_bytes().to_vec()),
            ("not a control file", b"<html>Not Found</html>".to_vec()),
            ("bad lengths", with("Hash-Lengths: 2,2,4", "Hash-Lengths: 2,9,4")),
            ("two lengths", with("Hash-Lengths: 2,2,4", "Hash-Lengths: 2,2")),
            ("block size", with("Blocksize: 1024", "Blocksize: 1000")),
            ("huge", with("Length: 100000", "Length: 99999999999999")),
            ("no version", with("zsync: 0.6.2", "zsink: 0.6.2")),
            ("compressed", with("URL:", "Z-Map2: 1\nURL:")),
        ] {
            assert!(Control::parse(&bytes).is_err(), "{why}");
        }
    }
}
