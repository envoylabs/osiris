//! The shell of osiris: the `ris` binary's library. Hosts the pure core,
//! owns the filesystem, the store and the clock, and renders output.

pub mod brief;
pub mod checks;
pub mod config;
pub mod decide;
pub mod drill;
pub mod evidence;
pub mod exits;
pub mod export;
pub mod graph;
pub mod import;
pub mod install;
pub mod preflight;
pub mod project;
pub mod render;
pub mod runner;
pub mod status;
pub mod style;
