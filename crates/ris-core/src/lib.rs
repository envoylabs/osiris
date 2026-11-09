//! The pure core of osiris.
//!
//! Everything here is a function of its inputs: a command and the
//! current state give events or a refusal, and state is a fold over the
//! event log. No clock, no filesystem, no database. Those live in the
//! shell that hosts this crate.
