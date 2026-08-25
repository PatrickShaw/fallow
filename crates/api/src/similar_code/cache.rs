//! Bounded persistent cache for source-derived local embeddings.

use std::path::{Path, PathBuf};

use fallow_engine::source::similar_code::SimilarCodeSourceDigest;
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};

use super::protocol::{
    EXTRACTION_SEMANTICS_VERSION, MODEL_DIMENSIONS, MODEL_ID, MODEL_MAX_TOKENS,
    MODEL_NORMALIZATION, MODEL_REVISION, WIRE_PROTOCOL_VERSION,
};

const MAGIC: &[u8; 8] = b"FSCVEC02";
const TOKEN_TRUNCATED_FLAG: u8 = 1;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const HEADER_BYTES: usize = 8 + 4 + 4 + 4 + 32 + 4;

/// Why a persistent vector cache was or was not reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CacheLoadState {
    Disabled,
    Missing,
    Hit,
    Corrupt,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CachedVector {
    pub(super) values: Vec<f32>,
    pub(super) token_truncated: bool,
}

pub(super) struct VectorCache {
    path: PathBuf,
    entries: FxHashMap<SimilarCodeSourceDigest, CachedVector>,
    dirty: bool,
    disabled: bool,
    pub(super) load_state: CacheLoadState,
}

impl VectorCache {
    fn empty(path: PathBuf, load_state: CacheLoadState) -> Self {
        Self {
            path,
            entries: FxHashMap::default(),
            dirty: load_state == CacheLoadState::Corrupt,
            disabled: false,
            load_state,
        }
    }

    pub(super) fn load(cache_dir: &Path, disabled: bool) -> Self {
        let path = cache_path(cache_dir);
        if disabled {
            return Self {
                path,
                entries: FxHashMap::default(),
                dirty: false,
                disabled: true,
                load_state: CacheLoadState::Disabled,
            };
        }
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(&path).is_err() =>
            {
                return Self::empty(path, CacheLoadState::Missing);
            }
            Err(_) => return Self::empty(path, CacheLoadState::Corrupt),
        };
        if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES as u64 {
            return Self::empty(path, CacheLoadState::Corrupt);
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) if bytes.len() <= MAX_CACHE_BYTES => bytes,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && std::fs::symlink_metadata(&path).is_err() =>
            {
                return Self::empty(path, CacheLoadState::Missing);
            }
            Ok(_) | Err(_) => return Self::empty(path, CacheLoadState::Corrupt),
        };
        let Some(entries) = decode_cache(&bytes) else {
            return Self::empty(path, CacheLoadState::Corrupt);
        };
        Self {
            path,
            entries,
            dirty: false,
            disabled: false,
            load_state: CacheLoadState::Hit,
        }
    }

    pub(super) fn get(&self, digest: &SimilarCodeSourceDigest) -> Option<&CachedVector> {
        self.entries.get(digest)
    }

    pub(super) fn insert(
        &mut self,
        digest: SimilarCodeSourceDigest,
        values: Vec<f32>,
        token_truncated: bool,
    ) -> bool {
        if self.disabled
            || values.len() != MODEL_DIMENSIONS
            || values.iter().any(|value| !value.is_finite())
        {
            return false;
        }
        let entry = CachedVector {
            values,
            token_truncated,
        };
        if self.entries.get(&digest) == Some(&entry) {
            return false;
        }
        self.entries.insert(digest, entry);
        self.dirty = true;
        true
    }

    pub(super) fn save(&mut self) -> Result<bool, String> {
        if self.disabled || !self.dirty {
            return Ok(false);
        }
        let bytes = encode_cache(&self.entries);
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "similar-code vector cache has no parent directory".to_owned())?;
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create similar-code vector cache {}: {error}",
                parent.display()
            )
        })?;
        fallow_config::atomic_write(&self.path, &bytes).map_err(|error| {
            format!(
                "failed to publish similar-code vector cache {}: {error}",
                self.path.display()
            )
        })?;
        self.dirty = false;
        Ok(true)
    }
}

pub(super) fn clear(cache_dir: &Path) -> Result<bool, String> {
    let directory = cache_path(cache_dir)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "similar-code vector cache has no parent directory".to_owned())?;
    if !directory.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&directory).map_err(|error| {
        format!(
            "failed to remove similar-code vector cache {}: {error}",
            directory.display()
        )
    })?;
    Ok(true)
}

