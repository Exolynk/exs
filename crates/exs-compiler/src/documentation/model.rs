//! The documentation model: every public declaration of the standard library, of library
//! modules, and of project modules, independent of how it is rendered.

/// The documented API: the standard library, library modules, and project modules.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ApiDocumentation {
    /// Modules in order: `std` and its submodules first, then libraries, then project files.
    pub modules: Vec<ModuleDocs>,
}

/// One documented module.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ModuleDocs {
    /// Displayed module name: `std`, a library path such as `exo::db`, or a project file
    /// path such as `./math.exs`.
    pub name: String,
    /// Unique lowercase path segments that place the module in generated output, such as
    /// `["exo", "db"]`.
    pub segments: Vec<String>,
    /// Where the module comes from.
    pub origin: ModuleOrigin,
    /// Markdown overview, taken from the module's leading `//!` comment.
    pub description: String,
    /// Public declarations in declaration order: types, enums, traits, namespaces, constants,
    /// then functions.
    pub items: Vec<ItemDocs>,
}

/// Where a documented module comes from.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ModuleOrigin {
    /// The built-in standard library.
    Standard,
    /// A library module the embedder provides, which every module may name.
    Library,
    /// A source file of the project.
    Project,
}

/// One documented public declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ItemDocs {
    /// The kind of declaration.
    pub kind: ItemKind,
    /// Source-visible name.
    pub name: String,
    /// Markdown description.
    pub description: String,
    /// ExS declaration without bodies, such as `type Point {\n    x: Int,\n}`.
    pub declaration: String,
    /// Optional runnable ExS example.
    pub usage: Option<String>,
    /// Detail sections, such as variants or implemented methods.
    pub sections: Vec<SectionDocs>,
}

/// The kind of a documented declaration.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ItemKind {
    /// A nominal or runtime-owned type.
    Type,
    /// An enum.
    Enum,
    /// A trait.
    Trait,
    /// A namespace of static functions, such as `Host`.
    Namespace,
    /// A function.
    Function,
    /// A module constant.
    Constant,
}

impl ItemKind {
    /// Returns the label rendered before the item's name, such as `Type`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::Enum => "Enum",
            Self::Trait => "Trait",
            Self::Namespace => "Namespace",
            Self::Function => "Function",
            Self::Constant => "Constant",
        }
    }

    /// Reports whether the item is documented on its module's page in every layout, as
    /// functions and constants are.
    #[must_use]
    pub fn is_inline(self) -> bool {
        matches!(self, Self::Function | Self::Constant)
    }
}

/// One detail section of a documented declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SectionDocs {
    /// Section title, such as `Variants` or `Inherent methods`.
    pub title: String,
    /// The group the section belongs to, such as `Implemented Methods`; consecutive sections
    /// of one group share a heading.
    pub group: Option<String>,
    /// A declaration the title refers to, such as the implemented trait.
    pub link: Option<ItemRef>,
    /// Markdown introduction.
    pub description: String,
    /// How the compact reference shows the section.
    pub compact: CompactForm,
    /// Documented members in declaration order.
    pub members: Vec<MemberDocs>,
    /// Related declarations, rendered as a list of links.
    pub references: Vec<ItemRef>,
}

/// How the compact reference shows one section.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CompactForm {
    /// Lists the member declarations inside an ExS block with this header, such as
    /// `impl Iterator for Point`.
    Block(String),
    /// Lists the member declarations as a bullet list.
    List,
    /// Leaves the section out.
    Omit,
}

/// One documented member of a section, such as a method or a variant.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MemberDocs {
    /// Member name used for headings and anchors.
    pub name: String,
    /// Markdown description.
    pub description: String,
    /// ExS declaration, such as `fn len(self) -> Int`; empty when there is none.
    pub declaration: String,
    /// Optional runnable ExS example.
    pub example: Option<String>,
}

/// A reference to one documented declaration.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ItemRef {
    /// Name of the module that declares the item.
    pub module: String,
    /// Name of the item.
    pub name: String,
}

impl ApiDocumentation {
    /// Returns the module named `name`.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<&ModuleDocs> {
        self.modules.iter().find(|module| module.name == name)
    }
}

impl ModuleDocs {
    /// Returns the declaration named `name` other than a function or constant, or the
    /// function or constant when there is no such declaration.
    #[must_use]
    pub fn item(&self, name: &str) -> Option<&ItemDocs> {
        self.items
            .iter()
            .find(|item| item.name == name && !item.kind.is_inline())
            .or_else(|| self.items.iter().find(|item| item.name == name))
    }
}
