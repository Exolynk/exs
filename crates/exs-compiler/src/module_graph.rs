//! Resolver-owned source graph loading and module-name resolution.

use std::collections::{HashMap, HashSet};

use crate::ast::{
    BinaryOperator, Block, Expression, FunctionDeclaration, Identifier, Module, Parameter,
    Statement, TestDeclaration, TypeAnnotation,
};
use crate::{
    CompileOptions, CompiledModule, CompiledTest, CompiledTests, ModuleResolver, SourceInput,
};

/// Compiler-private name assigned to a production root entry while testing.
const PROGRAM_MAIN: &str = "$exs_program_main";

/// Compiler-private name assigned to the generated test dispatcher entry.
const TEST_MAIN: &str = "main";

/// One source file retained for the lifetime of graph compilation.
struct SourceFile {
    /// Canonical source identity.
    source_id: String,
    /// Complete UTF-8 source text.
    text: String,
}

/// Compiles a resolved source graph into one linked executable module.
pub(super) fn compile<R: ModuleResolver>(
    source: SourceInput<'_>,
    options: CompileOptions,
    resolver: &mut R,
) -> Result<CompiledModule, String> {
    compile_target(source, options, resolver, false).map(|(module, _)| module)
}

/// Compiles one resolved source graph with an internal test dispatcher entry point.
pub(super) fn compile_tests<R: ModuleResolver>(
    source: SourceInput<'_>,
    options: CompileOptions,
    resolver: &mut R,
) -> Result<CompiledTests, String> {
    let (module, tests) = compile_target(source, options, resolver, true)?;
    Ok(CompiledTests {
        wasm: module.wasm,
        tests,
    })
}

/// Compiles one source graph for either production or individual source-test execution.
fn compile_target<R: ModuleResolver>(
    source: SourceInput<'_>,
    options: CompileOptions,
    resolver: &mut R,
    test_target: bool,
) -> Result<(CompiledModule, Vec<CompiledTest>), String> {
    let mut files = vec![SourceFile {
        source_id: source.source_id.to_owned(),
        text: source.text.to_owned(),
    }];
    let mut edges: Vec<Vec<(String, usize)>> = vec![Vec::new()];
    let mut indices = HashMap::from([(source.source_id.to_owned(), 0_usize)]);
    let mut index = 0;
    while index < files.len() {
        let source_id = files[index].source_id.clone();
        let text = files[index].text.clone();
        let lexed = crate::lexer::lex(SourceInput {
            source_id: &source_id,
            text: &text,
        });
        if !lexed.diagnostics.is_empty() {
            return Err(lexed.diagnostics.render(&text));
        }
        let parsed = crate::parser::parse(&source_id, lexed.tokens, false)
            .map_err(|error| error.render(&text))?;
        let imports = parsed.imports;
        for import in imports {
            let resolved = resolver
                .resolve(&source_id, &import.path)
                .map_err(|error| {
                    format!(
                        "{}:{}-{}: could not resolve import `{}`: {error}",
                        source_id, import.span.start_byte, import.span.end_byte, import.path
                    )
                })?;
            let next = if let Some(existing) = indices.get(&resolved.source_id) {
                *existing
            } else {
                let next = files.len();
                indices.insert(resolved.source_id.clone(), next);
                files.push(SourceFile {
                    source_id: resolved.source_id,
                    text: resolved.text,
                });
                edges.push(Vec::new());
                next
            };
            let alias = import
                .alias
                .map_or_else(|| default_namespace(&import.path), |alias| alias.name);
            if !is_identifier(&alias) {
                return Err(format!(
                    "{}:{}-{}: import namespace `{alias}` is not a valid identifier; use `as`",
                    source_id, import.span.start_byte, import.span.end_byte
                ));
            }
            edges[index].push((alias, next));
        }
        index += 1;
    }
    if let Some(cycle) = find_cycle(&edges) {
        let names = cycle
            .iter()
            .map(|index| files[*index].source_id.as_str())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(format!("{}: import cycle: {names}", files[0].source_id));
    }
    let mut modules = Vec::new();
    for file in &files {
        let lexed = crate::lexer::lex(SourceInput {
            source_id: &file.source_id,
            text: &file.text,
        });
        if !lexed.diagnostics.is_empty() {
            return Err(lexed.diagnostics.render(&file.text));
        }
        modules.push(
            crate::parser::parse(&file.source_id, lexed.tokens, false)
                .map_err(|error| error.render(&file.text))?,
        );
    }
    let mut exports = Vec::new();
    for (index, module) in modules.iter().enumerate() {
        if index != 0
            && module
                .functions
                .iter()
                .any(|function| function.name.name == "main")
        {
            return Err(format!(
                "{}: imported modules must not declare fn main()",
                files[index].source_id
            ));
        }
        exports.push(collect_exports(module, index)?);
    }
    let mut root_tests = if test_target {
        std::mem::take(&mut modules[0].tests)
    } else {
        Vec::new()
    };
    let tests = test_metadata(&root_tests, &files[0].source_id)?;
    let mut combined = Module {
        imports: Vec::new(),
        uses: Vec::new(),
        types: Vec::new(),
        enums: Vec::new(),
        traits: Vec::new(),
        implementations: Vec::new(),
        functions: Vec::new(),
        tests: Vec::new(),
    };
    for index in 0..modules.len() {
        let mut bindings = bindings_for(&modules[index], index, &edges[index], &exports)?;
        if test_target && index == 0 {
            if let Some(function) = modules[index]
                .functions
                .iter_mut()
                .find(|function| function.name.name == "main")
            {
                function.name.name = PROGRAM_MAIN.to_owned();
                bindings.insert("main".to_owned(), PROGRAM_MAIN.to_owned());
            }
            for test in &mut root_tests {
                rewrite_block(&mut test.body, &bindings);
            }
        }
        rewrite_module(&mut modules[index], index, &bindings);
        combined.types.append(&mut modules[index].types);
        combined.enums.append(&mut modules[index].enums);
        combined.traits.append(&mut modules[index].traits);
        combined
            .implementations
            .append(&mut modules[index].implementations);
        combined.functions.append(&mut modules[index].functions);
    }
    if test_target {
        append_test_functions(&mut combined, &root_tests)?;
    }
    let mut prelude = crate::prelude::parse().map_err(|error| error.to_string())?;
    prelude.types.append(&mut combined.types);
    prelude.enums.append(&mut combined.enums);
    prelude.traits.append(&mut combined.traits);
    prelude
        .implementations
        .append(&mut combined.implementations);
    prelude.functions.append(&mut combined.functions);
    combined = prelude;
    if combined
        .functions
        .iter()
        .filter(|function| function.name.name == "main")
        .count()
        != 1
    {
        return Err(format!(
            "{}: root module must declare exactly one fn main()",
            files[0].source_id
        ));
    }
    let mut source_inputs = crate::prelude::source_inputs();
    source_inputs.extend(files.iter().map(|file| SourceInput {
        source_id: &file.source_id,
        text: &file.text,
    }));
    let wasm = crate::codegen::compile_project_module(&mut combined, &source_inputs, options)
        .map_err(|error| error.render(&files[0].text))?;
    Ok((CompiledModule { wasm }, tests))
}

