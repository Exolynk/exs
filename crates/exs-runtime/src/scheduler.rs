//! Deterministic root-execution task scheduling state.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use exs_value::ValueRef;

/// Number of scheduler checkpoints a task may consume before yielding.
const TASK_QUANTUM: u32 = 64;

/// One execution-local task identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct TaskId(u64);

/// The result of delivering one runner host completion to the scheduler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostResume {
    /// The completion was delivered to its waiting task.
    Delivered,
    /// The completion belongs to a task cancelled before it resumed.
    Invalidated,
}

/// The scheduler-owned lifecycle state for one language task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskState {
    /// The task exists but has not entered the runnable queue.
    Created,
    /// The task is eligible to execute one scheduler quantum.
    Runnable,
    /// The task currently owns WebAssembly execution.
    Running,
    /// The task is suspended until one host completion is delivered.
    WaitingHost,
    /// The task is suspended until direct child tasks complete.
    WaitingChildren,
    /// The task produced its terminal result.
    Completed,
    /// The runner cancelled the task before it completed.
    #[allow(dead_code)] // Phase 9.3 exposes runner-driven cancellation.
    Cancelled,
}

/// One scheduler-owned language task.
struct RuntimeTask {
    /// Monotonic execution-local task identity.
    id: TaskId,
    /// Current lifecycle state.
    state: TaskState,
    /// The continuation frame currently dispatched for this task.
    frame: Option<u32>,
    /// The active runner host call, when one was started by this task.
    host_call: Option<u64>,
    /// A local or resumed host value awaiting continuation consumption.
    ready_host_result: Option<ValueRef>,
    /// Remaining scheduler checkpoints before this task yields its turn.
    quantum_remaining: u32,
    /// Parallel group and source-order result index when this is a child task.
    parallel: Option<(ValueRef, usize)>,
}

/// One parent-owned collection of parallel child task results.
struct ParallelGroup {
    /// Runtime-owned List value that identifies this pending group.
    handle: ValueRef,
    /// Task waiting for every child result.
    parent: TaskId,
    /// Source-order child results, filled as children complete.
    results: Vec<Option<ValueRef>>,
}

/// Mutable scheduler state for one root invocation.
pub(crate) struct ExecutionContext {
    /// All tasks created within this root execution.
    tasks: Vec<RuntimeTask>,
    /// Runnable task identifiers in deterministic first-in, first-out order.
    runnable: VecDeque<TaskId>,
    /// The only task currently allowed to execute Wasm instructions.
    current: Option<TaskId>,
    /// The next nonzero task identifier to allocate.
    next_task_id: u64,
    /// Host-call identifiers cancelled before their runner completions arrived.
    invalidated_host_calls: Vec<u64>,
    /// Active parent-owned parallel task groups.
    parallel_groups: Vec<ParallelGroup>,
}

impl ExecutionContext {
    /// Creates an execution context with one running root task.
    pub(crate) fn start() -> Self {
        let mut context = Self {
            tasks: Vec::new(),
            runnable: VecDeque::new(),
            current: None,
            next_task_id: 1,
            invalidated_host_calls: Vec::new(),
            parallel_groups: Vec::new(),
        };
        let root = context.create_task();
        context.transition(root, TaskState::Runnable);
        context.enqueue(root);
        context.run_next();
        context
    }

    /// Returns whether every task has reached a terminal state.
    pub(crate) fn is_complete(&self) -> bool {
        self.tasks
            .iter()
            .all(|task| matches!(task.state, TaskState::Completed | TaskState::Cancelled))
    }

    /// Associates the currently running task with one continuation frame.
    pub(crate) fn set_current_frame(&mut self, frame: u32) {
        self.current_task_mut().frame = Some(frame);
    }

    /// Returns the active continuation frame for the running task.
    pub(crate) fn current_frame(&self) -> Option<u32> {
        self.current_task().frame
    }

    /// Returns whether the scheduler has selected a runnable task for dispatch.
    pub(crate) fn has_current(&self) -> bool {
        self.current.is_some()
    }

    /// Consumes one scheduler checkpoint and rotates the running task when its quantum expires.
    pub(crate) fn checkpoint_current(&mut self) {
        let Some(task_id) = self.current else {
            crate::runtime::trap();
        };
        let task = self.current_task_mut();
        let Some(remaining) = task.quantum_remaining.checked_sub(1) else {
            crate::runtime::trap();
        };
        if remaining != 0 {
            task.quantum_remaining = remaining;
            return;
        }
        task.quantum_remaining = TASK_QUANTUM;
        self.transition(task_id, TaskState::Runnable);
        self.enqueue(task_id);
        self.current = None;
        self.run_next();
    }

    /// Records a host call started by the currently running task.
    pub(crate) fn begin_host_call(&mut self, call_id: u64) {
        let task = self.current_task_mut();
        if task.host_call.replace(call_id).is_some() {
            crate::runtime::trap();
        }
    }

