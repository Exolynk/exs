//! The tasks that run the frames of resumable functions, and their host calls.
//!
//! A resumable function keeps its state in a heap `FrameObject`: its current state, its caller
//! and the caller slot that receives its result, its trace entry and its durable slots. A task
//! is a heap `TaskObject` with its active frame; the root task runs the entry, `par` starts
//! child tasks. The VM's [`exs_layout::TaskState`] holds the running task's state and the
//! scheduler's queues. Generated step functions run the active frame; these functions create,
//! link and complete frames, schedule tasks and start, finish, resume and cancel host calls.

mod frames;
mod host;
mod scheduler;
mod state;

pub use frames::{frame_call, frame_return, frame_start, take_completed};
pub use host::{Host, host_call_resume, host_call_start, host_call_take, task_cancel};
pub use scheduler::{parallel_new, parallel_wait, task_checkpoint, task_spawn, task_suspend};
