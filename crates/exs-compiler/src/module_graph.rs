//! Resolver-owned source graph loading and module-name resolution.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BinaryOperator, Block, Expression, FunctionDeclaration, Identifier, Module, Parameter,
    Statement, TestDeclaration, TypeAnnotation, Visibility,
};
use crate::diagnostic::CompileDiagnostics;
use crate::loaded_project::{ImportEdge, LoadedProject, LoadedSource};
use crate::{
    CompileError, CompileOptions, CompiledModule, CompiledTest, CompiledTests, SourceInput,
    SourceSpan,
};

/// Compiler-private name assigned to a production root entry while testing.
const PROGRAM_MAIN: &str = "$exs_program_main";

/// Compiler-private name assigned to the generated test dispatcher entry.
const TEST_MAIN: &str = "main";

/// Compiles a resolved source graph, keeping located entry-source diagnostics on failure.
pub(super) fn compile(
    source: SourceInput<'_>,
    options: CompileOptions<'_>,
) -> Result<CompiledModule, CompileError> {
    compile_target(source, options, false).map(|(module, _)| module)
}

/// Compiles the test dispatcher, keeping located entry-source diagnostics on failure.
pub(super) fn compile_tests(
    source: SourceInput<'_>,
    options: CompileOptions<'_>,
) -> Result<CompiledTests, CompileError> {
    let (module, tests) = compile_target(source, options, true)?;
    Ok(CompiledTests {
        wasm: module.wasm,
        tests,
    })
}