    /// Suspends the current task after its host function returned Pending.
    pub(crate) fn suspend_current_for_host(&mut self, call_id: u64) {
        let Some(task_id) = self.current else {
            crate::runtime::trap();
        };
        if self.current_task().host_call != Some(call_id) {
            crate::runtime::trap();
        }
        self.transition(task_id, TaskState::WaitingHost);
        self.current = None;
        self.run_next_if_available();
    }

    /// Clears the active host call after the task consumes its ready response.
    pub(crate) fn finish_current_host_call(&mut self) {
        self.current_task_mut().host_call = None;
    }

    /// Returns the current task's active host call identifier.
    pub(crate) fn current_host_call(&self) -> Option<u64> {
        self.current_task().host_call
    }

    /// Stores a locally-created ready host Error for the running task.
    pub(crate) fn set_current_ready_host_result(&mut self, value: ValueRef) {
        let task = self.current_task_mut();
        if task.ready_host_result.replace(value).is_some() {
            crate::runtime::trap();
        }
    }

    /// Takes the current task's locally-created or resumed host result.
    pub(crate) fn take_current_ready_host_result(&mut self) -> Option<ValueRef> {
        self.current_task_mut().ready_host_result.take()
    }

    /// Delivers one host result and makes its waiting task the next runnable task.
    pub(crate) fn resume_host_call(&mut self, call_id: u64, value: ValueRef) -> HostResume {
        if self.invalidated_host_calls.contains(&call_id) {
            return HostResume::Invalidated;
        }
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.host_call == Some(call_id))
        else {
            crate::runtime::trap();
        };
        if task.state != TaskState::WaitingHost || task.ready_host_result.replace(value).is_some() {
            crate::runtime::trap();
        }
        let task_id = task.id;
        self.transition(task_id, TaskState::Runnable);
        self.enqueue(task_id);
        self.run_next_if_available();
        HostResume::Delivered
    }

    /// Cancels every nonterminal task and invalidates any pending host-call identifiers.
    pub(crate) fn cancel(&mut self) {
        for index in 0..self.tasks.len() {
            let task_id = self.tasks[index].id;
            let state = self.tasks[index].state;
            if matches!(state, TaskState::Completed | TaskState::Cancelled) {
                continue;
            }
            if let Some(call_id) = self.tasks[index].host_call {
                self.invalidated_host_calls.push(call_id);
            }
            self.transition(task_id, TaskState::Cancelled);
            let task = self.task_mut(task_id);
            task.host_call = None;
            task.ready_host_result = None;
            crate::runtime::task_release();
        }
        self.runnable.clear();
        self.current = None;
    }

    /// Creates one parent-owned group that will collect the supplied number of child results.
    pub(crate) fn parallel_new(&mut self, handle: ValueRef, count: usize) {
        let parent = self.current.unwrap_or_else(|| crate::runtime::trap());
        self.parallel_groups.push(ParallelGroup {
            handle,
            parent,
            results: vec![None; count],
        });
    }

    /// Registers one child frame in a parallel group without changing the active parent task.
    pub(crate) fn parallel_spawn(&mut self, handle: ValueRef, index: usize, frame: u32) {
        let group = self.parallel_group(handle);
        if index >= group.results.len() || group.results[index].is_some() {
            crate::runtime::trap();
        }
        let task_id = self.create_task();
        let task = self.task_mut(task_id);
        task.frame = Some(frame);
        task.parallel = Some((handle, index));
        self.transition(task_id, TaskState::Runnable);
        self.enqueue(task_id);
    }

    /// Suspends the current parent until every child in its group has produced one result.
    pub(crate) fn parallel_wait(&mut self, handle: ValueRef) {
        let parent = self.current.unwrap_or_else(|| crate::runtime::trap());
        let group = self.parallel_group(handle);
        if group.parent != parent {
            crate::runtime::trap();
        }
        if group.results.iter().all(Option::is_some) {
            return;
        }
        self.transition(parent, TaskState::WaitingChildren);
        self.current = None;
        self.run_next_if_available();
    }

    /// Returns all completed child results in source order and releases their group state.
    pub(crate) fn parallel_take_results(&mut self, handle: ValueRef) -> Vec<ValueRef> {
        let index = self
            .parallel_groups
            .iter()
            .position(|group| group.handle == handle)
            .unwrap_or_else(|| crate::runtime::trap());
        let group = self.parallel_groups.swap_remove(index);
        group
            .results
            .into_iter()
            .map(|value| value.unwrap_or_else(|| crate::runtime::trap()))
            .collect()
    }

    /// Marks the running task terminal and returns whether it completed the root execution.
    pub(crate) fn complete_current_task(&mut self, value: ValueRef) -> bool {
        let Some(task_id) = self.current else {
            crate::runtime::trap();
        };
        if crate::runtime::is_fatal_error(value) {
            self.cancel();
            return true;
        }
        let parallel = self.current_task().parallel;
        self.transition(task_id, TaskState::Completed);
        self.current = None;
        crate::runtime::task_release();
        let Some((handle, index)) = parallel else {
            return true;
        };
        let group = self.parallel_group_mut(handle);
        let Some(slot) = group.results.get_mut(index) else {
            crate::runtime::trap();
        };
        if slot.replace(value).is_some() {
            crate::runtime::trap();
        }
        if group.results.iter().all(Option::is_some) {
            let parent = group.parent;
            self.transition(parent, TaskState::Runnable);
            self.enqueue(parent);
        }
        self.run_next_if_available();
        false
    }

    /// Returns values held in task-owned host-result slots for garbage collection.
    pub(crate) fn roots(&self) -> impl Iterator<Item = ValueRef> + '_ {
        self.tasks
            .iter()
            .filter_map(|task| task.ready_host_result)
            .chain(
                self.parallel_groups
                    .iter()
                    .flat_map(|group| group.results.iter().flatten().copied()),
            )
    }

    /// Enqueues one task after every previously runnable task.
    fn enqueue(&mut self, task_id: TaskId) {
        self.runnable.push_back(task_id);
    }

    /// Allocates one Created task with the next execution-local identifier.
    fn create_task(&mut self) -> TaskId {
        let task_id = TaskId(self.next_task_id);
        let Some(next_task_id) = self.next_task_id.checked_add(1) else {
            crate::runtime::trap();
        };
        crate::runtime::task_acquire();
        self.next_task_id = next_task_id;
        self.tasks.push(RuntimeTask {
            id: task_id,
            state: TaskState::Created,
            frame: None,
            host_call: None,
            ready_host_result: None,
            quantum_remaining: TASK_QUANTUM,
            parallel: None,
        });
        task_id
    }

    /// Selects the next runnable task in queue order.
    fn run_next(&mut self) {
        if self.current.is_some() {
            crate::runtime::trap();
        }
        let Some(task_id) = self.runnable.pop_front() else {
            crate::runtime::trap();
        };
        self.transition(task_id, TaskState::Running);
        self.current = Some(task_id);
    }

    /// Selects the next runnable task only when one is queued.
    fn run_next_if_available(&mut self) {
        if self.current.is_none() && !self.runnable.is_empty() {
            self.run_next();
        }
    }

    /// Returns the mutable group identified by one compiler-internal List handle.
    fn parallel_group_mut(&mut self, handle: ValueRef) -> &mut ParallelGroup {
        self.parallel_groups
            .iter_mut()
            .find(|group| group.handle == handle)
            .unwrap_or_else(|| crate::runtime::trap())
    }

    /// Returns the group identified by one compiler-internal List handle.
    fn parallel_group(&self, handle: ValueRef) -> &ParallelGroup {
        self.parallel_groups
            .iter()
            .find(|group| group.handle == handle)
            .unwrap_or_else(|| crate::runtime::trap())
    }

    /// Returns the current running task.
    fn current_task(&self) -> &RuntimeTask {
        let Some(task_id) = self.current else {
            crate::runtime::trap();
        };
        self.task(task_id)
    }

    /// Returns the current running task mutably.
    fn current_task_mut(&mut self) -> &mut RuntimeTask {
        let Some(task_id) = self.current else {
            crate::runtime::trap();
        };
        self.task_mut(task_id)
    }

    /// Returns one task by its one-based monotonic identifier.
    fn task(&self, task_id: TaskId) -> &RuntimeTask {
        let index = usize::try_from(task_id.0 - 1).unwrap_or_else(|_| crate::runtime::trap());
        self.tasks
            .get(index)
            .unwrap_or_else(|| crate::runtime::trap())
    }

    /// Returns one task mutably by its one-based monotonic identifier.
    fn task_mut(&mut self, task_id: TaskId) -> &mut RuntimeTask {
        let index = usize::try_from(task_id.0 - 1).unwrap_or_else(|_| crate::runtime::trap());
        self.tasks
            .get_mut(index)
            .unwrap_or_else(|| crate::runtime::trap())
    }

    /// Validates and applies one state transition.
    fn transition(&mut self, task_id: TaskId, next: TaskState) {
        let task = self.task_mut(task_id);
        let valid = matches!(
            (task.state, next),
            (TaskState::Created, TaskState::Runnable)
                | (TaskState::Runnable, TaskState::Running)
                | (TaskState::Running, TaskState::Runnable)
                | (TaskState::Running, TaskState::WaitingHost)
                | (TaskState::Running, TaskState::WaitingChildren)
                | (TaskState::WaitingHost, TaskState::Runnable)
                | (TaskState::WaitingChildren, TaskState::Runnable)
                | (TaskState::Running, TaskState::Completed)
                | (TaskState::Created, TaskState::Cancelled)
                | (TaskState::Runnable, TaskState::Cancelled)
                | (TaskState::Running, TaskState::Cancelled)
                | (TaskState::WaitingHost, TaskState::Cancelled)
                | (TaskState::WaitingChildren, TaskState::Cancelled)
        );
        if !valid {
            crate::runtime::trap();
        }
        task.state = next;
    }
}
