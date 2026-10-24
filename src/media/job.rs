//! Job system for conversions (Phase B-3).
//!
//! The existing Converter is a single-shot image pipeline. This module
//! provides the skeleton for progress, cancellation, and multi-format
//! dispatch that Phase B-5 (video/audio) will use.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Running { progress: f32 },
    Done,
    Cancelled,
    Failed(String),
}

pub struct Job {
    pub inputs: Vec<PathBuf>,
    pub target_format: String,
    pub status: JobStatus,
}

impl Job {
    pub fn new(inputs: Vec<PathBuf>, target_format: String) -> Self {
        Job { inputs, target_format, status: JobStatus::Pending }
    }
}