/// Compiles one source graph for either production or individual source-test execution.
fn compile_target(
    source: SourceInput<'_>,
    options: CompileOptions<'_>,
    test_target: bool,
) -> Result<(CompiledModule, Vec<CompiledTest>), CompileError> {
    let project = LoadedProject::load(source, options.module_resolver(), options.limits)?;
    validate_import_names(&project)?;
    project.validate_no_cycles()?;
    let files = project.files();
    let edges = project.edges();
    let mut modules = project.parse_modules(!test_target)?;
    let mut exports = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        if index != 0
            && let Some(main) = module
                .functions
                .iter()
                .find(|function| function.name.name == "main")
        {
            return Err(CompileError::at(
                "E0235",
                main.name.span,
                &files[index].text,
                "imported modules must not declare fn main()",
            ));
        }
        exports.push(collect_exports(module, index, &files[index].text)?);
    }
    let libraries: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| file.library.as_deref().map(|path| (path, index)))
        .collect();
    let packages = packages(files, edges);
    let library_bindings = library_bindings(&libraries, &exports, &packages);
    let prelude_private = crate::prelude::private_functions();
    for (module, package) in modules.iter_mut().zip(&packages) {
        let functions = module.functions.iter_mut().chain(
            module
                .implementations
                .iter_mut()
                .flat_map(|implementation| &mut implementation.methods),
        );
        for function in functions {
            function.package.clone_from(package);
        }
    }
    let mut root_tests = if test_target {
        std::mem::take(&mut modules[0].tests)
    } else {
        Vec::new()
    };
    let tests = test_metadata(&root_tests, &files[0].text)?;
    let mut combined = Module {
        imports: Vec::new(),
        uses: Vec::new(),
        types: Vec::new(),
        enums: Vec::new(),
        traits: Vec::new(),
        implementations: Vec::new(),
        functions: Vec::new(),
        constants: Vec::new(),
        tests: Vec::new(),
    };
    let mut all_bindings = Vec::with_capacity(modules.len());
    for index in 0..modules.len() {
        all_bindings.push(bindings_for(
            &modules[index],
            index,
            &packages[index],
            &edges[index],
            &exports,
            &libraries,
            &library_bindings,
            &prelude_private,
            &files[index],
        )?);
    }
    let constants = crate::constants::Declared::new(&modules, canonical)
        .resolve(&all_bindings)
        .map_err(|error| project_error(&error, files))?;
    for (index, mut bindings) in all_bindings.into_iter().enumerate() {
        let visible = bindings
            .iter()
            .filter_map(|(name, canonical)| {
                constants
                    .get(canonical)
                    .map(|value| (name.clone(), value.clone()))
            })
            .collect();
        let private = bindings
            .iter()
            .filter(|(name, canonical)| {
                canonical.starts_with(PRIVATE_PREFIX)
                    && library_bindings
                        .get(*name)
                        .is_some_and(|binding| constants.contains_key(&binding.canonical))
            })
            .map(|(name, _)| name.clone())
            .collect();
        crate::constants::inline(&mut modules[index], &visible, &private)
            .map_err(|error| project_error(&error, files))?;
        if test_target && index == 0 {
            crate::constants::inline_tests(&mut root_tests, &visible, &private)
                .map_err(|error| project_error(&error, files))?;
        }
        if test_target && index == 0 {
            if let Some(function) = modules[index]
                .functions
                .iter_mut()
                .find(|function| function.name.name == "main")
            {
                function.name.name = PROGRAM_MAIN.to_owned();
                function.entry = false;
                bindings.insert("main".to_owned(), PROGRAM_MAIN.to_owned());
            }
            for test in &mut root_tests {
                rewrite_block(&mut test.body, &bindings);
            }
        }
        rewrite_module(&mut modules[index], index, &bindings);
        if index != 0 {
            for function in &mut modules[index].functions {
                function.entry = false;
            }
        }
        combined.types.append(&mut modules[index].types);
        combined.enums.append(&mut modules[index].enums);
        combined.traits.append(&mut modules[index].traits);
        combined
            .implementations
            .append(&mut modules[index].implementations);
        combined.functions.append(&mut modules[index].functions);
    }
    if test_target {
        if root_tests.is_empty() {
            return Err(CompileError::at(
                "E0235",
                SourceSpan::empty(&files[0].source_id),
                &files[0].text,
                "test source declares no tests",
            ));
        }
        append_test_functions(&mut combined, &root_tests)?;
    }
    let mut prelude = crate::prelude::parse().map_err(|error| project_error(&error, files))?;
    prelude.types.append(&mut combined.types);
    prelude.enums.append(&mut combined.enums);
    prelude.traits.append(&mut combined.traits);
    prelude
        .implementations
        .append(&mut combined.implementations);
    combined.functions.append(&mut prelude.functions);
    prelude.functions = combined.functions;
    combined = prelude;
    if combined
        .functions
        .iter()
        .filter(|function| function.name.name == "main")
        .count()
        != 1
    {
        return Err(CompileError::at(
            "E0235",
            SourceSpan::empty(&files[0].source_id),
            &files[0].text,
            "root module must declare exactly one fn main()",
        ));
    }
    let mut source_inputs = crate::prelude::source_inputs();
    source_inputs.extend(files.iter().map(|file| SourceInput {
        source_id: &file.source_id,
        text: &file.text,
    }));
    let wasm = crate::codegen::compile_project_module(&mut combined, &source_inputs, options)
        .map_err(|error| project_error(&error, files))?;
    Ok((CompiledModule { wasm }, tests))
}

/// Creates the failure of `diagnostics`, located in the sources of the project or the prelude.
fn project_error(diagnostics: &CompileDiagnostics<'_>, files: &[LoadedSource]) -> CompileError {
    let prelude = crate::prelude::source_inputs();
    CompileError::from_sources(diagnostics, |source_id| {
        files
            .iter()
            .find(|file| file.source_id == source_id)
            .map(|file| file.text.as_str())
            .or_else(|| {
                prelude
                    .iter()
                    .find(|source| source.source_id == source_id)
                    .map(|source| source.text)
            })
            .unwrap_or_default()
    })
}

