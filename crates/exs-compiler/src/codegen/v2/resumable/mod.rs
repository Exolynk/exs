//! Resumable functions of the v2 backend: the units that may suspend, directly through a host
//! call or through a resumable callee, compile to step functions over heap frames.
//!
//! A step function runs v1's continuation graph of its unit as a state machine over the
//! durable slots of its frame and returns a status to the dispatcher: ready to run the active
//! frame, pending on a host call, or complete.

mod analysis;
mod calls;
mod closures;
mod entry;
mod flow;
mod host;
mod layouts;
mod parallel;
mod step;
mod values;

pub(super) use analysis::{parallel_units, resumable_units};
pub(super) use layouts::{ClosureStep, Resumable};
pub(super) use step::StepEmitter;
