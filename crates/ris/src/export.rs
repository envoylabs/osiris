//! `ris export`: the event log as JSONL, one file per closed phase.
//!
//! A phase's file holds every event after the previous phase's close up
//! to and including its own close, one JSON object per line, with text
//! and salts so another machine can verify and replay it. Evidence blobs
//! are not exported; their hashes are. Before anything is written, every
//! text field is scanned again under the current policy: the denylist
//! may have grown since the text went in. A closed phase's file never
//! changes, so an existing file must match byte for byte or the export
//! is refused.

use std::fs;
use std::path::Path;

use ris_core::event::{Event, Structure};
use ris_core::text::{Policy, scan};

use crate::project::CliError;

/// One file to write: its name and contents.
pub struct PhaseFile {
    pub name: String,
    pub contents: String,
}

/// Split the log at each phase close into one file per closed phase.
pub fn phase_files(log: &[Event]) -> Result<Vec<PhaseFile>, CliError> {
    let mut files = Vec::new();
    let mut start = 0;
    for (i, e) in log.iter().enumerate() {
        if let Structure::PhaseClosed { phase, .. } = &e.structure {
            let mut contents = String::new();
            for ev in &log[start..=i] {
                contents.push_str(
                    &serde_json::to_string(ev).map_err(|e| CliError::Other(e.to_string()))?,
                );
                contents.push('\n');
            }
            files.push(PhaseFile {
                name: format!("{phase}.jsonl"),
                contents,
            });
            start = i + 1;
        }
    }
    Ok(files)
}

/// Every text field in `log` that fails the scan, as `event seq: field`.
pub fn rescan(log: &[Event], policy: &Policy) -> Vec<String> {
    let mut hits = Vec::new();
    for e in log {
        if let Some(text) = &e.text {
            for (field, t) in text {
                if let Some(category) = scan(t.as_str(), policy) {
                    hits.push(format!(
                        "event {} `{field}`: {}",
                        e.seq,
                        category.description()
                    ));
                }
            }
        }
    }
    hits
}

/// Write the files into `dir`. Returns how many were new.
pub fn write(dir: &Path, files: &[PhaseFile]) -> Result<usize, CliError> {
    fs::create_dir_all(dir)?;
    for f in files {
        let path = dir.join(&f.name);
        if let Ok(existing) = fs::read_to_string(&path)
            && existing != f.contents
        {
            return Err(CliError::Other(format!(
                "{} exists and differs from the log; a closed phase's file never changes\n  \
                 next: export to a fresh directory, or find out why it differs",
                f.name
            )));
        }
    }
    let mut new = 0;
    for f in files {
        let path = dir.join(&f.name);
        if !path.exists() {
            fs::write(&path, &f.contents)?;
            new += 1;
        }
    }
    Ok(new)
}