/// Validates source test names of the root source `text` and returns runner-visible metadata in
/// declaration order.
fn test_metadata(
    tests: &[TestDeclaration<'_>],
    text: &str,
) -> Result<Vec<CompiledTest>, CompileError> {
    let mut names = HashSet::new();
    let mut metadata = Vec::with_capacity(tests.len());
    for test in tests {
        if test.description.is_empty() {
            return Err(CompileError::at(
                "E0235",
                test.span,
                text,
                "test descriptions must not be empty",
            ));
        }
        if !names.insert(test.description.as_str()) {
            return Err(CompileError::at(
                "E0235",
                test.span,
                text,
                format!("duplicate test description `{}`", test.description),
            ));
        }
        metadata.push(CompiledTest {
            description: test.description.clone(),
        });
    }
    Ok(metadata)
}

/// Appends compiler-private test functions and a dispatcher entry point to one module.
fn append_test_functions<'a>(
    module: &mut Module<'a>,
    tests: &[TestDeclaration<'a>],
) -> Result<(), CompileError> {
    for (index, test) in tests.iter().enumerate() {
        module.functions.push(FunctionDeclaration {
            visibility: Visibility::Private,
            entry: false,
            package: String::new(),
            name: Identifier {
                name: test_function_name(index),
                span: test.span,
            },
            parameters: Vec::new(),
            return_type: None,
            body: test.body.clone(),
            span: test.span,
        });
    }
    let span = tests[0].span;
    let test_index = Identifier {
        name: "$exs_test_index".to_owned(),
        span,
    };
    let mut statements = Vec::with_capacity(tests.len() + 1);
    for index in 0..tests.len() {
        let index_value = i128::try_from(index)
            .map_err(|_| "too many tests for ExS Int test dispatch".to_owned())?;
        let condition = Expression::Binary {
            operator: BinaryOperator::Equal,
            left: Box::new(Expression::Variable(test_index.clone())),
            right: Box::new(Expression::Integer(index_value, span)),
            span,
        };
        let result = Expression::Call {
            callee: Identifier {
                name: test_function_name(index),
                span,
            },
            arguments: Vec::new(),
            span,
        };
        statements.push(Statement::If {
            condition,
            then_block: Block {
                statements: vec![Statement::Return {
                    value: Some(result),
                    span,
                }],
                span,
            },
            else_branch: None,
            span,
        });
    }
    statements.push(Statement::Return {
        value: Some(Expression::Call {
            callee: Identifier {
                name: "Error".to_owned(),
                span,
            },
            arguments: vec![
                Expression::String("TestRunnerError".to_owned(), span),
                Expression::String("unknown test index".to_owned(), span),
                Expression::Variable(test_index.clone()),
            ],
            span,
        }),
        span,
    });
    module.functions.push(FunctionDeclaration {
        visibility: Visibility::Public,
        entry: true,
        package: String::new(),
        name: Identifier {
            name: TEST_MAIN.to_owned(),
            span,
        },
        parameters: vec![Parameter {
            name: test_index,
            type_annotation: None,
            variadic: false,
        }],
        return_type: None,
        body: Block { statements, span },
        span,
    });
    Ok(())
}

/// Returns the compiler-private function name for one source test index.
fn test_function_name(index: usize) -> String {
    format!("$exs_test_{index}")
}

/// The declarations of one module with their canonical names, and which are private.
struct Exports {
    /// Canonical name of every declaration and enum variant by its name in the module.
    names: HashMap<String, String>,
    /// Names of the private declarations and the variants of private enums.
    private: HashSet<String>,
}

/// Collects direct declarations that another module may import.
fn collect_exports(module: &Module<'_>, index: usize, text: &str) -> Result<Exports, CompileError> {
    let mut exports = HashMap::new();
    let mut private = HashSet::new();
    let declarations = module
        .functions
        .iter()
        .map(|item| (&item.name, item.visibility, DeclarationKind::Function))
        .chain(
            module
                .types
                .iter()
                .map(|item| (&item.name, item.visibility, DeclarationKind::Nominal)),
        )
        .chain(
            module
                .enums
                .iter()
                .map(|item| (&item.name, item.visibility, DeclarationKind::Nominal)),
        )
        .chain(
            module
                .traits
                .iter()
                .map(|item| (&item.name, item.visibility, DeclarationKind::Nominal)),
        )
        .chain(
            module
                .constants
                .iter()
                .map(|item| (&item.name, item.visibility, DeclarationKind::Constant)),
        );
    let mut kinds = HashMap::new();
    for (identifier, visibility, kind) in declarations {
        let name = &identifier.name;
        if visibility == Visibility::Private {
            private.insert(name.clone());
        }
        if name == "Self" {
            return Err(CompileError::at(
                "E0219",
                identifier.span,
                text,
                "`Self` is reserved for trait type annotations",
            ));
        }
        if matches!(name.as_str(), "assert" | "assert_eq") {
            return Err(CompileError::at(
                "E0219",
                identifier.span,
                text,
                format!("`{name}` is a reserved standard-library function"),
            ));
        }
        // Compilation reports declarations of one kind that share a name, each of them located.
        match kinds.insert(name.as_str(), kind) {
            Some(_) if kind == DeclarationKind::Constant => {
                return Err(CompileError::at(
                    "E0122",
                    identifier.span,
                    text,
                    format!("duplicate declaration `{name}`"),
                ));
            }
            Some(previous) if previous != kind => {
                return Err(CompileError::at(
                    "E0219",
                    identifier.span,
                    text,
                    format!("duplicate exported declaration `{name}`"),
                ));
            }
            _ => {}
        }
        let canonical = if kind == DeclarationKind::Nominal {
            nominal_symbol(index, name)
        } else {
            canonical(index, name)
        };
        exports.insert(name.clone(), canonical);
    }
    for declaration in &module.enums {
        for variant in &declaration.variants {
            let name = format!("{}::{}", declaration.name.name, variant.name.name);
            if declaration.visibility == Visibility::Private {
                private.insert(name.clone());
            }
            let canonical = format!(
                "{}::{}",
                nominal_symbol(index, &declaration.name.name),
                variant.name.name
            );
            exports.insert(name, canonical);
        }
    }
    Ok(Exports {
        names: exports,
        private,
    })
}

/// What a top-level declaration declares, as far as sharing a name matters.
#[derive(Clone, Copy, Eq, PartialEq)]
enum DeclarationKind {
    /// A function.
    Function,
    /// A type, enum or trait.
    Nominal,
    /// A constant.
    Constant,
}

/// Returns the package of every loaded source: its library's name for a library module,
/// such as `app` for `app::db`, else the package of the source importing it, empty for the
/// project.
fn packages(files: &[LoadedSource], edges: &[Vec<ImportEdge>]) -> Vec<String> {
    let mut packages: Vec<Option<String>> = files
        .iter()
        .map(|file| {
            file.library
                .as_deref()
                .map(|path| path.split("::").next().unwrap_or(path).to_owned())
        })
        .collect();
    if let Some(root) = packages.first_mut() {
        root.get_or_insert_with(String::new);
    }
    // Sources are discovered after the sources importing them.
    for (index, targets) in edges.iter().enumerate() {
        let package = packages[index].clone().unwrap_or_default();
        for edge in targets {
            packages[edge.target].get_or_insert_with(|| package.clone());
        }
    }
    packages
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect()
}

/// Returns the qualified name of every declaration of the library modules `libraries`, by
/// path and source index, such as `log::info`, with its canonical name. Every module may
/// name them.
fn library_bindings(
    libraries: &HashMap<&str, usize>,
    exports: &[Exports],
    packages: &[String],
) -> HashMap<String, LibraryBinding> {
    let mut bindings = HashMap::new();
    for (path, index) in libraries {
        let module = &exports[*index];
        for (name, canonical) in &module.names {
            bindings.insert(
                format!("{path}::{name}"),
                LibraryBinding {
                    canonical: canonical.clone(),
                    private_to: module
                        .private
                        .contains(name)
                        .then(|| packages[*index].clone()),
                },
            );
        }
    }
    bindings
}

/// One declaration of a library module as other modules see it.
struct LibraryBinding {
    /// Its canonical name.
    canonical: String,
    /// The package it is private to, if it is private.
    private_to: Option<String>,
}

/// Prefix of the binding of a name the module may not use because it is private elsewhere.
pub(crate) const PRIVATE_PREFIX: &str = "$private:";

/// Returns the binding of `name`, private to another package, which diagnostics name.
fn hidden(name: &str) -> String {
    format!("{PRIVATE_PREFIX}{name}")
}

/// Builds canonical name lookups visible from one source module: its own declarations,
/// every library module's declarations by their path, its import namespaces and its `use`
/// declarations.
#[allow(clippy::too_many_arguments)] // The module, its package and the project's names.
fn bindings_for(
    module: &Module<'_>,
    index: usize,
    package: &str,
    edges: &[ImportEdge],
    exports: &[Exports],
    libraries: &HashMap<&str, usize>,
    library_bindings: &HashMap<String, LibraryBinding>,
    prelude_private: &[&str],
    file: &LoadedSource,
) -> Result<HashMap<String, String>, CompileError> {
    let text = file.text.as_str();
    let mut bindings = collect_exports(module, index, text)?.names;
    for (name, binding) in library_bindings {
        let canonical = match &binding.private_to {
            Some(owner) if owner != package => hidden(name),
            _ => binding.canonical.clone(),
        };
        bindings.insert(name.clone(), canonical);
    }
    // The prelude's private functions stay callable only by the prelude itself.
    for name in prelude_private {
        bindings
            .entry((*name).to_owned())
            .or_insert_with(|| hidden(name));
    }
    let mut namespaces: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in edges {
        let alias = &edge.namespace;
        let edge_error = |message: String| {
            CompileError::at(
                "E0233",
                SourceSpan {
                    source_id: &file.source_id,
                    start_byte: edge.start_byte,
                    end_byte: edge.end_byte,
                },
                text,
                message,
            )
        };
        if alias == "std" {
            return Err(edge_error(
                "`std` is a reserved built-in type namespace".to_owned(),
            ));
        }
        if libraries
            .keys()
            .any(|path| path.split("::").next() == Some(alias.as_str()))
        {
            return Err(edge_error(format!(
                "import namespace `{alias}` collides with the library of the same name"
            )));
        }
        let names = namespaces.entry(alias).or_default();
        for (name, canonical) in &exports[edge.target].names {
            if !names.insert(name) {
                return Err(edge_error(format!(
                    "duplicate export `{name}` in merged namespace `{alias}`"
                )));
            }
            bindings.insert(format!("{alias}::{name}"), canonical.clone());
        }
    }
    for declaration in &module.uses {
        for item in &declaration.items {
            let source = if declaration.namespace.name.is_empty() {
                item.name.name.clone()
            } else {
                format!("{}::{}", declaration.namespace.name, item.name.name)
            };
            let local = item
                .alias
                .as_ref()
                .map_or(&item.name.name, |alias| &alias.name);
            let Some(canonical) = bindings.get(&source).cloned() else {
                use_module(&mut bindings, &source, local, libraries)
                    .map_err(|error| CompileError::at("E0234", declaration.span, text, error))?;
                continue;
            };
            if canonical.starts_with(PRIVATE_PREFIX) {
                return Err(CompileError::at(
                    "E0234",
                    declaration.span,
                    text,
                    format!("`{source}` is private to its library"),
                ));
            }
            if bindings.insert(local.clone(), canonical).is_some() {
                return Err(CompileError::at(
                    "E0234",
                    declaration.span,
                    text,
                    format!("used name `{local}` collides with an existing declaration"),
                ));
            }
            let variant_prefix = format!("{source}::");
            let variants = bindings
                .iter()
                .filter_map(|(name, canonical)| {
                    name.strip_prefix(&variant_prefix)
                        .map(|suffix| (suffix.to_owned(), canonical.clone()))
                })
                .collect::<Vec<_>>();
            for (variant, canonical) in variants {
                let variant_local = format!("{local}::{variant}");
                if bindings.insert(variant_local.clone(), canonical).is_some() {
                    return Err(CompileError::at(
                        "E0234",
                        declaration.span,
                        text,
                        format!(
                            "used enum variant `{variant_local}` collides with an existing declaration"
                        ),
                    ));
                }
            }
        }
    }
    Ok(bindings)
}

/// Binds the module at `path`, a library module or an import namespace, under `local`, so
/// `local::name` names its declarations; `use log;` binds nothing new.
fn use_module(
    bindings: &mut HashMap<String, String>,
    path: &str,
    local: &str,
    libraries: &HashMap<&str, usize>,
) -> Result<(), String> {
    let prefix = format!("{path}::");
    let members = bindings
        .iter()
        .filter_map(|(name, canonical)| {
            name.strip_prefix(&prefix)
                .map(|member| (member.to_owned(), canonical.clone()))
        })
        .collect::<Vec<_>>();
    if members.is_empty() && !libraries.contains_key(path) {
        return Err(format!("unknown import `{path}`"));
    }
    if local == path {
        return Ok(());
    }
    for (member, canonical) in members {
        let name = format!("{local}::{member}");
        if bindings.insert(name.clone(), canonical).is_some() {
            return Err(format!(
                "used name `{name}` collides with an existing declaration"
            ));
        }
    }
    Ok(())
}

/// Rewrites declared and referenced top-level symbols to linker-stable canonical names.
fn rewrite_module(module: &mut Module<'_>, index: usize, bindings: &HashMap<String, String>) {
    for declaration in &mut module.types {
        rename_nominal(&mut declaration.name, index);
        for field in &mut declaration.fields {
            if let Some(annotation) = &mut field.type_annotation {
                rewrite_annotation(annotation, bindings);
            }
        }
    }
    for declaration in &mut module.enums {
        rename_nominal(&mut declaration.name, index);
        for variant in &mut declaration.variants {
            for field in &mut variant.fields {
                if let Some(annotation) = &mut field.type_annotation {
                    rewrite_annotation(annotation, bindings);
                }
            }
        }
    }
    for declaration in &mut module.traits {
        rename_nominal(&mut declaration.name, index);
        for method in &mut declaration.methods {
            rewrite_function_parts(
                &mut method.parameters,
                &mut method.return_type,
                method.body.as_mut(),
                bindings,
            );
        }
    }
    for declaration in &mut module.implementations {
        rewrite_identifier(&mut declaration.type_name, bindings);
        if let Some(trait_name) = &mut declaration.trait_name {
            rewrite_identifier(trait_name, bindings);
        }
        for method in &mut declaration.methods {
            rewrite_function_parts(
                &mut method.parameters,
                &mut method.return_type,
                Some(&mut method.body),
                bindings,
            );
        }
    }
    for function in &mut module.functions {
        rewrite_function(function, index, bindings);
    }
}

/// Rewrites one direct function declaration and its body.
fn rewrite_function(
    function: &mut FunctionDeclaration<'_>,
    index: usize,
    bindings: &HashMap<String, String>,
) {
    rename(&mut function.name, index);
    rewrite_function_parts(
        &mut function.parameters,
        &mut function.return_type,
        Some(&mut function.body),
        bindings,
    );
}

/// Rewrites function boundary annotations and an optional executable body.
fn rewrite_function_parts(
    parameters: &mut [crate::ast::Parameter<'_>],
    return_type: &mut Option<TypeAnnotation<'_>>,
    body: Option<&mut Block<'_>>,
    bindings: &HashMap<String, String>,
) {
    for parameter in parameters {
        if let Some(annotation) = &mut parameter.type_annotation {
            rewrite_annotation(annotation, bindings);
        }
    }
    if let Some(annotation) = return_type {
        rewrite_annotation(annotation, bindings);
    }
    if let Some(body) = body {
        rewrite_block(body, bindings);
    }
}

/// Rewrites one type annotation.
fn rewrite_annotation(annotation: &mut TypeAnnotation<'_>, bindings: &HashMap<String, String>) {
    for member in &mut annotation.members {
        if let Some(name) = bindings.get(&member.name) {
            member.name = name.clone();
        }
        if let Some(argument) = member.argument.as_deref_mut() {
            rewrite_annotation(argument, bindings);
        }
    }
}

/// Rewrites direct calls and type receivers within one block.
fn rewrite_block(block: &mut Block<'_>, bindings: &HashMap<String, String>) {
    for statement in &mut block.statements {
        rewrite_statement(statement, bindings);
    }
}

/// Rewrites source-level symbol references in one statement.
fn rewrite_statement(statement: &mut Statement<'_>, bindings: &HashMap<String, String>) {
    match statement {
        Statement::Let {
            type_annotation,
            value,
            ..
        } => {
            if let Some(annotation) = type_annotation {
                rewrite_annotation(annotation, bindings);
            }
            rewrite_expression(value, bindings);
        }
        Statement::Expression {
            expression: value, ..
        } => rewrite_expression(value, bindings),
        Statement::Assign { target, value, .. } => {
            match target {
                crate::ast::AssignmentTarget::Index {
                    receiver, index, ..
                } => {
                    rewrite_expression(receiver, bindings);
                    rewrite_expression(index, bindings);
                }
                crate::ast::AssignmentTarget::Property { receiver, .. } => {
                    rewrite_expression(receiver, bindings)
                }
                crate::ast::AssignmentTarget::Variable(_) => {}
            }
            rewrite_expression(value, bindings);
        }
        Statement::Return { value, .. } => {
            if let Some(value) = value {
                rewrite_expression(value, bindings);
            }
        }
        Statement::Block { block, .. } => rewrite_block(block, bindings),
        Statement::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            rewrite_expression(condition, bindings);
            rewrite_block(then_block, bindings);
            if let Some(else_branch) = else_branch {
                match else_branch {
                    crate::ast::ElseBranch::Block(block) => rewrite_block(block, bindings),
                    crate::ast::ElseBranch::If(statement) => rewrite_statement(statement, bindings),
                }
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            rewrite_expression(condition, bindings);
            rewrite_block(body, bindings);
        }
        Statement::For {
            binding: _,
            type_annotation,
            iterable,
            body,
            ..
        } => {
            if let Some(annotation) = type_annotation {
                rewrite_annotation(annotation, bindings);
            }
            rewrite_expression(iterable, bindings);
            rewrite_block(body, bindings);
        }
        Statement::Break { .. } | Statement::Continue { .. } => {}
    }
}

/// Rewrites source-level symbol references in one expression.
fn rewrite_expression(expression: &mut Expression<'_>, bindings: &HashMap<String, String>) {
    match expression {
        Expression::FormattedString { parts, .. } => {
            for part in parts {
                if let crate::ast::FormattedStringPart::Expression(expression) = part {
                    rewrite_expression(expression, bindings);
                }
            }
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            rewrite_identifier(callee, bindings);
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::StaticMethodCall {
            type_name,
            method,
            arguments,
            span,
        } => {
            // `module::function(...)` of a used module parses like a static call.
            let function = format!("{}::{}", type_name.name, method.name);
            if !bindings.contains_key(&type_name.name)
                && let Some(canonical) = bindings.get(&function)
            {
                let callee = Identifier {
                    name: canonical.clone(),
                    span: type_name.span.through(method.span),
                };
                let mut arguments = std::mem::take(arguments);
                for argument in &mut arguments {
                    rewrite_expression(argument, bindings);
                }
                *expression = Expression::Call {
                    callee,
                    arguments,
                    span: *span,
                };
                return;
            }
            rewrite_identifier(type_name, bindings);
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::TypedObject {
            type_name,
            properties,
            ..
        } => {
            rewrite_identifier(type_name, bindings);
            for property in properties {
                rewrite_expression(&mut property.value, bindings);
            }
        }
        Expression::MethodCall {
            receiver,
            arguments,
            ..
        } => {
            rewrite_expression(receiver, bindings);
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::HostCall {
            name, arguments, ..
        } => {
            rewrite_expression(name, bindings);
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::HostStream { arguments, .. } => {
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::HostTime { arguments, .. } => {
            for argument in arguments {
                rewrite_expression(argument, bindings);
            }
        }
        Expression::Closure {
            parameters, body, ..
        } => {
            for parameter in parameters {
                if let Some(annotation) = &mut parameter.type_annotation {
                    rewrite_annotation(annotation, bindings);
                }
            }
            rewrite_block(body, bindings);
        }
        Expression::ParallelStatic { tasks, .. }
        | Expression::List {
            elements: tasks, ..
        } => {
            for task in tasks {
                rewrite_expression(task, bindings);
            }
        }
        Expression::IsType {
            value,
            type_annotation,
            ..
        } => {
            rewrite_annotation(type_annotation, bindings);
            rewrite_expression(value, bindings);
        }
        Expression::ParallelDynamic { functions, .. }
        | Expression::Propagate {
            value: functions, ..
        }
        | Expression::Unary {
            operand: functions, ..
        }
        | Expression::Property {
            receiver: functions,
            ..
        } => rewrite_expression(functions, bindings),
        Expression::Binary { left, right, .. }
        | Expression::Index {
            receiver: left,
            index: right,
            ..
        } => {
            rewrite_expression(left, bindings);
            rewrite_expression(right, bindings);
        }
        Expression::Object { properties, .. } => {
            for property in properties {
                rewrite_expression(&mut property.value, bindings);
            }
        }
        Expression::Match { value, arms, .. } => {
            rewrite_expression(value, bindings);
            for arm in arms {
                if let crate::ast::MatchPattern::Variant { type_name, .. } = &mut arm.pattern {
                    rewrite_identifier(type_name, bindings);
                }
                match &mut arm.body {
                    crate::ast::MatchArmBody::Expression(value) => {
                        rewrite_expression(value, bindings);
                    }
                    crate::ast::MatchArmBody::Block(block) => rewrite_block(block, bindings),
                }
            }
        }
        Expression::Variable(identifier) => {
            if identifier.name.contains("::") {
                rewrite_identifier(identifier, bindings);
            }
        }
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bytes(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Unit(_) => {}
    }
}

/// Replaces an identifier with its canonical binding where one exists.
fn rewrite_identifier(identifier: &mut Identifier<'_>, bindings: &HashMap<String, String>) {
    if let Some(name) = bindings.get(&identifier.name) {
        identifier.name = name.clone();
    } else if let Some((prefix, member)) = identifier.name.rsplit_once("::")
        && let Some(prefix) = bindings.get(prefix)
    {
        identifier.name = format!("{prefix}::{member}");
    }
}

/// Renames a declaration to its unique linked symbol key.
fn rename(identifier: &mut Identifier<'_>, index: usize) {
    if index != 0 {
        identifier.name = canonical(index, &identifier.name);
    }
}

/// Renames a type, enum or trait declaration to its unique linked symbol key.
fn rename_nominal(identifier: &mut Identifier<'_>, index: usize) {
    identifier.name = nominal_symbol(index, &identifier.name);
}

/// Builds the linker key of one type, enum or trait: the root source's declarations keep their
/// names unless they take the place of a prelude type.
fn nominal_symbol(index: usize, name: &str) -> String {
    if index == 0 && crate::prelude::is_shadowable(name) {
        format!("$module0::{name}")
    } else {
        canonical(index, name)
    }
}

/// Builds a private linker key for one declaration in an imported source.
fn canonical(index: usize, name: &str) -> String {
    if index == 0 {
        name.to_owned()
    } else {
        format!("$module{index}::{name}")
    }
}

/// Checks the restricted identifier shape accepted for a default namespace.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(character) if character == '_' || character.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Validates namespaces whose legality is specific to executable module linking.
fn validate_import_names(project: &LoadedProject) -> Result<(), CompileError> {
    for (source_index, edges) in project.edges().iter().enumerate() {
        for edge in edges {
            if !is_identifier(&edge.namespace) {
                let file = &project.files()[source_index];
                return Err(CompileError::at(
                    "E0233",
                    SourceSpan {
                        source_id: &file.source_id,
                        start_byte: edge.start_byte,
                        end_byte: edge.end_byte,
                    },
                    &file.text,
                    format!(
                        "import namespace `{}` is not a valid identifier; use `as`",
                        edge.namespace
                    ),
                ));
            }
        }
    }
    Ok(())
}
