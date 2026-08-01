//! Canonical source rendering for parsed ExS modules.

use crate::SourceInput;
use crate::ast::{
    AssignmentTarget, BinaryOperator, Block, EnumDeclaration, Expression, FunctionDeclaration,
    Identifier, ImplDeclaration, Module, ObjectProperty, Parameter, Statement, TraitDeclaration,
    TraitMethodDeclaration, TypeAnnotation, TypeDeclaration, UnaryOperator,
};
use crate::diagnostic::CompileDiagnostics;

/// Formats one lexically and syntactically valid ExS source unit.
pub(super) fn format<'a>(source: SourceInput<'a>) -> Result<String, CompileDiagnostics<'a>> {
    let lexed = crate::lexer::lex(source);
    let mut diagnostics = lexed.diagnostics;
    let module = match crate::parser::parse(source.source_id, lexed.tokens, false) {
        Ok(module) => module,
        Err(parser_diagnostics) => {
            diagnostics.extend(parser_diagnostics);
            diagnostics.sort_by_span();
            return Err(diagnostics);
        }
    };
    if !diagnostics.is_empty() {
        diagnostics.sort_by_span();
        return Err(diagnostics);
    }
    Ok(Formatter::new().module(&module))
}

/// Stateful canonical ExS source writer.
struct Formatter {
    /// Generated source text.
    output: String,
    /// Current indentation level in four-space units.
    indentation: usize,
}

impl Formatter {
    /// Creates an empty canonical source writer.
    fn new() -> Self {
        Self {
            output: String::new(),
            indentation: 0,
        }
    }