pub(super) fn parameter_digest() -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(MODEL_ID.as_bytes());
    hasher.update([0]);
    hasher.update(MODEL_REVISION.as_bytes());
    hasher.update([0]);
    hasher.update(MODEL_NORMALIZATION.as_bytes());
    hasher.update((MODEL_DIMENSIONS as u64).to_le_bytes());
    hasher.update((MODEL_MAX_TOKENS as u64).to_le_bytes());
    hasher.update(WIRE_PROTOCOL_VERSION.to_le_bytes());
    hasher.update(EXTRACTION_SEMANTICS_VERSION.to_le_bytes());
    hasher.finalize().into()
}

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir
        .join("similar-code")
        .join("v1")
        .join(MODEL_REVISION)
        .join("vectors.bin")
}

fn record_bytes() -> usize {
    32 + 1 + MODEL_DIMENSIONS * std::mem::size_of::<f32>()
}

fn max_records() -> usize {
    MAX_CACHE_BYTES
        .saturating_sub(HEADER_BYTES)
        .checked_div(record_bytes())
        .unwrap_or(0)
}

fn encode_cache(entries: &FxHashMap<SimilarCodeSourceDigest, CachedVector>) -> Vec<u8> {
    let mut ordered = entries.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(digest, _)| **digest);
    ordered.truncate(max_records());
    let mut bytes = Vec::with_capacity(HEADER_BYTES + ordered.len() * record_bytes());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&WIRE_PROTOCOL_VERSION.to_le_bytes());
    bytes.extend_from_slice(&EXTRACTION_SEMANTICS_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(MODEL_DIMENSIONS as u32).to_le_bytes());
    bytes.extend_from_slice(&parameter_digest());
    bytes.extend_from_slice(&(ordered.len() as u32).to_le_bytes());
    for (digest, entry) in ordered {
        bytes.extend_from_slice(digest.as_bytes());
        bytes.push(u8::from(entry.token_truncated) * TOKEN_TRUNCATED_FLAG);
        for value in &entry.values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

fn decode_cache(bytes: &[u8]) -> Option<FxHashMap<SimilarCodeSourceDigest, CachedVector>> {
    if bytes.len() < HEADER_BYTES || bytes.get(..8)? != MAGIC {
        return None;
    }
    let mut cursor = 8usize;
    let protocol = take_u32(bytes, &mut cursor)?;
    let extraction = take_u32(bytes, &mut cursor)?;
    let dimensions = take_u32(bytes, &mut cursor)? as usize;
    let parameters: [u8; 32] = bytes.get(cursor..cursor + 32)?.try_into().ok()?;
    cursor += 32;
    let count = take_u32(bytes, &mut cursor)? as usize;
    if protocol != WIRE_PROTOCOL_VERSION
        || extraction != EXTRACTION_SEMANTICS_VERSION
        || dimensions != MODEL_DIMENSIONS
        || parameters != parameter_digest()
        || count > max_records()
        || bytes.len() != HEADER_BYTES.checked_add(count.checked_mul(record_bytes())?)?
    {
        return None;
    }
    let mut entries = FxHashMap::default();
    for _ in 0..count {
        let digest = SimilarCodeSourceDigest::new(bytes.get(cursor..cursor + 32)?.try_into().ok()?);
        cursor += 32;
        let flags = *bytes.get(cursor)?;
        cursor += 1;
        if flags & !TOKEN_TRUNCATED_FLAG != 0 {
            return None;
        }
        let mut values = Vec::with_capacity(MODEL_DIMENSIONS);
        for _ in 0..MODEL_DIMENSIONS {
            let value = f32::from_le_bytes(bytes.get(cursor..cursor + 4)?.try_into().ok()?);
            cursor += 4;
            if !value.is_finite() {
                return None;
            }
            values.push(value);
        }
        let entry = CachedVector {
            values,
            token_truncated: flags & TOKEN_TRUNCATED_FLAG != 0,
        };
        if entries.insert(digest, entry).is_some() {
            return None;
        }
    }
    Some(entries)
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    let value = u32::from_le_bytes(bytes.get(*cursor..*cursor + 4)?.try_into().ok()?);
    *cursor += 4;
    Some(value)
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test fixture construction must fail immediately"
)]
mod tests {
    use super::*;

