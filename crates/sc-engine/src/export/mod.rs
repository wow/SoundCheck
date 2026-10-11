//! Exporting a batch. [`plan_export`] says what exporting does to one analysed file (write it
//! with a gain, a head cut and tags; leave it to the rekordbox XML; or skip it, and why);
//! [`plan_snapped_cut`] plans every value that depends on the head cut from the cut the
//! renderer makes, and [`plan_export_snapped`] does both for a file on disk; [`ExportSource`]
//! holds what the planner needs to know about the source file; [`ProcessSettings`] says how a
//! batch exports its files, one by one in `process` (the batch's `Task::Process`);
//! [`grid_check`] compares each written file's grid with the exported one; [`artefacts`]
//! turns what a batch did into its rekordbox XML and grid report.

pub mod artefacts;
pub mod grid_check;
mod plan;
pub(crate) mod process;
mod snap;
mod source;

pub use artefacts::{
    Artefacts, BatchRow, RowAction, RowTrack, XmlSelect, report_rows, write_artefacts, xml_tracks,
};
pub use plan::{
    ExportInput, ExportSource, SeratoPresence, plan_export, plan_snapped_cut, replaygain,
};
pub use process::{ProcessDone, ProcessSettings};
pub use snap::plan_export_snapped;
