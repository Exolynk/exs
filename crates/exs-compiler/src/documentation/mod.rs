//! API documentation: a model of every public declaration of the standard library, of
//! library modules, and of project modules, rendered as Markdown pages or as one compact
//! reference for an LLM.

mod build;
mod llm;
mod markdown;
mod model;
mod shared;
mod standard;

pub(crate) use build::{libraries, project};
pub(crate) use llm::render as render_llm;
pub(crate) use markdown::render as render_markdown;
pub use markdown::{MarkdownOptions, PageInfo, PageKind, PageLayout};
pub use model::{
    ApiDocumentation, CompactForm, ItemDocs, ItemKind, ItemRef, MemberDocs, ModuleDocs,
    ModuleOrigin, SectionDocs,
};
pub use standard::{
    StandardEnum, StandardFunction, StandardMethod, StandardNamespace, StandardTrait, StandardType,
    standard_library_enums, standard_library_functions, standard_library_namespace,
    standard_library_namespaces, standard_library_traits, standard_library_type_static_functions,
    standard_library_types,
};
