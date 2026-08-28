//! OMV1 shared-memory video frame writer (contract: video/omv_shm.h).
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::MmapMut;

pub const MAGIC: u32 = 0x3156_4D4F;
pub const HDR_SIZE: usize = 4096;
pub const SLOT_HDR: usize = 4096;
pub const SLOTS: usize = 3;

fn align(n: usize, a: usize) -> usize { (n + a - 1) / a * a }

pub struct ShmWriter {
    /// When true, Drop must NOT unlink `path`: it belongs to a replacement writer.
    defused: bool,
    path: PathBuf,
    map: MmapMut,
    max_w: u32,
    max_h: u32,
    slot_stride: usize,
    slot: usize,
    seq: u64,
    slot_seq: [u32; SLOTS],
    generation: u32,
}

impl ShmWriter {
    pub fn create(name: &str, max_w: u32, max_h: u32) -> anyhow::Result<Self> {
        let dir = crate::paths::shm_dir();
        crate::paths::ensure_private_dir(&dir)?;
        let path = dir.join(format!("video-{name}.shm"));
        Self::create_at(path, max_w, max_h, 1)
    }

    fn create_at(path: PathBuf, max_w: u32, max_h: u32, generation: u32) -> anyhow::Result<Self> {
        let stride = align(max_w as usize * 4, 64);
        let slot_stride = SLOT_HDR + align(stride * max_h as usize, 4096);
        let size = HDR_SIZE + SLOTS * slot_stride;
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let file = OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&tmp)?;
        file.set_len(size as u64)?;
        let mut map = unsafe { MmapMut::map_mut(&file)? };
        map[..HDR_SIZE].fill(0);
        for i in 0..SLOTS {
            let off = HDR_SIZE + i * slot_stride;
            map[off..off + 64].fill(0);
        }
        let put32 = |m: &mut MmapMut, off: usize, v: u32| m[off..off + 4].copy_from_slice(&v.to_le_bytes());
        put32(&mut map, 0x04, 1);
        put32(&mut map, 0x08, HDR_SIZE as u32);
        put32(&mut map, 0x0C, SLOTS as u32);
        put32(&mut map, 0x10, slot_stride as u32);
        put32(&mut map, 0x14, max_w);
        put32(&mut map, 0x18, max_h);
        put32(&mut map, 0x1C, 1);
        put32(&mut map, 0x20, generation);
        map[0x28..0x30].copy_from_slice(&0u64.to_le_bytes());
        // magic last (release)
        let magic = unsafe { &*(map.as_ptr().add(0) as *const AtomicU32) };
        magic.store(MAGIC, Ordering::Release);
        std::fs::rename(&tmp, &path)?;
        Ok(ShmWriter { defused: false, path, map, max_w, max_h, slot_stride, slot: 0, seq: 0, slot_seq: [0; SLOTS], generation })
    }

    pub fn path(&self) -> &std::path::Path { &self.path }
    pub fn max_size(&self) -> (u32, u32) { (self.max_w, self.max_h) }

    /// Grow (new inode + generation) if a frame exceeds the current geometry.
    pub fn ensure_capacity(&mut self, w: u32, h: u32) -> anyhow::Result<bool> {
        if w <= self.max_w && h <= self.max_h { return Ok(false); }
        let nw = w.max(self.max_w).max(640);
        let nh = h.max(self.max_h).max(360);
        let path = self.path.clone();
        let gen = self.generation + 1;
        let new = Self::create_at(path, nw, nh, gen)?;
        // The replacement was renamed over our path; the old mapping must not unlink it.
        self.defused = true;
        *self = new;
        Ok(true)
    }

    /// Write one RGBA frame via a fill callback that receives (dst rows buffer, dst stride).
    pub fn write_with<F: FnOnce(&mut [u8], usize)>(&mut self, w: u32, h: u32, mirror: bool, fill: F) {
        if w == 0 || h == 0 || w > self.max_w || h > self.max_h { return; }
        let next = (self.slot + 1) % SLOTS;
        let base = HDR_SIZE + next * self.slot_stride;
        let stride = align(w as usize * 4, 64);
        // seq odd
        self.slot_seq[next] = self.slot_seq[next].wrapping_add(1);
        let seq_atomic = unsafe { &*(self.map.as_ptr().add(base) as *const AtomicU32) };
        seq_atomic.store(self.slot_seq[next], Ordering::Release);
        let now_us = monotonic_us();
        let m = &mut self.map;
        m[base + 4..base + 8].copy_from_slice(&w.to_le_bytes());
        m[base + 8..base + 12].copy_from_slice(&h.to_le_bytes());
        m[base + 12..base + 16].copy_from_slice(&(stride as u32).to_le_bytes());
        m[base + 16..base + 24].copy_from_slice(&now_us.to_le_bytes());
        m[base + 24..base + 32].copy_from_slice(&(self.seq + 1).to_le_bytes());
        m[base + 32..base + 36].copy_from_slice(&0u32.to_le_bytes());
        m[base + 36..base + 40].copy_from_slice(&(mirror as u32).to_le_bytes());
        let px = base + SLOT_HDR;
        fill(&mut m[px..px + stride * h as usize], stride);
        // seq even
        self.slot_seq[next] = self.slot_seq[next].wrapping_add(1);
        seq_atomic.store(self.slot_seq[next], Ordering::Release);
        self.seq += 1;
        self.slot = next;
        let latest = unsafe { &*(self.map.as_ptr().add(0x28) as *const AtomicU64) };
        latest.store((self.seq << 8) | next as u64, Ordering::Release);
    }
}

impl Drop for ShmWriter {
    fn drop(&mut self) {
        if !self.defused {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn monotonic_us() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

/// Remove stale files from a previous engine instance.
pub fn sweep() {
    let dir = crate::paths::shm_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
