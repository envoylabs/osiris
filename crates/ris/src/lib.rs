//! The shell of osiris: the `ris` binary's library. Hosts the pure core,
//! owns the filesystem, the store and the clock, and renders output.

pub mod checks;
pub mod config;
pub mod evidence;
pub mod exits;
pub mod export;
pub mod import;
pub mod project;
pub mod render;
pub mod style;
