//! Post-backup restore verification.
//!
//! After a successful backup, backmatic can pick a random sample of files from the (still mounted)
//! source, restore the same paths from the freshly written repository, and compare SHA-256 hashes.
//! A mismatch or restore failure marks the job failed (and pings the healthcheck as failed), giving
//! confidence that the repository actually contains recoverable data rather than silently corrupt
//! or empty snapshots.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A sampled source file together with its original content hash, used to check that the same file
/// restored from the backup repository matches byte-for-byte.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Path relative to the source root.
    pub rel_path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

/// Pick up to `count` regular files (respecting an optional `max_size` cap) from the tree rooted at
/// `root`, hashing each chosen file's current contents. Uses reservoir sampling over a recursive
/// walk so selection is spread across the whole tree without materializing the full file list.
///
/// Symlinks are not followed (we only sample real files) and unreadable entries are skipped. An
/// empty result simply means there was nothing to verify.
pub fn sample_files(root: &Path, count: u32, max_size: Option<u64>) -> Vec<Sample> {
    if count == 0 {
        return Vec::new();
    }
    let mut rng = Rng::seeded();
    let mut reservoir: Vec<PathBuf> = Vec::with_capacity(count as usize);
    let mut seen: u64 = 0;
    collect(root, max_size, &mut |file| {
        seen += 1;
        if reservoir.len() < count as usize {
            reservoir.push(file);
        } else {
            // Replace an existing entry with decreasing probability (classic reservoir sampling).
            let j = rng.below(seen);
            if (j as usize) < reservoir.len() {
                reservoir[j as usize] = file;
            }
        }
    });

    let mut samples = Vec::new();
    for path in reservoir {
        match hash_file(&path) {
            Ok((sha256, size)) => {
                let rel_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                samples.push(Sample {
                    rel_path,
                    sha256,
                    size,
                });
            }
            Err(e) => log::debug!("verify: skipping unhashable sample {}: {e}", path.display()),
        }
    }
    samples
}

/// Recursively visit regular files under `dir`, invoking `visit` for each file that passes the
/// optional size cap. Best-effort: directories and files that can't be read are skipped.
fn collect(dir: &Path, max_size: Option<u64>, visit: &mut impl FnMut(PathBuf)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Use symlink_metadata so we neither follow symlinks nor traverse into symlinked dirs.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            collect(&path, max_size, visit);
        } else if meta.is_file() {
            if let Some(cap) = max_size {
                if meta.len() > cap {
                    continue;
                }
            }
            visit(path);
        }
    }
}

/// Compare a restored file's bytes against a sample's original hash. Returns `Ok(())` on a match,
/// or a descriptive error string on mismatch.
pub fn compare(sample: &Sample, restored: &[u8]) -> Result<(), String> {
    let got = hash_bytes(restored);
    if got == sample.sha256 {
        Ok(())
    } else {
        Err(format!(
            "hash mismatch for '{}' (expected {}, restored {} bytes hashing to {})",
            sample.rel_path.display(),
            sample.sha256,
            restored.len(),
            got
        ))
    }
}

fn hash_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex(&hasher.finalize()), total))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Tiny xorshift64* PRNG. Avoids pulling in a dependency for the modest randomness reservoir
/// sampling needs.
struct Rng(u64);

impl Rng {
    fn seeded() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E3779B97F4A7C15)
            ^ (std::process::id() as u64).wrapping_mul(0x2545F4914F6CDD1D);
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Uniform-ish value in `[0, bound)`. `bound` must be > 0.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_bytes_is_stable_and_matches_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, b"hello world").unwrap();
        let (file_hash, size) = hash_file(&f).unwrap();
        assert_eq!(size, 11);
        assert_eq!(file_hash, hash_bytes(b"hello world"));
        // Known SHA-256 of "hello world".
        assert_eq!(
            file_hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn compare_detects_mismatch() {
        let sample = Sample {
            rel_path: PathBuf::from("a.txt"),
            sha256: hash_bytes(b"original"),
            size: 8,
        };
        assert!(compare(&sample, b"original").is_ok());
        let err = compare(&sample, b"tampered").unwrap_err();
        assert!(err.contains("hash mismatch"), "got: {err}");
    }

    #[test]
    fn sample_files_picks_requested_count_and_relpaths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("f{i}.txt")), format!("data-{i}")).unwrap();
        }
        std::fs::write(dir.path().join("sub/deep.txt"), b"deep").unwrap();

        let samples = sample_files(dir.path(), 3, None);
        assert_eq!(samples.len(), 3, "should pick exactly the requested count");
        for s in &samples {
            assert!(!s.rel_path.is_absolute(), "rel_path must be relative: {:?}", s.rel_path);
            let full = dir.path().join(&s.rel_path);
            let (h, _) = hash_file(&full).unwrap();
            assert_eq!(h, s.sha256, "recorded hash must match file contents");
        }
    }

    #[test]
    fn sample_files_respects_max_size() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("small.txt"), b"tiny").unwrap();
        std::fs::write(dir.path().join("big.txt"), vec![0u8; 5000]).unwrap();

        let samples = sample_files(dir.path(), 10, Some(100));
        assert_eq!(samples.len(), 1, "only the small file fits under the cap");
        assert_eq!(samples[0].rel_path, PathBuf::from("small.txt"));
    }

    #[test]
    fn sample_files_empty_when_no_files_or_zero_count() {
        let dir = tempfile::tempdir().unwrap();
        assert!(sample_files(dir.path(), 3, None).is_empty(), "no files -> empty");
        std::fs::write(dir.path().join("f.txt"), b"x").unwrap();
        assert!(sample_files(dir.path(), 0, None).is_empty(), "zero count -> empty");
    }
}
