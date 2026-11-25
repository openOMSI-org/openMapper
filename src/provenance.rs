//! Original byte identities retained across imports and conversion.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const INDEX: &str = ".openmapper-provenance.json";
const LIMIT: u64 = 4 * 1024 * 1024;
static WRITER: Mutex<()> = Mutex::new(());

pub fn file_hash(path: &Path) -> io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub format_version: u32,
    /// Output byte digest -> original input byte digests. No machine paths are recorded.
    pub entries: BTreeMap<String, BTreeSet<String>>,
}

impl History {
    pub fn load(content: &Path) -> io::Result<Self> {
        let path = content.join(INDEX);
        if !path.exists() {
            return Ok(Self {
                format_version: 1,
                entries: BTreeMap::new(),
            });
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("provenance index exceeds its size limit"));
        }
        let history: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if history.format_version != 1 || history.entries.len() > 32768 {
            return Err(io::Error::other(
                "unsupported or oversized provenance index",
            ));
        }
        Ok(history)
    }
}

pub struct Source {
    hashes: BTreeSet<String>,
    inputs: BTreeMap<PathBuf, String>,
}

impl Source {
    /// This runs before any decoder or converter reads the input.
    pub fn capture(path: &Path) -> io::Result<Self> {
        if omsi_cfg::vfs::is_sealed(path) {
            return Err(io::Error::other(
                "protected playback assets cannot be imported for editing",
            ));
        }
        let mut source = Self { hashes: BTreeSet::new(), inputs: BTreeMap::new() };
        source.include(path)?;
        Ok(source)
    }

    pub fn include(&mut self, path: &Path) -> io::Result<()> {
        if omsi_cfg::vfs::is_sealed(path) {
            return Err(io::Error::other(
                "protected playback assets cannot be converted",
            ));
        }
        let path = std::path::absolute(path)?;
        let hash = file_hash(&path)?;
        if self.inputs.get(&path).is_some_and(|previous| previous != &hash) {
            return Err(io::Error::other("source changed during conversion"));
        }
        self.inputs.insert(path, hash.clone());
        self.hashes.insert(hash);
        Ok(())
    }

    pub fn record(&self, content: &Path, output: &Path) -> io::Result<()> {
        for (path, hash) in &self.inputs {
            if file_hash(path)? != *hash {
                return Err(io::Error::other("source changed during conversion; provenance was not recorded"));
            }
        }
        let _writer = WRITER
            .lock()
            .map_err(|_| io::Error::other("provenance index is busy"))?;
        let mut history = History::load(content)?;
        let mut stack = vec![output.to_path_buf()];
        while let Some(path) = stack.pop() {
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(io::Error::other(
                    "converted output contains a symbolic link",
                ));
            }
            if metadata.is_dir() {
                for entry in std::fs::read_dir(path)? {
                    stack.push(entry?.path());
                }
            } else if metadata.is_file() {
                history
                    .entries
                    .entry(file_hash(&path)?)
                    .or_default()
                    .extend(self.hashes.clone());
                if history.entries.len() > 32768 {
                    return Err(io::Error::other("provenance index has too many entries"));
                }
            }
        }
        let bytes = serde_json::to_vec(&history).map_err(io::Error::other)?;
        if bytes.len() as u64 > LIMIT {
            return Err(io::Error::other("provenance index exceeds its size limit"));
        }
        let target = content.join(INDEX);
        let temporary = content.join(format!("{INDEX}.{}.new", std::process::id()));
        let result = (|| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &target)
        })();
        if temporary.exists() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn changed_originals_cannot_be_recorded_as_the_preconversion_bytes() {
        let root = std::env::temp_dir().join(format!("om-source-change-{}", std::process::id()));
        std::fs::create_dir_all(root.join("output")).unwrap();
        let source = root.join("source.obj");
        std::fs::write(&source, b"owned original").unwrap();
        let provenance = super::Source::capture(&source).unwrap();
        std::fs::write(&source, b"changed during conversion").unwrap();
        std::fs::write(root.join("output/mesh.x"), b"converted bytes").unwrap();
        assert!(provenance.record(&root, &root.join("output")).is_err());
        assert!(!root.join(super::INDEX).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
