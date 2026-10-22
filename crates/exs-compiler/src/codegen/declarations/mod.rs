//! Function declarations as the code generator sees them: signatures, implementation methods
//! and lifted closures, and the checks every function body passes.

mod analysis;
mod lifted;
mod method;
mod signature;

pub(super) use analysis::contains_statements;
pub(super) use lifted::lifted_functions;
pub(super) use method::{InstanceMethod, MethodRegistry};
pub(super) use signature::{FunctionSignature, LiftedFunction, build_signatures, validate};