/// Validates source test names and returns runner-visible metadata in declaration order.
fn test_metadata(
    tests: &[TestDeclaration<'_>],
    source_id: &str,
) -> Result<Vec<CompiledTest>, String> {
    let mut names = HashSet::new();
    let mut metadata = Vec::with_capacity(tests.len());
    for test in tests {
        if test.description.is_empty() {
            return Err(format!("{source_id}: test descriptions must not be empty"));
        }
        if !names.insert(test.description.as_str()) {
            return Err(format!(
                "{source_id}: duplicate test description `{}`",
                test.description
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
) -> Result<(), String> {
    if tests.is_empty() {
        return Err("test source declares no tests".to_owned());
    }
    for (index, test) in tests.iter().enumerate() {
        module.functions.push(FunctionDeclaration {
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

/// Collects direct declarations that another module may import.
fn collect_exports(module: &Module<'_>, index: usize) -> Result<HashMap<String, String>, String> {
    let mut exports = HashMap::new();
    for name in module
        .functions
        .iter()
        .map(|item| &item.name.name)
        .chain(module.types.iter().map(|item| &item.name.name))
        .chain(module.enums.iter().map(|item| &item.name.name))
        .chain(module.traits.iter().map(|item| &item.name.name))
    {
        if name == "Self" {
            return Err("`Self` is reserved for trait type annotations".to_owned());
        }
        if matches!(name.as_str(), "assert" | "assert_eq") {
            return Err(format!("`{name}` is a reserved standard-library function"));
        }
        let canonical = canonical(index, name);
        if exports.insert(name.clone(), canonical).is_some() {
            return Err(format!("duplicate exported declaration `{name}`"));
        }
    }
    for declaration in &module.enums {
        for variant in &declaration.variants {
            let name = format!("{}::{}", declaration.name.name, variant.name.name);
            let canonical = canonical(index, &name);
            if exports.insert(name.clone(), canonical).is_some() {
                return Err(format!("duplicate exported declaration `{name}`"));
            }
        }
    }
    Ok(exports)
}

/// Builds canonical name lookups visible from one source module.
fn bindings_for(
    module: &Module<'_>,
    index: usize,
    edges: &[(String, usize)],
    exports: &[HashMap<String, String>],
) -> Result<HashMap<String, String>, String> {
    let mut bindings = collect_exports(module, index)?;
    let mut namespaces: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (alias, target) in edges {
        if alias == "std" {
            return Err("`std` is a reserved built-in type namespace".to_owned());
        }
        let names = namespaces.entry(alias).or_default();
        for (name, canonical) in &exports[*target] {
            if !names.insert(name) {
                return Err(format!(
                    "duplicate export `{name}` in merged namespace `{alias}`"
                ));
            }
            bindings.insert(format!("{alias}::{name}"), canonical.clone());
        }
    }
    for declaration in &module.uses {
        for item in &declaration.items {
            let source = format!("{}::{}", declaration.namespace.name, item.name.name);
            let Some(canonical) = bindings.get(&source).cloned() else {
                return Err(format!(
                    "{}: unknown import `{source}`",
                    declaration.span.source_id
                ));
            };
            let local = item
                .alias
                .as_ref()
                .map_or(&item.name.name, |alias| &alias.name);
            if bindings.insert(local.clone(), canonical).is_some() {
                return Err(format!(
                    "{}: used name `{local}` collides with an existing declaration",
                    declaration.span.source_id
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
                    return Err(format!(
                        "{}: used enum variant `{variant_local}` collides with an existing declaration",
                        declaration.span.source_id
                    ));
                }
            }
        }
    }
    Ok(bindings)
}

/// Rewrites declared and referenced top-level symbols to linker-stable canonical names.
fn rewrite_module(module: &mut Module<'_>, index: usize, bindings: &HashMap<String, String>) {
    for declaration in &mut module.types {
        rename(&mut declaration.name, index);
        for field in &mut declaration.fields {
            if let Some(annotation) = &mut field.type_annotation {
                rewrite_annotation(annotation, bindings);
            }
        }
    }
    for declaration in &mut module.enums {
        rename(&mut declaration.name, index);
        for variant in &mut declaration.variants {
            for field in &mut variant.fields {
                if let Some(annotation) = &mut field.type_annotation {
                    rewrite_annotation(annotation, bindings);
                }
            }
        }
    }
    for declaration in &mut module.traits {
        rename(&mut declaration.name, index);
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
        Statement::Let { value, .. }
        | Statement::Expression {
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
        Statement::For { iterable, body, .. } => {
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
            arguments,
            ..
        } => {
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
        Expression::ParallelDynamic { functions, .. }
        | Expression::IsError {
            value: functions, ..
        }
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
        Expression::Integer(_, _)
        | Expression::Float(_, _)
        | Expression::String(_, _)
        | Expression::Bool(_, _)
        | Expression::None(_)
        | Expression::Variable(_) => {}
    }
}

/// Replaces an identifier with its canonical binding where one exists.
fn rewrite_identifier(identifier: &mut Identifier<'_>, bindings: &HashMap<String, String>) {
    if let Some(name) = bindings.get(&identifier.name) {
        identifier.name = name.clone();
    } else if let Some((prefix, member)) = identifier.name.split_once("::")
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

/// Builds a private linker key for one declaration in an imported source.
fn canonical(index: usize, name: &str) -> String {
    if index == 0 {
        name.to_owned()
    } else {
        format!("$module{index}::{name}")
    }
}

/// Derives an import namespace from a relative `.exs` path.
fn default_namespace(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .strip_suffix(".exs")
        .unwrap_or(path)
        .to_owned()
}

/// Checks the restricted identifier shape accepted for a default namespace.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(character) if character == '_' || character.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Finds one directed cycle in an import graph.
fn find_cycle(edges: &[Vec<(String, usize)>]) -> Option<Vec<usize>> {
    fn visit(
        node: usize,
        edges: &[Vec<(String, usize)>],
        states: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        states[node] = 1;
        stack.push(node);
        for (_, next) in &edges[node] {
            if states[*next] == 1 {
                let start = stack.iter().position(|item| item == next)?;
                let mut cycle = stack[start..].to_vec();
                cycle.push(*next);
                return Some(cycle);
            }
            if states[*next] == 0
                && let Some(cycle) = visit(*next, edges, states, stack)
            {
                return Some(cycle);
            }
        }
        stack.pop();
        states[node] = 2;
        None
    }
    let mut states = vec![0; edges.len()];
    let mut stack = Vec::new();
    for node in 0..edges.len() {
        if states[node] == 0
            && let Some(cycle) = visit(node, edges, &mut states, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}