    /// Renders one complete parsed source module.
    fn module(mut self, module: &Module<'_>) -> String {
        for import in &module.imports {
            self.line(&format!(
                "import {}{};",
                quote_string(&import.path),
                import
                    .alias
                    .as_ref()
                    .map_or_else(String::new, |alias| format!(" as {}", alias.name))
            ));
        }
        for declaration in &module.uses {
            let items = declaration
                .items
                .iter()
                .map(|item| {
                    item.alias.as_ref().map_or_else(
                        || item.name.name.clone(),
                        |alias| format!("{} as {}", item.name.name, alias.name),
                    )
                })
                .collect::<Vec<_>>();
            let suffix = if items.len() == 1 {
                items[0].clone()
            } else {
                format!("{{{}}}", items.join(", "))
            };
            self.line(&format!("use {}::{};", declaration.namespace.name, suffix));
        }
        let has_prelude = !module.imports.is_empty() || !module.uses.is_empty();
        let declarations = module
            .types
            .iter()
            .map(Declaration::Type)
            .chain(module.enums.iter().map(Declaration::Enum))
            .chain(module.traits.iter().map(Declaration::Trait))
            .chain(module.implementations.iter().map(Declaration::Impl))
            .chain(module.functions.iter().map(Declaration::Function))
            .collect::<Vec<_>>();
        if has_prelude && !declarations.is_empty() {
            self.blank_line();
        }
        for (index, declaration) in declarations.iter().enumerate() {
            if index > 0 {
                self.blank_line();
            }
            match declaration {
                Declaration::Type(declaration) => self.type_declaration(declaration),
                Declaration::Enum(declaration) => self.enum_declaration(declaration),
                Declaration::Trait(declaration) => self.trait_declaration(declaration),
                Declaration::Impl(declaration) => self.implementation(declaration),
                Declaration::Function(declaration) => self.function(declaration),
            }
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.output
    }

    /// Renders one nominal enum declaration.
    fn enum_declaration(&mut self, declaration: &EnumDeclaration<'_>) {
        if declaration.variants.is_empty() {
            self.line(&format!("enum {} {{}}", declaration.name.name));
            return;
        }
        self.line(&format!("enum {} {{", declaration.name.name));
        self.indentation += 1;
        for variant in &declaration.variants {
            let fields = variant
                .fields
                .iter()
                .map(|field| {
                    field.type_annotation.as_ref().map_or_else(
                        || field.name.name.clone(),
                        |annotation| {
                            format!("{}: {}", field.name.name, type_annotation(annotation))
                        },
                    )
                })
                .collect::<Vec<_>>();
            let suffix = if fields.is_empty() {
                String::new()
            } else {
                format!("({})", fields.join(", "))
            };
            self.line(&format!("{}{},", variant.name.name, suffix));
        }
        self.indentation -= 1;
        self.line("}");
    }

    /// Renders one nominal type declaration.
    fn type_declaration(&mut self, declaration: &TypeDeclaration<'_>) {
        if declaration.fields.is_empty() {
            self.line(&format!("type {} {{}}", declaration.name.name));
            return;
        }
        self.line(&format!("type {} {{", declaration.name.name));
        self.indentation += 1;
        for field in &declaration.fields {
            let annotation = field
                .type_annotation
                .as_ref()
                .map_or_else(String::new, |value| format!(": {}", type_annotation(value)));
            self.line(&format!("{}{},", field.name.name, annotation));
        }
        self.indentation -= 1;
        self.line("}");
    }

    /// Renders one trait declaration.
    fn trait_declaration(&mut self, declaration: &TraitDeclaration<'_>) {
        self.line(&format!("trait {} {{", declaration.name.name));
        self.indentation += 1;
        for (index, method) in declaration.methods.iter().enumerate() {
            if index > 0 {
                self.blank_line();
            }
            self.trait_method(method);
        }
        self.indentation -= 1;
        self.line("}");
    }

    /// Renders one trait signature or default method body.
    fn trait_method(&mut self, declaration: &TraitMethodDeclaration<'_>) {
        let header = function_header(
            &declaration.name,
            &declaration.parameters,
            declaration.return_type.as_ref(),
        );
        if let Some(body) = &declaration.body {
            self.block_after(&header, body);
        } else {
            self.line(&format!("{header};"));
        }
    }

    /// Renders one implementation declaration.
    fn implementation(&mut self, declaration: &ImplDeclaration<'_>) {
        let header = declaration.trait_name.as_ref().map_or_else(
            || format!("impl {}", declaration.type_name.name),
            |trait_name| {
                format!(
                    "impl {} for {}",
                    trait_name.name, declaration.type_name.name
                )
            },
        );
        self.line(&format!("{header} {{"));
        self.indentation += 1;
        for (index, method) in declaration.methods.iter().enumerate() {
            if index > 0 {
                self.blank_line();
            }
            self.function(method);
        }
        self.indentation -= 1;
        self.line("}");
    }

    /// Renders one direct function declaration.
    fn function(&mut self, declaration: &FunctionDeclaration<'_>) {
        self.block_after(
            &function_header(
                &declaration.name,
                &declaration.parameters,
                declaration.return_type.as_ref(),
            ),
            &declaration.body,
        );
    }

    /// Renders one block after its already formatted opening header.
    fn block_after(&mut self, header: &str, block: &Block<'_>) {
        self.line(&format!("{header} {{"));
        self.indentation += 1;
        for statement in &block.statements {
            self.statement(statement);
        }
        self.indentation -= 1;
        self.line("}");
    }

    /// Renders one executable statement.
    fn statement(&mut self, statement: &Statement<'_>) {
        match statement {
            Statement::Let { name, value, .. } => self.line(&format!(
                "let {} = {};",
                name.name,
                render_expression(value)
            )),
            Statement::Assign { target, value, .. } => self.line(&format!(
                "{} = {};",
                assignment_target(target),
                render_expression(value)
            )),
            Statement::Return { value, .. } => self.line(&value.as_ref().map_or_else(
                || "ret;".to_owned(),
                |value| format!("ret {};", render_expression(value)),
            )),
            Statement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.block_after(&format!("if {}", render_expression(condition)), then_block);
                if let Some(else_block) = else_block {
                    self.line("else {");
                    self.indentation += 1;
                    for statement in &else_block.statements {
                        self.statement(statement);
                    }
                    self.indentation -= 1;
                    self.line("}");
                }
            }
            Statement::While {
                condition, body, ..
            } => self.block_after(&format!("while {}", render_expression(condition)), body),
            Statement::For {
                binding,
                iterable,
                body,
                ..
            } => self.block_after(
                &format!("for {} in {}", binding.name, render_expression(iterable)),
                body,
            ),
            Statement::Break { .. } => self.line("break;"),
            Statement::Continue { .. } => self.line("continue;"),
            Statement::Expression {
                expression: value, ..
            } => self.line(&format!("{};", render_expression(value))),
        }
    }

