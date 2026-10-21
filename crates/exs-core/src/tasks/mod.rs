//! The task that runs the frames of resumable functions, and its host calls.
//!
//! A resumable function keeps its state in a heap `FrameObject`: its current state, its caller
//! and the caller slot that receives its result, and its durable slots. The VM's
//! [`exs_layout::TaskState`] holds the active frame, the host result it consumes next and the
//! root frame's result. Generated step functions run the active frame; these functions create,
//! link and complete frames and start, finish, resume and cancel host calls.

mod frames;
mod host;
mod state;

pub use frames::{frame_call, frame_return, frame_start, take_completed};
pub use host::{Host, host_call_resume, host_call_start, host_call_take, task_cancel};
