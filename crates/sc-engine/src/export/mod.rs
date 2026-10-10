//! Exporting a batch. [`plan_export`] says what exporting does to one analysed file (write it
//! with a gain, a head cut and tags; leave it to the rekordbox XML; or skip it, and why);
//! [`ExportSource`] holds what the planner needs to know about the source file.

mod plan;
mod source;

pub use plan::{ExportInput, ExportSource, SeratoPresence, plan_export, replaygain};
