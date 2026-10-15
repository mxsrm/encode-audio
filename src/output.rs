//! Transactional output: mux into a private temporary file, then publish.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tempfile::NamedTempFile;

use crate::config::parent;

/// Removes incomplete output on ordinary errors or panic unwinding.
/// A hard process kill can leave the temporary file; it never exposes a partial
/// file at the requested destination.
pub(crate) struct OutputFile {
    temp: NamedTempFile,
    destination: PathBuf,
}

impl OutputFile {
    pub fn new(destination: &Path) -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix(".encode-audio-")
            .tempfile_in(parent(destination))
            .context("creating temporary output")?;
        Ok(Self {
            temp,
            destination: destination.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        self.temp.path()
    }

    /// Called only after the FFmpeg output context has closed its file handle.
    pub fn commit(self) -> Result<()> {
        self.temp.as_file().sync_all().context("syncing encoded output")?;
        self.temp
            .persist_noclobber(&self.destination)
            .map_err(|e| e.error)
            .context("publishing output without overwriting")?;
        Ok(())
    }
}