    /// Appends one indented line and a trailing line feed.
    fn line(&mut self, text: &str) {
        self.indentation_line(text);
        self.output.push('\n');
    }

    /// Appends an indentation-prefixed line without a trailing line feed.
    fn indentation_line(&mut self, text: &str) {
        for _ in 0..self.indentation {
            self.output.push_str("    ");
        }
        self.output.push_str(text);
    }

    /// Inserts one blank line unless the output already ends in one.
    fn blank_line(&mut self) {
        if !self.output.ends_with("\n\n") {
            self.output.push('\n');
        }
    }
}

/// A module declaration viewed without allocating a second AST representation.
enum Declaration<'a> {
    /// Nominal object type.
    Type(&'a TypeDeclaration<'a>),
    /// Nominal enum declaration.
    Enum(&'a EnumDeclaration<'a>),
    /// Trait declaration.
    Trait(&'a TraitDeclaration<'a>),
    /// Implementation declaration.
    Impl(&'a ImplDeclaration<'a>),
    /// Direct function declaration.
    Function(&'a FunctionDeclaration<'a>),
}

/// Renders one direct function header without its body.
fn function_header(
    name: &Identifier<'_>,
    parameters: &[Parameter<'_>],
    return_type: Option<&TypeAnnotation<'_>>,
) -> String {
    let parameters = parameters
        .iter()
        .map(|parameter| {
            parameter.type_annotation.as_ref().map_or_else(
                || parameter.name.name.clone(),
                |annotation| format!("{}: {}", parameter.name.name, type_annotation(annotation)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let return_type = return_type.map_or_else(String::new, |annotation| {
        format!(" -> {}", type_annotation(annotation))
    });
    format!("fn {}({parameters}){return_type}", name.name)
}

/// Renders one union type annotation.
fn type_annotation(annotation: &TypeAnnotation<'_>) -> String {
    annotation
        .members
        .iter()
        .map(|member| member.name.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Renders one source assignment target.
fn assignment_target(target: &AssignmentTarget<'_>) -> String {
    match target {
        AssignmentTarget::Variable(identifier) => identifier.name.clone(),
        AssignmentTarget::Index {
            receiver, index, ..
        } => format!(
            "{}[{}]",
            render_expression(receiver),
            render_expression(index)
        ),
        AssignmentTarget::Property {
            receiver, property, ..
        } => format!("{}.{}", render_expression(receiver), property.name),
    }
}

/// Renders one source expression with canonical parentheses where precedence requires them.
fn render_expression(expression: &Expression<'_>) -> String {
    expression_at(expression, 0)
}

/// Renders one expression inside a parent precedence context.
fn expression_at(expression: &Expression<'_>, parent_precedence: u8) -> String {
    let precedence = expression_precedence(expression);
    let value = match expression {
        Expression::Integer(value, _) => value.to_string(),
        Expression::Float(value, _) => {
            if value.fract() == 0.0 {
                format!("{value:.1}")
            } else {
                value.to_string()
            }
        }
        Expression::String(value, _) => quote_string(value),
        Expression::Bool(value, _) => value.to_string(),
        Expression::None(_) => "None".to_owned(),
        Expression::Variable(identifier) => identifier.name.clone(),
        Expression::IsError { value, .. } => {
            format!("{} is Error", expression_at(value, precedence))
        }
        Expression::Propagate { value, .. } => format!("{}?", expression_at(value, precedence)),
        Expression::List { elements, .. } => format!("[{}]", expressions(elements)),
        Expression::Object { properties, .. } => format!("{{{}}}", render_properties(properties)),
        Expression::TypedObject {
            type_name,
            properties: values,
            ..
        } => format!("{} {{{}}}", type_name.name, render_properties(values)),
        Expression::Match { value, arms, .. } => format!(
            "match {} {{ {} }}",
            render_expression(value),
            arms.iter()
                .map(|arm| format!(
                    "{} => {}",
                    render_match_pattern(&arm.pattern),
                    render_expression(&arm.value)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Expression::Closure {
            parameters, body, ..
        } => format!(
            "({}) => {}",
            render_parameters(parameters),
            inline_block(body)
        ),
        Expression::ParallelStatic { tasks, .. } => format!(
            "par {{ {} }}",
            tasks.iter().map(par_task).collect::<Vec<_>>().join(" ")
        ),
        Expression::ParallelDynamic { functions, .. } => {
            format!("par({})", render_expression(functions))
        }
        Expression::Unary {
            operator, operand, ..
        } => format!(
            "{}{}",
            unary_operator(*operator),
            expression_at(operand, precedence)
        ),
        Expression::Binary {
            operator,
            left,
            right,
            ..
        } => format!(
            "{} {} {}",
            expression_at(left, precedence),
            binary_operator(*operator),
            expression_at(right, precedence + 1)
        ),
        Expression::Call {
            callee, arguments, ..
        } => format!("{}({})", callee.name, expressions(arguments)),
        Expression::HostCall {
            name, arguments, ..
        } => {
            let mut values = vec![render_expression(name)];
            values.extend(arguments.iter().map(render_expression));
            format!("host.call({})", values.join(", "))
        }
        Expression::MethodCall {
            receiver,
            method,
            arguments,
            ..
        } => format!(
            "{}.{}({})",
            expression_at(receiver, precedence),
            method.name,
            expressions(arguments)
        ),
        Expression::StaticMethodCall {
            type_name,
            method,
            arguments,
            ..
        } => format!(
            "{}::{}({})",
            type_name.name,
            method.name,
            expressions(arguments)
        ),
        Expression::Index {
            receiver, index, ..
        } => format!(
            "{}[{}]",
            expression_at(receiver, precedence),
            render_expression(index)
        ),
        Expression::Property {
            receiver, property, ..
        } => format!("{}.{}", expression_at(receiver, precedence), property.name),
    };
    if precedence < parent_precedence {
        format!("({value})")
    } else {
        value
    }
}

/// Renders one enum-variant or fallback match pattern.
fn render_match_pattern(pattern: &crate::ast::MatchPattern<'_>) -> String {
    match pattern {
        crate::ast::MatchPattern::Variant {
            type_name,
            variant,
            bindings,
            ..
        } if bindings.is_empty() => format!("{}::{}", type_name.name, variant.name),
        crate::ast::MatchPattern::Variant {
            type_name,
            variant,
            bindings,
            ..
        } => format!(
            "{}::{}({})",
            type_name.name,
            variant.name,
            bindings
                .iter()
                .map(|binding| binding.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        crate::ast::MatchPattern::Wildcard(_) => "_".to_owned(),
    }
}

/// Renders one closure task from the parser's synthesized static parallel representation.
fn par_task(task: &Expression<'_>) -> String {
    if let Expression::Closure { body, .. } = task
        && let [
            Statement::Return {
                value: Some(value), ..
            },
        ] = body.statements.as_slice()
    {
        return format!("{};", render_expression(value));
    }
    format!("{};", render_expression(task))
}

/// Renders a compact block used in a closure expression.
fn inline_block(block: &Block<'_>) -> String {
    let statements = block
        .statements
        .iter()
        .map(|statement| match statement {
            Statement::Let { name, value, .. } => {
                format!("let {} = {};", name.name, render_expression(value))
            }
            Statement::Assign { target, value, .. } => format!(
                "{} = {};",
                assignment_target(target),
                render_expression(value)
            ),
            Statement::Return { value, .. } => value.as_ref().map_or_else(
                || "ret;".to_owned(),
                |value| format!("ret {};", render_expression(value)),
            ),
            Statement::Expression { expression, .. } => {
                format!("{};", render_expression(expression))
            }
            _ => expression_from_statement(statement),
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("{{ {statements} }}")
}

/// Renders compound closure statements through a temporary canonical statement fragment.
fn expression_from_statement(statement: &Statement<'_>) -> String {
    match statement {
        Statement::If {
            condition,
            then_block,
            else_block,
            ..
        } => format!(
            "if {} {}{}",
            render_expression(condition),
            inline_block(then_block),
            else_block
                .as_ref()
                .map_or_else(String::new, |block| format!(
                    " else {}",
                    inline_block(block)
                ))
        ),
        Statement::While {
            condition, body, ..
        } => format!(
            "while {} {}",
            render_expression(condition),
            inline_block(body)
        ),
        Statement::For {
            binding,
            iterable,
            body,
            ..
        } => format!(
            "for {} in {} {}",
            binding.name,
            render_expression(iterable),
            inline_block(body)
        ),
        Statement::Break { .. } => "break;".to_owned(),
        Statement::Continue { .. } => "continue;".to_owned(),
        Statement::Let { .. }
        | Statement::Assign { .. }
        | Statement::Return { .. }
        | Statement::Expression { .. } => String::new(),
    }
}

/// Renders one parameter sequence.
fn render_parameters(parameters: &[Parameter<'_>]) -> String {
    parameters
        .iter()
        .map(|parameter| {
            parameter.type_annotation.as_ref().map_or_else(
                || parameter.name.name.clone(),
                |annotation| format!("{}: {}", parameter.name.name, type_annotation(annotation)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a comma-separated expression sequence.
fn expressions(expressions: &[Expression<'_>]) -> String {
    expressions
        .iter()
        .map(render_expression)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a comma-separated object property sequence.
fn render_properties(properties: &[ObjectProperty<'_>]) -> String {
    properties
        .iter()
        .map(|property| {
            format!(
                "{}: {}",
                object_key(&property.key),
                render_expression(&property.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Renders a property key without turning valid identifiers into quoted strings.
fn object_key(key: &str) -> String {
    if is_identifier(key) {
        key.to_owned()
    } else {
        quote_string(key)
    }
}

/// Renders one canonical escaped source string.
fn quote_string(value: &str) -> String {
    format!("{value:?}")
}

/// Tests whether a property key may use bare identifier syntax.
fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(character) if character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Returns the binding precedence for one expression.
fn expression_precedence(expression: &Expression<'_>) -> u8 {
    match expression {
        Expression::Binary { operator, .. } => match operator {
            BinaryOperator::Or => 1,
            BinaryOperator::And => 2,
            BinaryOperator::Equal | BinaryOperator::NotEqual => 3,
            BinaryOperator::LessThan
            | BinaryOperator::LessOrEqual
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterOrEqual => 4,
            BinaryOperator::Add | BinaryOperator::Subtract => 5,
            BinaryOperator::Multiply => 6,
        },
        Expression::IsError { .. } => 4,
        Expression::Unary { .. } => 7,
        Expression::Propagate { .. }
        | Expression::Call { .. }
        | Expression::MethodCall { .. }
        | Expression::StaticMethodCall { .. }
        | Expression::Index { .. }
        | Expression::Property { .. } => 8,
        _ => 9,
    }
}

/// Renders one unary operator spelling.
fn unary_operator(operator: UnaryOperator) -> &'static str {
    match operator {
        UnaryOperator::Negate => "-",
        UnaryOperator::Not => "!",
    }
}

/// Renders one binary operator spelling.
fn binary_operator(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Equal => "==",
        BinaryOperator::NotEqual => "!=",
        BinaryOperator::LessThan => "<",
        BinaryOperator::LessOrEqual => "<=",
        BinaryOperator::GreaterThan => ">",
        BinaryOperator::GreaterOrEqual => ">=",
        BinaryOperator::And => "&&",
        BinaryOperator::Or => "||",
    }
}
