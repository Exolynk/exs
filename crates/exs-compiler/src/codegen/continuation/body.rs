//! Step-function body builder that tracks structured control nesting.

use wasm_encoder::{Function, Instruction};

/// A Wasm function body that records how deeply the current instruction is nested.
///
/// Continuation states branch directly to the dispatch loop or to later state labels, so the
/// emitter needs the relative label depth of every branch it writes.
pub(super) struct StepBody {
    /// Wasm function body under construction.
    function: Function,
    /// Number of currently open `block`, `loop` and `if` constructs.
    depth: u32,
    /// Runtime source position known to be current at this point of straight-line code.
    position: Option<u32>,
}

impl StepBody {
    /// Wraps an empty function body.
    pub(super) const fn new(function: Function) -> Self {
        Self {
            function,
            depth: 0,
            position: None,
        }
    }

    /// Appends one instruction and updates the control nesting depth.
    ///
    /// Control constructs may be entered from several paths, and generated function calls may
    /// set their own positions, so both forget the known runtime source position.
    pub(super) fn instruction(&mut self, instruction: &Instruction<'_>) -> &mut Self {
        match instruction {
            Instruction::Block(_) | Instruction::Loop(_) | Instruction::If(_) => {
                self.depth += 1;
                self.position = None;
            }
            Instruction::End => {
                self.depth = self.depth.saturating_sub(1);
                self.position = None;
            }
            Instruction::Else | Instruction::Call(_) | Instruction::CallIndirect { .. } => {
                self.position = None;
            }
            _ => {}
        }
        self.function.instruction(instruction);
        self
    }

    /// Sets the runtime source position unless straight-line code already set the same one.
    pub(super) fn source_position(&mut self, position: u32, set_source_position: u32) {
        if self.position == Some(position) {
            return;
        }
        self.function
            .instruction(&Instruction::I32Const(position.cast_signed()));
        self.function
            .instruction(&Instruction::Call(set_source_position));
        self.position = Some(position);
    }

    /// Calls one runtime export, which never changes the runtime source position.
    pub(super) fn runtime_call(&mut self, function: u32) {
        self.function.instruction(&Instruction::Call(function));
    }

    /// Returns the raw body for emitters whose control constructs are always balanced.
    pub(super) fn raw(&mut self) -> &mut Function {
        &mut self.function
    }

    /// Returns the number of currently open control constructs.
    pub(super) const fn depth(&self) -> u32 {
        self.depth
    }

    /// Returns the finished Wasm function body.
    pub(super) fn finish(self) -> Function {
        self.function
    }
}