    fn vector(value: f32) -> Vec<f32> {
        vec![value; MODEL_DIMENSIONS]
    }

    fn entry(value: f32, token_truncated: bool) -> CachedVector {
        CachedVector {
            values: vector(value),
            token_truncated,
        }
    }

    #[test]
    fn round_trip_uses_full_digest_and_fixed_parameters() {
        let digest = SimilarCodeSourceDigest::new([7; 32]);
        let mut entries = FxHashMap::default();
        entries.insert(digest, entry(0.25, true));
        let decoded = decode_cache(&encode_cache(&entries)).unwrap();
        assert_eq!(decoded[&digest], entry(0.25, true));
    }

    #[test]
    fn corruption_and_parameter_drift_are_misses() {
        let digest = SimilarCodeSourceDigest::new([9; 32]);
        let mut entries = FxHashMap::default();
        entries.insert(digest, entry(0.5, false));
        let mut bytes = encode_cache(&entries);
        bytes[20] ^= 1;
        assert!(decode_cache(&bytes).is_none());
    }

    #[test]
    fn save_atomically_replaces_an_existing_cache() {
        let temp = tempfile::tempdir().unwrap();
        let first = SimilarCodeSourceDigest::new([1; 32]);
        let second = SimilarCodeSourceDigest::new([2; 32]);
        let mut cache = VectorCache::load(temp.path(), false);
        assert!(cache.insert(first, vector(0.25), true));
        assert!(cache.save().unwrap());

        let mut cache = VectorCache::load(temp.path(), false);
        assert!(cache.insert(second, vector(0.5), false));
        assert!(cache.save().unwrap());

        let cache = VectorCache::load(temp.path(), false);
        assert_eq!(cache.get(&first), Some(&entry(0.25, true)));
        assert_eq!(cache.get(&second), Some(&entry(0.5, false)));
    }

    #[test]
    fn old_and_malformed_cache_records_are_rejected() {
        let digest = SimilarCodeSourceDigest::new([3; 32]);
        let mut entries = FxHashMap::default();
        entries.insert(digest, entry(0.75, true));

        let mut old = encode_cache(&entries);
        old[..8].copy_from_slice(b"FSCVEC01");
        assert!(decode_cache(&old).is_none());

        let mut malformed = encode_cache(&entries);
        malformed[HEADER_BYTES + 32] = 2;
        assert!(decode_cache(&malformed).is_none());
    }

    #[test]
    fn corrupt_cache_is_atomically_rewritten_without_new_vectors() {
        let temp = tempfile::tempdir().unwrap();
        let path = cache_path(temp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid cache").unwrap();

        let mut cache = VectorCache::load(temp.path(), false);
        assert_eq!(cache.load_state, CacheLoadState::Corrupt);
        assert!(cache.save().unwrap());

        let cache = VectorCache::load(temp.path(), false);
        assert_eq!(cache.load_state, CacheLoadState::Hit);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn oversized_present_cache_is_corrupt_and_rewritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = cache_path(temp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        file.set_len((MAX_CACHE_BYTES as u64) + 1).unwrap();

        let mut cache = VectorCache::load(temp.path(), false);
        assert_eq!(cache.load_state, CacheLoadState::Corrupt);
        assert!(cache.save().unwrap());

        let cache = VectorCache::load(temp.path(), false);
        assert_eq!(cache.load_state, CacheLoadState::Hit);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn disabled_and_unchanged_entries_are_not_counted_as_writes() {
        let digest = SimilarCodeSourceDigest::new([4; 32]);
        let temp = tempfile::tempdir().unwrap();
        let mut disabled = VectorCache::load(temp.path(), true);
        assert!(!disabled.insert(digest, vector(0.25), false));

        let mut cache = VectorCache::load(temp.path(), false);
        assert!(cache.insert(digest, vector(0.25), false));
        assert!(!cache.insert(digest, vector(0.25), false));
        assert!(cache.insert(digest, vector(0.25), true));
    }

    #[test]
    fn clear_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!clear(temp.path()).unwrap());
        let path = cache_path(temp.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"cache").unwrap();
        assert!(clear(temp.path()).unwrap());
        assert!(!clear(temp.path()).unwrap());
    }
}
