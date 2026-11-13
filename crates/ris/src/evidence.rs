//! Evidence blobs: content-addressed files under `.osiris/evidence/`.
//!
//! Test output, benchmark files and attached logs live here, named by
//! the SHA-256 of their content. The log holds only the hash, so an
//! export of state carries no blob and a blob can be deleted without
//! touching the chain. Writing is idempotent: the same content is the
//! same file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ris_core::ids::{EvidenceId, Hash};
use sha2::{Digest, Sha256};

/// A directory of blobs.
pub struct EvidenceStore {
    dir: PathBuf,
}

impl EvidenceStore {
    /// Use `dir`, creating it if needed.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// The id content would have, without storing it.
    pub fn id_of(bytes: &[u8]) -> EvidenceId {
        EvidenceId(Hash(Sha256::digest(bytes).into()))
    }

    /// Store `bytes`; return their id. Writes to a temporary name and
    /// renames, so a reader never sees a partial blob.
    pub fn put(&self, bytes: &[u8]) -> io::Result<EvidenceId> {
        let id = Self::id_of(bytes);
        let path = self.path_of(id);
        if path.exists() {
            return Ok(id);
        }
        let tmp = self
            .dir
            .join(format!(".tmp-{}-{}", std::process::id(), id.0));
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok(id)
    }

    /// Store a file's content.
    pub fn put_file(&self, file: &Path) -> io::Result<EvidenceId> {
        self.put(&fs::read(file)?)
    }

    /// The blob, if it is still here. Content is checked against the id
    /// on the way out.
    pub fn get(&self, id: EvidenceId) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path_of(id)) {
            Ok(bytes) => {
                if Self::id_of(&bytes) != id {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("evidence {id} does not match its content"),
                    ));
                }
                Ok(Some(bytes))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Whether the blob is present.
    pub fn has(&self, id: EvidenceId) -> bool {
        self.path_of(id).exists()
    }

    fn path_of(&self, id: EvidenceId) -> PathBuf {
        self.dir.join(id.0.to_hex())
    }
}

#[cfg(test)]
#[path = "tests/evidence.rs"]
mod tests;
