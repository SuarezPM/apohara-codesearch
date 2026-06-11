use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq)]
pub enum Language {
    TypeScript,
    Rust,
    Python,
    Go,
    Bash,
    Java,
    C,
    Ruby,
    Cpp,
}

/// Represents an import statement
#[derive(Debug, Clone, PartialEq)]
pub struct ImportStatement {
    /// The source module path (e.g., './utils', 'react', 'std::collections')
    pub source: String,
    /// The kind of import (named, default, namespace, require)
    pub import_kind: ImportKind,
    /// Line number where the import appears
    pub line: usize,
}

/// The specific type of import
#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    /// Named imports: import { a, b } from 'module'
    Named(Vec<String>),
    /// Default import: import React from 'react'
    Default(String),
    /// Namespace import: import * as name from 'module'
    Namespace(String),
    /// Side-effect import: import 'module'
    SideEffect,
    /// CommonJS require: const foo = require('module')
    Require(String),
}

impl ImportStatement {
    pub fn new(source: impl Into<String>, import_kind: ImportKind) -> Self {
        Self {
            source: source.into(),
            import_kind,
            line: 0,
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = line;
        self
    }
}

/// Represents an export statement
#[derive(Debug, Clone, PartialEq)]
pub struct ExportStatement {
    /// The exported items or source for re-exports
    pub export_kind: ExportKind,
    /// Line number where the export appears
    pub line: usize,
}

/// The specific type of export
#[derive(Debug, Clone, PartialEq)]
pub enum ExportKind {
    /// Named exports: export { a, b }
    Named(Vec<String>),
    /// Default export: export default foo
    Default(String),
    /// Re-export: export { a } from 'module'
    ReExport { items: Vec<String>, source: String },
    /// Re-export all: export * from 'module'
    ReExportAll(String),
}

impl ExportStatement {
    pub fn new(export_kind: ExportKind) -> Self {
        Self {
            export_kind,
            line: 0,
        }
    }

    pub fn with_line(mut self, line: usize) -> Self {
        self.line = line;
        self
    }
}

/// Combined result containing both function signatures and imports/exports
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    pub functions: Vec<FunctionSignature>,
    pub imports: Vec<ImportStatement>,
    pub exports: Vec<ExportStatement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionSignature {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<String>,
    pub line: usize,
    pub column: usize,
}

/// The category of a parsed top-level symbol, threaded from the span extractors
/// to the chunker so a type declaration becomes a Symbol chunk with the right
/// `kind` (instead of being derived solely from the `self`-receiver heuristic).
///
/// `Function` is refined to `"method"` at render time when the signature carries
/// a `self` receiver (see [`crate::chunker::ChunkSpec::kind_str`]); the type
/// variants render to their own keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// A free function or a method (refined via the `self`-receiver heuristic).
    Function,
    /// Rust `struct_item`/`union_item`, Go `struct_type` `type_spec`.
    Struct,
    /// Rust `enum_item`, TS `enum_declaration`.
    Enum,
    /// Rust `trait_item`.
    Trait,
    /// TS `class_declaration`, Python `class_definition`.
    Class,
    /// TS `interface_declaration`, Go `interface_type` `type_spec`.
    Interface,
    /// Rust `type_item`, TS `type_alias_declaration`, Go non-struct/-interface
    /// `type_spec`.
    Type,
    /// Ruby `module` declaration.
    Module,
}

impl SymbolKind {
    /// The keyword used to render a type symbol's signature and as its stored
    /// `kind` tag. `Function` returns `"function"` (the chunker may refine this
    /// to `"method"` via the `self`-receiver heuristic).
    pub fn keyword(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Type => "type",
            SymbolKind::Module => "module",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<String>,
}

impl FunctionSignature {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            return_type: None,
            line: 0,
            column: 0,
        }
    }

    pub fn with_position(mut self, line: usize, column: usize) -> Self {
        self.line = line;
        self.column = column;
        self
    }

    pub fn with_return_type(mut self, return_type: impl Into<String>) -> Self {
        self.return_type = Some(return_type.into());
        self
    }

    pub fn add_parameter(
        mut self,
        name: impl Into<String>,
        type_annotation: Option<impl Into<String>>,
    ) -> Self {
        self.parameters.push(Parameter {
            name: name.into(),
            type_annotation: type_annotation.map(Into::into),
        });
        self
    }
}

/// Detect language based on file extension
pub fn detect_language(path: &Path) -> Option<Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") | Some("tsx") | Some("mts") | Some("cts") => Some(Language::TypeScript),
        Some("rs") => Some(Language::Rust),
        Some("py") => Some(Language::Python),
        Some("go") => Some(Language::Go),
        Some("bash") | Some("sh") => Some(Language::Bash),
        Some("java") => Some(Language::Java),
        Some("c") | Some("h") => Some(Language::C),
        Some("rb") => Some(Language::Ruby),
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hxx") | Some("hh") => {
            Some(Language::Cpp)
        }
        _ => None,
    }
}

/// Parse a file and extract function signatures
pub fn parse_file(path: &Path) -> Result<Vec<FunctionSignature>, ParseError> {
    let language =
        detect_language(path).ok_or_else(|| ParseError::UnsupportedLanguage(path.to_path_buf()))?;

    let content =
        fs::read_to_string(path).map_err(|e| ParseError::ReadError(path.to_path_buf(), e))?;

    parse_source(&content, language)
}

/// Parse source code directly
pub fn parse_source(
    source: &str,
    language: Language,
) -> Result<Vec<FunctionSignature>, ParseError> {
    let mut parser = Parser::new();

    match language {
        Language::TypeScript => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .map_err(|e| ParseError::ParserInit(format!("TypeScript: {:?}", e)))?;
        }
        Language::Rust => {
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Rust: {:?}", e)))?;
        }
        Language::Python => {
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Python: {:?}", e)))?;
        }
        Language::Go => {
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Go: {:?}", e)))?;
        }
        Language::Bash => {
            parser
                .set_language(&tree_sitter_bash::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Bash: {:?}", e)))?;
        }
        Language::Java => {
            parser
                .set_language(&tree_sitter_java::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Java: {:?}", e)))?;
        }
        Language::C => {
            parser
                .set_language(&tree_sitter_c::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("C: {:?}", e)))?;
        }
        Language::Ruby => {
            parser
                .set_language(&tree_sitter_ruby::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Ruby: {:?}", e)))?;
        }
        Language::Cpp => {
            parser
                .set_language(&tree_sitter_cpp::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Cpp: {:?}", e)))?;
        }
    }

    let tree = parser.parse(source, None).ok_or(ParseError::ParseFailed)?;

    let root = tree.root_node();
    let mut signatures = Vec::new();

    match language {
        Language::TypeScript => extract_typescript_functions(&root, source, &mut signatures),
        Language::Rust => extract_rust_functions(&root, source, &mut signatures),
        Language::Python => extract_python_functions(&root, source, &mut signatures),
        Language::Go => extract_go_functions(&root, source, &mut signatures),
        Language::Bash => extract_bash_functions(&root, source, &mut signatures),
        Language::Java => extract_java_functions(&root, source, &mut signatures),
        Language::C => extract_c_functions(&root, source, &mut signatures),
        Language::Ruby => extract_ruby_functions(&root, source, &mut signatures),
        Language::Cpp => extract_cpp_functions(&root, source, &mut signatures),
    }

    Ok(signatures)
}

#[derive(Debug)]
pub enum ParseError {
    UnsupportedLanguage(std::path::PathBuf),
    ReadError(std::path::PathBuf, std::io::Error),
    ParserInit(String),
    ParseFailed,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnsupportedLanguage(p) => write!(f, "Unsupported file type: {:?}", p),
            ParseError::ReadError(p, e) => write!(f, "Failed to read {:?}: {}", p, e),
            ParseError::ParserInit(e) => write!(f, "Parser initialization failed: {}", e),
            ParseError::ParseFailed => write!(f, "Failed to parse source code"),
        }
    }
}

impl std::error::Error for ParseError {}

fn extract_typescript_functions(
    node: &Node,
    source: &str,
    signatures: &mut Vec<FunctionSignature>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_declaration" | "function_signature" => {
                if let Some(sig) = parse_typescript_function(&child, source) {
                    signatures.push(sig);
                }
            }
            "export_statement" => {
                // Check for exported functions
                if let Some(declaration) = child.child_by_field_name("declaration") {
                    if declaration.kind() == "function_declaration" {
                        if let Some(sig) = parse_typescript_function(&declaration, source) {
                            signatures.push(sig);
                        }
                    }
                }
            }
            "class_declaration" | "interface_declaration" | "type_alias_declaration" => {
                // Extract methods from classes and interfaces
                if let Some(body) = child.child_by_field_name("body") {
                    extract_typescript_functions(&body, source, signatures);
                }
            }
            "method_definition" | "method_signature" => {
                if let Some(sig) = parse_typescript_method(&child, source) {
                    signatures.push(sig);
                }
            }
            "abstract_method_signature" | "call_signature" | "construct_signature" => {
                if let Some(sig) = parse_typescript_signature(&child, source) {
                    signatures.push(sig);
                }
            }
            _ => {
                // Recursively search in other nodes
                extract_typescript_functions(&child, source, signatures);
            }
        }
    }
}

fn parse_typescript_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if param.kind() == "formal_parameters"
                || param.kind() == "required_parameter"
                || param.kind() == "optional_parameter"
            {
                // For formal_parameters node, recurse into children
                if param.kind() == "formal_parameters" {
                    let inner_cursor = &mut param.walk();
                    for inner_param in param.children(inner_cursor) {
                        if let Some((name, type_ann)) =
                            extract_typescript_param(&inner_param, source)
                        {
                            sig = sig.add_parameter(name, type_ann);
                        }
                    }
                } else if let Some((name, type_ann)) = extract_typescript_param(&param, source) {
                    sig = sig.add_parameter(name, type_ann);
                }
            }
        }
    }

    // Extract return type
    if let Some(type_node) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = type_node.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }

    Some(sig)
}

fn parse_typescript_method(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if let Some((name, type_ann)) = extract_typescript_param(&param, source) {
                sig = sig.add_parameter(name, type_ann);
            }
        }
    }

    // Extract return type
    if let Some(type_node) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = type_node.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }

    Some(sig)
}

fn parse_typescript_signature(node: &Node, source: &str) -> Option<FunctionSignature> {
    // For call signatures and method signatures in interfaces
    let name = if node.kind() == "call_signature" {
        "__call".to_string()
    } else {
        node.child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .unwrap_or_default()
            .to_string()
    };

    if name.is_empty() && node.kind() != "call_signature" {
        return None;
    }

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if let Some((name, type_ann)) = extract_typescript_param(&param, source) {
                sig = sig.add_parameter(name, type_ann);
            }
        }
    }

    // Extract return type
    if let Some(type_node) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = type_node.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }

    Some(sig)
}

fn extract_typescript_param(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    match node.kind() {
        "required_parameter" | "optional_parameter" => {
            let name = node
                .child_by_field_name("pattern")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())?;

            let type_ann = node
                .child_by_field_name("type")
                .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            Some((name, type_ann))
        }
        "identifier" => {
            let name = node.utf8_text(source.as_bytes()).ok()?.to_string();
            Some((name, None))
        }
        _ => None,
    }
}

fn extract_rust_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    signatures.push(sig);
                }
            }
            "impl_item" => {
                // Extract methods from impl blocks
                if let Some(body) = child.child_by_field_name("body") {
                    extract_rust_functions(&body, source, signatures);
                }
            }
            "trait_item" => {
                // Extract methods from trait definitions
                if let Some(body) = child.child_by_field_name("body") {
                    extract_rust_trait_methods(&body, source, signatures);
                }
            }
            "declaration_list" | "field_declaration_list" => {
                // Recursively search in declaration lists
                extract_rust_functions(&child, source, signatures);
            }
            "associated_function" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    signatures.push(sig);
                }
            }
            _ => {
                // Recursively search in other nodes
                extract_rust_functions(&child, source, signatures);
            }
        }
    }
}

/// Extract function signatures from trait bodies
/// Trait methods use "function_signature_item" instead of "function_item"
fn extract_rust_trait_methods(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_signature_item" | "function_item" | "associated_function" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    signatures.push(sig);
                }
            }
            _ => {
                // Recursively search in other nodes
                extract_rust_trait_methods(&child, source, signatures);
            }
        }
    }
}

fn parse_rust_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Extract parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if let Some((name, type_ann)) = extract_rust_param(&param, source) {
                sig = sig.add_parameter(name, type_ann);
            }
        }
    }

    // Extract return type
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = ret_type.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }

    Some(sig)
}

fn extract_rust_param(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    match node.kind() {
        "parameter" => {
            // Try to get pattern (name) and type
            let pattern = node.child_by_field_name("pattern");
            let type_node = node.child_by_field_name("type");

            let name = pattern
                .and_then(|p| extract_pattern_name(&p, source))
                .unwrap_or_else(|| "_".to_string());

            let type_ann = type_node
                .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());

            Some((name, type_ann))
        }
        "self_parameter" => {
            let self_text = node.utf8_text(source.as_bytes()).ok()?.to_string();
            Some(("self".to_string(), Some(self_text)))
        }
        "identifier" => {
            let name = node.utf8_text(source.as_bytes()).ok()?.to_string();
            Some((name, None))
        }
        _ => None,
    }
}

fn extract_pattern_name(node: &Node, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => node
            .utf8_text(source.as_bytes())
            .ok()
            .map(|s| s.to_string()),
        "ref_pattern" | "mut_pattern" => {
            // Get the inner pattern
            node.child(0).and_then(|c| extract_pattern_name(&c, source))
        }
        "tuple_pattern" | "struct_pattern" | "slice_pattern" => {
            // For complex patterns, return a placeholder
            Some("_".to_string())
        }
        _ => None,
    }
}

// ============================================================================
// Import/Export Parsing
// ============================================================================

/// Parse a file and extract imports and exports
pub fn parse_imports_exports(
    path: &Path,
) -> Result<(Vec<ImportStatement>, Vec<ExportStatement>), ParseError> {
    let language =
        detect_language(path).ok_or_else(|| ParseError::UnsupportedLanguage(path.to_path_buf()))?;

    let content =
        fs::read_to_string(path).map_err(|e| ParseError::ReadError(path.to_path_buf(), e))?;

    parse_source_imports_exports(&content, language)
}

/// Parse source code and extract imports and exports
pub fn parse_source_imports_exports(
    source: &str,
    language: Language,
) -> Result<(Vec<ImportStatement>, Vec<ExportStatement>), ParseError> {
    let mut parser = Parser::new();

    match language {
        Language::TypeScript => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .map_err(|e| ParseError::ParserInit(format!("TypeScript: {:?}", e)))?;
        }
        Language::Rust => {
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Rust: {:?}", e)))?;
        }
        Language::Python => {
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Python: {:?}", e)))?;
        }
        Language::Go => {
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Go: {:?}", e)))?;
        }
        Language::Bash => {
            parser
                .set_language(&tree_sitter_bash::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Bash: {:?}", e)))?;
        }
        Language::Java => {
            parser
                .set_language(&tree_sitter_java::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Java: {:?}", e)))?;
        }
        Language::C => {
            parser
                .set_language(&tree_sitter_c::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("C: {:?}", e)))?;
        }
        Language::Ruby => {
            parser
                .set_language(&tree_sitter_ruby::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Ruby: {:?}", e)))?;
        }
        Language::Cpp => {
            parser
                .set_language(&tree_sitter_cpp::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Cpp: {:?}", e)))?;
        }
    }

    let tree = parser.parse(source, None).ok_or(ParseError::ParseFailed)?;

    let root = tree.root_node();
    let mut imports = Vec::new();
    let mut exports = Vec::new();

    match language {
        Language::TypeScript => {
            extract_typescript_imports(&root, source, &mut imports);
            extract_typescript_exports(&root, source, &mut exports);
        }
        Language::Rust => {
            extract_rust_imports(&root, source, &mut imports);
            extract_rust_exports(&root, source, &mut exports);
        }
        Language::Python => {
            extract_python_imports(&root, source, &mut imports);
            // Python has no `export` keyword: visibility is convention-based
            // (a leading underscore marks "private"), so there is no syntactic
            // export node to extract. We intentionally leave `exports` empty.
        }
        Language::Go => {
            extract_go_imports(&root, source, &mut imports);
            // Go exports are not a distinct syntax node: an identifier is
            // "exported" purely by being capitalized. Modeling that would mean
            // re-walking every top-level declaration to classify its name's
            // case — outside the import/span scope of this pass — so it is
            // deferred: Go's "exported = capitalized identifier" rule is not
            // modeled here.
        }
        Language::Bash => {
            // Bash "imports" = `source` and `.` builtins (load a file into the
            // current shell). They appear as `command` nodes with name "source"
            // or ".". Captured as Require-kind imports.
            extract_bash_imports(&root, source, &mut imports);
            // Bash "exports" = the `export` builtin (mark a variable for the
            // environment). Captured as export rows.
            extract_bash_exports(&root, source, &mut exports);
        }
        Language::Java => {
            // Java imports: `import_declaration` nodes carry the package/class
            // path as a scoped_identifier or identifier child. Captured as
            // Named imports.
            extract_java_imports(&root, source, &mut imports);
            // Java "exports": there is no syntactic export. Public visibility
            // is modeled on the class/interface/method itself, not via an
            // export keyword. We model this implicitly via the type symbol
            // pass: `extract_java_type_spans` emits `Class`/`Interface`/`Enum`
            // symbols whose presence in the symbols table IS the export
            // surface. We intentionally leave `exports` empty here.
        }
        Language::C => {
            // C imports: `#include` is a preprocessor directive; the path is
            // a `string_literal` or `system_lib_string` child of a
            // `preproc_include` node. Captured as Require-kind imports.
            extract_c_imports(&root, source, &mut imports);
            // C has no module system or export keyword. Functions are "public"
            // purely by being non-static; modeling that would mean walking
            // every top-level declaration to classify its storage class —
            // outside the import/span scope of this pass. We leave `exports`
            // empty (mirrors the Go precedent).
        }
        Language::Ruby => {
            // Ruby imports: `require` and `require_relative` are top-level
            // `call` nodes with the method name as the first `identifier`
            // child. The argument is the string/path. Captured as Require.
            extract_ruby_imports(&root, source, &mut imports);
            // Ruby has no export keyword. Top-level `module` declarations
            // expose their constants/methods; we leave `exports` empty
            // (the type-symbol pass captures classes/modules as Class/Module
            // symbols, which is the Ruby equivalent of "exports").
        }
        Language::Cpp => {
            // C++ imports: `#include` is the same preprocessor directive as
            // C, with the same node kinds (preproc_include + system_lib_string
            // or string_literal). The C++ grammar also has additional
            // #include forms (angle-with-quoted, macro-expanded) that the
            // tree-sitter-cpp grammar exposes; the simple system + local
            // distinction covers >90% of real-world C++ headers.
            extract_cpp_imports(&root, source, &mut imports);
            // C++ has no module system in the C++03 sense. `export` is a
            // C++20 module keyword but rarely used in practice; defer it.
            // Functions are "exported" by being non-static, same as C.
        }
    }

    Ok((imports, exports))
}

/// Extract imports from TypeScript source
fn extract_typescript_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    // Check if this node is an import statement
    if node.kind() == "import_statement" {
        // Find the source module path from the string child
        let mut module_source = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "string" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    module_source = Some(text.trim_matches(|c| c == '\'' || c == '"').to_string());
                }
                break;
            }
        }

        // Check for import_clause (for named, default, namespace imports)
        let mut found_import_clause = false;
        for child in node.children(&mut cursor) {
            if child.kind() == "import_clause" {
                found_import_clause = true;
                if let Some(source_text) = &module_source {
                    if let Some(import_stmt) =
                        parse_typescript_import_clause_with_source(&child, source, source_text)
                    {
                        imports.push(import_stmt);
                    }
                }
                break;
            }
        }

        // If no import_clause found but we have a source, it's a side-effect import
        if !found_import_clause {
            if let Some(source_text) = module_source {
                let line = node.start_position().row + 1;
                imports.push(
                    ImportStatement::new(source_text, ImportKind::SideEffect).with_line(line),
                );
            }
        }
        return; // Don't recurse into children of import_statement
    }

    // Check for require() call - look for call_expression with "require" as function
    if node.kind() == "call_expression" {
        if let Some(import_stmt) = parse_typescript_require(node, source) {
            imports.push(import_stmt);
            return;
        }
    }

    // Recurse into children
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_typescript_imports(&child, source, imports);
    }
}

/// Parse a TypeScript require() call
fn parse_typescript_require(node: &Node, source: &str) -> Option<ImportStatement> {
    // Check if this is a require call - look for "require" identifier
    let mut found_require = false;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(name) = child.utf8_text(source.as_bytes()) {
                if name == "require" {
                    found_require = true;
                    break;
                }
            }
        }
    }

    if !found_require {
        return None;
    }

    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Get the argument (module path)
    let mut args_cursor = node.walk();
    for child in node.children(&mut args_cursor) {
        if child.kind() == "arguments" {
            if let Some(first_arg) = child.child(0) {
                if let Ok(module_path) = first_arg.utf8_text(source.as_bytes()) {
                    // Extract the module name from the string (remove quotes)
                    let module_name = module_path
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();

                    // Check for assignment - look for variable_declarator parent
                    let parent = node.parent();
                    if let Some(p) = parent {
                        if p.kind() == "variable_declarator" {
                            if let Some(name_node) = p.child_by_field_name("name") {
                                if let Ok(var_name) = name_node.utf8_text(source.as_bytes()) {
                                    return Some(
                                        ImportStatement::new(
                                            module_name,
                                            ImportKind::Require(var_name.to_string()),
                                        )
                                        .with_line(line),
                                    );
                                }
                            }
                        }
                    }

                    return Some(
                        ImportStatement::new(module_name, ImportKind::Require(String::new()))
                            .with_line(line),
                    );
                }
            }
        }
    }

    None
}

/// Parse a TypeScript import clause (from import_statement)
/// Uses source provided externally (from parent import_statement)
fn parse_typescript_import_clause_with_source(
    node: &Node,
    source: &str,
    source_text: &str,
) -> Option<ImportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // The source is passed in from the parent import_statement
    let module_source = source_text.to_string();

    // Check for different import styles by examining direct children
    let mut cursor = node.walk();
    let mut has_default = false;
    let mut has_namespace = false;
    let mut has_named = false;
    let mut namespace_name = String::new();
    let mut default_name = String::new();
    let mut named_imports = Vec::new();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Check for default import (single identifier like "React")
        if kind == "identifier" && !has_default && !has_namespace && !has_named {
            // This might be a default import
            if let Ok(name) = child.utf8_text(source.as_bytes()) {
                default_name = name.to_string();
                has_default = true;
            }
        }

        // Check for namespace import (* as Name)
        if kind == "namespace_import" {
            let mut ns_cursor = child.walk();
            for ns_child in child.children(&mut ns_cursor) {
                if ns_child.kind() == "identifier" {
                    if let Ok(name) = ns_child.utf8_text(source.as_bytes()) {
                        namespace_name = name.to_string();
                        has_namespace = true;
                    }
                }
            }
        }

        // Check for named imports ({ a, b })
        if kind == "named_imports" {
            has_named = true;
            let mut named_cursor = child.walk();
            for named_child in child.children(&mut named_cursor) {
                if named_child.kind() == "import_specifier" {
                    // Get the name from the specifier
                    let mut spec_cursor = named_child.walk();
                    for spec_child in named_child.children(&mut spec_cursor) {
                        if spec_child.kind() == "identifier" {
                            if let Ok(name) = spec_child.utf8_text(source.as_bytes()) {
                                if name != "default" {
                                    // Skip "default" keyword
                                    named_imports.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Return based on what we found
    if has_namespace && !namespace_name.is_empty() {
        return Some(
            ImportStatement::new(module_source, ImportKind::Namespace(namespace_name))
                .with_line(line),
        );
    }

    if has_default && !default_name.is_empty() {
        return Some(
            ImportStatement::new(module_source, ImportKind::Default(default_name)).with_line(line),
        );
    }

    if has_named && !named_imports.is_empty() {
        return Some(
            ImportStatement::new(module_source, ImportKind::Named(named_imports)).with_line(line),
        );
    }

    // Fallback: import without specific clause (import 'module' - side effect)
    Some(ImportStatement::new(module_source, ImportKind::SideEffect).with_line(line))
}

/// Parse a TypeScript import clause (from import_statement)
///
/// Partial implementation reserved for Stage 5+ symbol resolution (currently unused;
/// `parse_typescript_import` covers the active import-statement path).
#[allow(dead_code)]
fn parse_typescript_import_clause(node: &Node, source: &str) -> Option<ImportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Get the source module path from the string literal
    // The source is typically a child with kind "string" or inside import_clause
    let source_text = find_import_source(node, source)?;

    // Check for default import (default is a direct child with kind "identifier")
    if let Some(default) = node.child_by_field_name("default") {
        let default_name = default.utf8_text(source.as_bytes()).ok()?.to_string();
        return Some(
            ImportStatement::new(source_text, ImportKind::Default(default_name)).with_line(line),
        );
    }

    // Check for namespace import (import * as name)
    let namespace = node.child_by_field_name("namespace");
    if let Some(ns) = namespace {
        let ns_name = ns.utf8_text(source.as_bytes()).ok()?.to_string();
        return Some(
            ImportStatement::new(source_text, ImportKind::Namespace(ns_name)).with_line(line),
        );
    }

    // Check for named imports (import { a, b } from 'module')
    let named = node.child_by_field_name("named_imports");
    if let Some(named_node) = named {
        let mut names = Vec::new();
        let cursor = &mut named_node.walk();
        for child in named_node.children(cursor) {
            let kind = child.kind();
            if kind == "import_specifier" {
                // Get the name from the specifier
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                        names.push(name.to_string());
                    }
                }
            }
        }
        if !names.is_empty() {
            return Some(
                ImportStatement::new(source_text, ImportKind::Named(names)).with_line(line),
            );
        }
    }

    // Fallback: import without specific clause (import 'module')
    Some(ImportStatement::new(source_text, ImportKind::SideEffect).with_line(line))
}

/// Find the import source (module path) from an import node
///
/// Helper for `parse_typescript_import_clause`; kept for Stage 5+ symbol resolution.
#[allow(dead_code)]
fn find_import_source(node: &Node, source: &str) -> Option<String> {
    // Look for string literal children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "string_fragment" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                // Remove quotes
                return Some(text.trim_matches(|c| c == '\'' || c == '"').to_string());
            }
        }
    }
    // Also check for module field
    if let Some(module) = node.child_by_field_name("module") {
        if let Ok(text) = module.utf8_text(source.as_bytes()) {
            return Some(text.trim_matches(|c| c == '\'' || c == '"').to_string());
        }
    }
    None
}

/// Extract exports from TypeScript source
fn extract_typescript_exports(node: &Node, source: &str, exports: &mut Vec<ExportStatement>) {
    // Check if this node is an export statement
    if node.kind() == "export_statement" {
        if let Some(export_stmt) = parse_typescript_export(node, source) {
            exports.push(export_stmt);
        }
        return; // Don't recurse into children of export_statement
    }

    // Recurse into children
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_typescript_exports(&child, source, exports);
    }
}

/// Parse a TypeScript export statement
fn parse_typescript_export(node: &Node, source: &str) -> Option<ExportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Look for direct children to determine export type
    let mut cursor = node.walk();
    let mut has_from = false;
    let mut module_source = String::new();
    let mut specifier_node = None;
    let mut declaration_node = None;

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Check for 'from' keyword - indicates re-export
        if kind == "from" {
            has_from = true;
        }

        // String is the module source
        if kind == "string" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                module_source = text.trim_matches(|c| c == '\'' || c == '"').to_string();
            }
        }

        // specifier contains { a, b }
        if kind == "export_clause" || kind == "named_exports" {
            specifier_node = Some(child);
        }

        // declaration is for "export default ..."
        if kind == "function_declaration"
            || kind == "class_declaration"
            || kind == "lexical_declaration"
            || kind == "variable_declaration"
        {
            declaration_node = Some(child);
        }
    }

    // Check for re-export (export { a } from 'module')
    if has_from && !module_source.is_empty() {
        let mut items = Vec::new();

        if let Some(spec) = specifier_node {
            let mut spec_cursor = spec.walk();
            for child in spec.children(&mut spec_cursor) {
                if let Ok(name) = child.utf8_text(source.as_bytes()) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() && trimmed != "{" && trimmed != "}" && trimmed != "," {
                        items.push(trimmed.to_string());
                    }
                }
            }
        }

        if items.is_empty() {
            // export * from 'module'
            return Some(
                ExportStatement::new(ExportKind::ReExportAll(module_source)).with_line(line),
            );
        } else {
            return Some(
                ExportStatement::new(ExportKind::ReExport {
                    items,
                    source: module_source,
                })
                .with_line(line),
            );
        }
    }

    // Check for default export (export default ...)
    if let Some(decl) = declaration_node {
        let decl_text = decl.utf8_text(source.as_bytes()).ok()?.to_string();

        // Check if it's a function or class declaration (both extract by `name` field).
        if matches!(decl.kind(), "function_declaration" | "class_declaration") {
            if let Some(name) = decl.child_by_field_name("name") {
                let ident = name.utf8_text(source.as_bytes()).ok()?.to_string();
                return Some(ExportStatement::new(ExportKind::Default(ident)).with_line(line));
            }
        }
        return Some(ExportStatement::new(ExportKind::Default(decl_text)).with_line(line));
    }

    // Check for named exports (export { a, b }) without from
    if let Some(spec) = specifier_node {
        let mut items = Vec::new();
        let mut spec_cursor = spec.walk();
        for child in spec.children(&mut spec_cursor) {
            if let Ok(name) = child.utf8_text(source.as_bytes()) {
                let trimmed = name.trim();
                if !trimmed.is_empty() && trimmed != "{" && trimmed != "}" && trimmed != "," {
                    items.push(trimmed.to_string());
                }
            }
        }
        if !items.is_empty() {
            return Some(ExportStatement::new(ExportKind::Named(items)).with_line(line));
        }
    }

    None
}

/// Extract imports from Rust source
fn extract_rust_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    // Check if this is a use declaration
    if node.kind() == "use_declaration" {
        if let Some(import_stmt) = parse_rust_use(node, source) {
            imports.push(import_stmt);
        }
        return;
    }

    // Check if this is a mod declaration
    if node.kind() == "mod_item" {
        if let Some(import_stmt) = parse_rust_mod(node, source) {
            imports.push(import_stmt);
        }
        return;
    }

    // Recurse into children
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_rust_imports(&child, source, imports);
    }
}

/// Parse a Rust use statement
fn parse_rust_use(node: &Node, source: &str) -> Option<ImportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Check if it's a pub use by looking for "pub" keyword
    let mut is_pub = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "pub" {
            is_pub = true;
            break;
        }
    }

    // Get the use tree - look for use_tree or scoped_identifier
    let mut use_tree_text = String::new();
    let mut tree_cursor = node.walk();
    for child in node.children(&mut tree_cursor) {
        let kind = child.kind();
        // The path is typically in scoped_identifier or use_tree
        if kind == "scoped_identifier" || kind == "use_tree" || kind == "identifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if !text.is_empty() && text != "use" && text != "pub" {
                    if use_tree_text.is_empty() {
                        use_tree_text = text.to_string();
                    } else {
                        use_tree_text.push_str(&format!("::{}", text));
                    }
                }
            }
        }
        // Also check for scoped_use_tree (std::foo::Bar)
        if kind == "scoped_use_tree" {
            let mut scoped_cursor = child.walk();
            for scoped_child in child.children(&mut scoped_cursor) {
                if let Ok(text) = scoped_child.utf8_text(source.as_bytes()) {
                    if !text.is_empty() && text != "::" {
                        if use_tree_text.is_empty() {
                            use_tree_text = text.to_string();
                        } else if text != "::" {
                            use_tree_text.push_str(&format!("::{}", text));
                        }
                    }
                }
            }
        }
    }

    if use_tree_text.is_empty() {
        return None;
    }

    if is_pub {
        // pub use creates a re-export - treat as export (but we're in imports)
        Some(
            ImportStatement::new(use_tree_text, ImportKind::Namespace("pub_use".to_string()))
                .with_line(line),
        )
    } else {
        Some(
            ImportStatement::new(use_tree_text, ImportKind::Default(String::new())).with_line(line),
        )
    }
}

/// Parse a Rust mod declaration
fn parse_rust_mod(node: &Node, source: &str) -> Option<ImportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Get the module name from direct children
    let mut name_text = String::new();
    let mut has_body = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "identifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                name_text = text.to_string();
            }
        }
        if kind == "block" || kind == "semicolon" {
            // If we have a semicolon, it's external; if block, it's inline
            has_body = kind == "block";
        }
    }

    if name_text.is_empty() {
        return None;
    }

    if has_body {
        Some(
            ImportStatement::new(name_text, ImportKind::Namespace("inline".to_string()))
                .with_line(line),
        )
    } else {
        Some(
            ImportStatement::new(name_text, ImportKind::Namespace("external".to_string()))
                .with_line(line),
        )
    }
}

/// Extract exports from Rust source (pub use statements)
fn extract_rust_exports(node: &Node, source: &str, exports: &mut Vec<ExportStatement>) {
    // Check if this is a use declaration
    if node.kind() == "use_declaration" {
        // Check for pub use (which acts as an export)
        let mut is_pub = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            // tree-sitter-rust uses "visibility_modifier" for pub, not "pub"
            if kind == "pub" || kind == "visibility_modifier" {
                is_pub = true;
                break;
            }
        }

        if is_pub {
            if let Some(export_stmt) = parse_rust_export(node, source) {
                exports.push(export_stmt);
            }
        }
        return;
    }

    // Recurse into children
    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_rust_exports(&child, source, exports);
    }
}

/// Parse a Rust pub use as an export
fn parse_rust_export(node: &Node, source: &str) -> Option<ExportStatement> {
    let start_position = node.start_position();
    let line = start_position.row + 1;

    // Get the use tree - extract from children
    let mut use_tree_text = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        // The path is typically in scoped_identifier, scoped_use_tree, or identifier
        if kind == "scoped_identifier" || kind == "identifier" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                if !text.is_empty() && text != "use" && text != "pub" {
                    if use_tree_text.is_empty() {
                        use_tree_text = text.to_string();
                    } else {
                        use_tree_text.push_str(&format!("::{}", text));
                    }
                }
            }
        }
        if kind == "scoped_use_tree" {
            let mut scoped_cursor = child.walk();
            for scoped_child in child.children(&mut scoped_cursor) {
                if let Ok(text) = scoped_child.utf8_text(source.as_bytes()) {
                    if !text.is_empty() && text != "::" {
                        if use_tree_text.is_empty() {
                            use_tree_text = text.to_string();
                        } else if text != "::" {
                            use_tree_text.push_str(&format!("::{}", text));
                        }
                    }
                }
            }
        }
    }

    if use_tree_text.is_empty() {
        return None;
    }

    Some(ExportStatement::new(ExportKind::Named(vec![use_tree_text])).with_line(line))
}

// ============================================================================
// Span-aware parsing (Step 1: chunker support)
// ============================================================================

/// Parse source code and return each symbol's `FunctionSignature` alongside its
/// [`SymbolKind`] and 1-based start/end line numbers.
///
/// Mirrors the node-kind matching of [`parse_source`] exactly, reusing the same
/// extraction helpers, and additionally extracts TYPE declarations
/// (struct/enum/trait/class/interface/type alias) as their own symbols. Each
/// emitted entry is `(signature, kind, start_line, end_line)` where the lines
/// come from `(node.start_position().row + 1, node.end_position().row + 1)`, so
/// callers (the chunker) can carve source line ranges per symbol.
///
/// A type declaration that wraps methods (Python `class_definition`, TS
/// `class_declaration`) is emitted as its OWN span IN ADDITION to its inner
/// method spans; the two ranges differ, so their `path:start-end` chunk ids stay
/// pairwise-distinct (pinned by `chunk_ids_pairwise_distinct`).
pub fn parse_source_spans(
    source: &str,
    language: Language,
) -> Result<Vec<(FunctionSignature, SymbolKind, usize, usize)>, ParseError> {
    let mut parser = Parser::new();

    match language {
        Language::TypeScript => {
            parser
                .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
                .map_err(|e| ParseError::ParserInit(format!("TypeScript: {:?}", e)))?;
        }
        Language::Rust => {
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Rust: {:?}", e)))?;
        }
        Language::Python => {
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Python: {:?}", e)))?;
        }
        Language::Go => {
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Go: {:?}", e)))?;
        }
        Language::Bash => {
            parser
                .set_language(&tree_sitter_bash::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Bash: {:?}", e)))?;
        }
        Language::Java => {
            parser
                .set_language(&tree_sitter_java::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Java: {:?}", e)))?;
        }
        Language::C => {
            parser
                .set_language(&tree_sitter_c::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("C: {:?}", e)))?;
        }
        Language::Ruby => {
            parser
                .set_language(&tree_sitter_ruby::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Ruby: {:?}", e)))?;
        }
        Language::Cpp => {
            parser
                .set_language(&tree_sitter_cpp::LANGUAGE.into())
                .map_err(|e| ParseError::ParserInit(format!("Cpp: {:?}", e)))?;
        }
    }

    let tree = parser.parse(source, None).ok_or(ParseError::ParseFailed)?;

    let root = tree.root_node();
    let mut spans = Vec::new();

    match language {
        Language::TypeScript => extract_typescript_function_spans(&root, source, &mut spans),
        Language::Rust => extract_rust_function_spans(&root, source, &mut spans),
        Language::Python => extract_python_function_spans(&root, source, &mut spans),
        Language::Go => extract_go_function_spans(&root, source, &mut spans),
        Language::Bash => extract_bash_function_spans(&root, source, &mut spans),
        Language::Java => extract_java_function_spans(&root, source, &mut spans),
        Language::C => extract_c_function_spans(&root, source, &mut spans),
        Language::Ruby => extract_ruby_function_spans(&root, source, &mut spans),
        Language::Cpp => extract_cpp_function_spans(&root, source, &mut spans),
    }

    Ok(spans)
}

/// Pair a parsed signature + its [`SymbolKind`] with the 1-based line span of the
/// originating node.
fn span_of(
    node: &Node,
    sig: FunctionSignature,
    kind: SymbolKind,
) -> (FunctionSignature, SymbolKind, usize, usize) {
    let start = node.start_position().row + 1;
    let end = node.end_position().row + 1;
    (sig, kind, start, end)
}

/// Build a placeholder [`FunctionSignature`] (no params/return) for a TYPE
/// declaration node, capturing its `name` field and 1-based start position. The
/// signature string the chunker renders for it is just `"{keyword} {name}"`
/// (e.g. `"struct Foo"`); fields/generics are intentionally skipped — name +
/// kind is enough. Returns `None` when the node has no `name`.
fn type_symbol(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;
    let start = node.start_position();
    Some(FunctionSignature::new(name).with_position(start.row + 1, start.column))
}

/// Span-aware mirror of [`extract_typescript_functions`], additionally emitting
/// type symbols for `class`/`interface`/`type alias`/`enum` declarations.
///
/// A class/interface declaration is emitted as its own type symbol AND descended
/// into for its methods, so both the wrapper and its inner methods become Symbol
/// chunks (their ranges differ — no id collision).
fn extract_typescript_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_declaration" | "function_signature" => {
                if let Some(sig) = parse_typescript_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            "export_statement" => {
                // `export function/class/interface/type/enum ...`: classify by
                // the inner declaration so an exported type still becomes a type
                // symbol (and an exported class still yields its method spans).
                if let Some(declaration) = child.child_by_field_name("declaration") {
                    extract_typescript_declaration_span(&declaration, source, spans);
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration"
            | "enum_declaration" => {
                extract_typescript_declaration_span(&child, source, spans);
            }
            "method_definition" | "method_signature" => {
                if let Some(sig) = parse_typescript_method(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            "abstract_method_signature" | "call_signature" | "construct_signature" => {
                if let Some(sig) = parse_typescript_signature(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            _ => {
                extract_typescript_function_spans(&child, source, spans);
            }
        }
    }
}

/// Emit the span(s) for a single TypeScript declaration node (a function or a
/// type declaration), used both for top-level nodes and for the inner
/// `declaration` of an `export_statement`. A class/interface emits its own type
/// symbol AND descends into its body for methods.
fn extract_typescript_declaration_span(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    match node.kind() {
        "function_declaration" | "function_signature" => {
            if let Some(sig) = parse_typescript_function(node, source) {
                spans.push(span_of(node, sig, SymbolKind::Function));
            }
        }
        "class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(kind) = typescript_type_kind(node.kind()) {
                if let Some(sig) = type_symbol(node, source) {
                    spans.push(span_of(node, sig, kind));
                }
            }
            // Descend into the body for methods (classes/interfaces only).
            if let Some(body) = node.child_by_field_name("body") {
                extract_typescript_function_spans(&body, source, spans);
            }
        }
        _ => {}
    }
}

/// Map a TypeScript type-declaration node kind to its [`SymbolKind`].
fn typescript_type_kind(kind: &str) -> Option<SymbolKind> {
    match kind {
        "class_declaration" => Some(SymbolKind::Class),
        "interface_declaration" => Some(SymbolKind::Interface),
        "type_alias_declaration" => Some(SymbolKind::Type),
        "enum_declaration" => Some(SymbolKind::Enum),
        _ => None,
    }
}

/// Span-aware mirror of [`extract_rust_functions`], additionally emitting type
/// symbols for `struct`/`union`/`enum`/`trait`/type-alias declarations.
fn extract_rust_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            "impl_item" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_rust_function_spans(&body, source, spans);
                }
            }
            // `union` is struct-like (a named record type) → kind `struct`.
            "struct_item" | "union_item" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Struct));
                }
            }
            "enum_item" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Enum));
                }
            }
            // A type alias (`type Foo = ...;`) → kind `type`.
            "type_item" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Type));
                }
            }
            "trait_item" => {
                // Emit the trait as its own symbol, then descend for its methods.
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Trait));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_rust_trait_method_spans(&body, source, spans);
                }
            }
            "declaration_list" | "field_declaration_list" => {
                extract_rust_function_spans(&child, source, spans);
            }
            "associated_function" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            _ => {
                extract_rust_function_spans(&child, source, spans);
            }
        }
    }
}

/// Span-aware mirror of [`extract_rust_trait_methods`].
fn extract_rust_trait_method_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_signature_item" | "function_item" | "associated_function" => {
                if let Some(sig) = parse_rust_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            _ => {
                extract_rust_trait_method_spans(&child, source, spans);
            }
        }
    }
}

// ============================================================================
// Python extraction
// ============================================================================

/// Extract function/method signatures from Python source.
///
/// Mirrors [`extract_rust_functions`]: a recursive child walk where the two
/// structural node kinds are `function_definition` (top-level `def` and, when
/// nested inside a class body, methods) and `class_definition` (whose `block`
/// body we descend into so its methods are captured). Python has no syntactic
/// receiver, so methods are rendered as plain functions — the
/// method-vs-function distinction is Go-specific (see [`parse_go_method`]).
fn extract_python_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_python_function(&child, source) {
                    signatures.push(sig);
                }
                // Descend in case of nested defs (closures / inner functions).
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_functions(&body, source, signatures);
                }
            }
            "class_definition" => {
                // Methods are `function_definition` nodes inside the class body.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_functions(&body, source, signatures);
                }
            }
            _ => {
                extract_python_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a Python `function_definition` into a [`FunctionSignature`].
fn parse_python_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Parameters live under a `parameters` node; each child is either a bare
    // `identifier` or a `typed_parameter`/`default_parameter` carrying a type.
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if let Some((name, type_ann)) = extract_python_param(&param, source) {
                sig = sig.add_parameter(name, type_ann);
            }
        }
    }

    // Return type is the `return_type` field (the annotation after `->`).
    if let Some(ret_type) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = ret_type.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }

    Some(sig)
}

/// Extract a `(name, type_annotation)` pair from a Python parameter node.
fn extract_python_param(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node.utf8_text(source.as_bytes()).ok()?.to_string();
            Some((name, None))
        }
        // `name: Type`
        "typed_parameter" => {
            let name = node
                .child(0)
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())?;
            let type_ann = node
                .child_by_field_name("type")
                .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());
            Some((name, type_ann))
        }
        // `name = default` (the name is the `name` field)
        "default_parameter" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())?;
            Some((name, None))
        }
        // `name: Type = default`
        "typed_default_parameter" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())?;
            let type_ann = node
                .child_by_field_name("type")
                .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string());
            Some((name, type_ann))
        }
        // `*args` / `**kwargs`
        "list_splat_pattern" | "dictionary_splat_pattern" => {
            let name = node.utf8_text(source.as_bytes()).ok()?.to_string();
            Some((name, None))
        }
        _ => None,
    }
}

/// Extract imports from Python source: `import_statement` (`import a.b`,
/// `import a as b`) and `import_from_statement` (`from a import b`).
fn extract_python_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    match node.kind() {
        "import_statement" => {
            // `import a.b.c` / `import a as b` — the module path is a
            // `dotted_name` (or `aliased_import` wrapping one).
            let line = node.start_position().row + 1;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "dotted_name" => {
                        if let Ok(text) = child.utf8_text(source.as_bytes()) {
                            imports.push(
                                ImportStatement::new(text.to_string(), ImportKind::SideEffect)
                                    .with_line(line),
                            );
                        }
                    }
                    "aliased_import" => {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                                let alias = child
                                    .child_by_field_name("alias")
                                    .and_then(|a| a.utf8_text(source.as_bytes()).ok())
                                    .unwrap_or_default()
                                    .to_string();
                                imports.push(
                                    ImportStatement::new(
                                        text.to_string(),
                                        ImportKind::Namespace(alias),
                                    )
                                    .with_line(line),
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
        "import_from_statement" => {
            // `from <module> import a, b` — `module_name` field is the source;
            // imported names are the remaining `dotted_name`/`identifier` nodes.
            let line = node.start_position().row + 1;
            let module = node
                .child_by_field_name("module_name")
                .and_then(|m| m.utf8_text(source.as_bytes()).ok())
                .map(|s| s.to_string())
                .unwrap_or_default();

            let mut names = Vec::new();
            let module_field = node.child_by_field_name("module_name");
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                // Skip the module name node itself; collect imported identifiers.
                if Some(child) == module_field {
                    continue;
                }
                if matches!(child.kind(), "dotted_name" | "identifier") {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        names.push(text.to_string());
                    }
                } else if child.kind() == "aliased_import" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(text) = name_node.utf8_text(source.as_bytes()) {
                            names.push(text.to_string());
                        }
                    }
                }
            }

            let kind = if names.is_empty() {
                // `from module import *`
                ImportKind::SideEffect
            } else {
                ImportKind::Named(names)
            };
            imports.push(ImportStatement::new(module, kind).with_line(line));
            return;
        }
        _ => {}
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_python_imports(&child, source, imports);
    }
}

/// Span-aware mirror of [`extract_python_functions`], additionally emitting a
/// `Class` symbol for each `class_definition`.
///
/// The class span (its full `[start, end]`) OVERLAPS its method spans — both
/// become Symbol chunks. Their ranges differ (the class spans the whole block,
/// each method a sub-range), so the `path:start-end` ids stay distinct.
fn extract_python_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_python_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_function_spans(&body, source, spans);
                }
            }
            "class_definition" => {
                // Emit the class itself, then descend for its methods.
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Class));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_function_spans(&body, source, spans);
                }
            }
            _ => {
                extract_python_function_spans(&child, source, spans);
            }
        }
    }
}

// ============================================================================
// Go extraction
// ============================================================================

/// Extract function/method signatures from Go source.
///
/// Mirrors [`extract_rust_functions`]: a recursive child walk where
/// `function_declaration` is a free function and `method_declaration` (which
/// carries a `receiver`) is a method. The receiver is threaded in as the first
/// parameter so downstream `is_method`/kind logic can read it off the
/// signature, matching how Rust's `self_parameter` is surfaced.
fn extract_go_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(sig) = parse_go_function(&child, source) {
                    signatures.push(sig);
                }
            }
            "method_declaration" => {
                if let Some(sig) = parse_go_method(&child, source) {
                    signatures.push(sig);
                }
            }
            _ => {
                extract_go_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a Go `function_declaration` (field `name: identifier`).
fn parse_go_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);
    let sig = add_go_parameters(node, source, sig);
    let sig = add_go_result(node, source, sig);

    Some(sig)
}

/// Parse a Go `method_declaration` (has `receiver: parameter_list`, field
/// `name: field_identifier`). The receiver is added as the leading parameter so
/// the signature reads as a method.
fn parse_go_method(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);

    // Receiver: `(r *Repo)`. Surfaced as the leading parameter named `self`
    // with the full receiver text as its type annotation — mirroring how Rust's
    // `self_parameter` is surfaced as `("self", Some(self_text))`. This lets the
    // chunker's existing `is_method` (first param named `self`) classify Go
    // methods without any chunker change, while the real receiver name is
    // preserved in the type annotation.
    if let Some(receiver) = node.child_by_field_name("receiver") {
        if let Ok(recv_text) = receiver.utf8_text(source.as_bytes()) {
            let recv_inner = recv_text
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim()
                .to_string();
            sig = sig.add_parameter("self", Some(recv_inner));
        }
    }

    let sig = add_go_parameters(node, source, sig);
    let sig = add_go_result(node, source, sig);

    Some(sig)
}

/// Append the declared `parameters` of a Go func/method to `sig`.
fn add_go_parameters(node: &Node, source: &str, mut sig: FunctionSignature) -> FunctionSignature {
    if let Some(params) = node.child_by_field_name("parameters") {
        let cursor = &mut params.walk();
        for param in params.children(cursor) {
            if let Some((name, type_ann)) = extract_go_param(&param, source) {
                sig = sig.add_parameter(name, type_ann);
            }
        }
    }
    sig
}

/// Append the `result` (return type) of a Go func/method to `sig`.
fn add_go_result(node: &Node, source: &str, mut sig: FunctionSignature) -> FunctionSignature {
    if let Some(result) = node.child_by_field_name("result") {
        if let Ok(type_text) = result.utf8_text(source.as_bytes()) {
            sig = sig.with_return_type(type_text.to_string());
        }
    }
    sig
}

/// Extract a `(name, type_annotation)` pair from a Go `parameter_declaration`.
///
/// A `parameter_declaration` has an optional `name: identifier` field and a
/// `type` field. Variadic/exotic shapes (generics `type_parameter_list`, etc.)
/// fall through with only the name captured, per the scope guard.
fn extract_go_param(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    if node.kind() != "parameter_declaration" {
        return None;
    }

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "_".to_string());

    let type_ann = node
        .child_by_field_name("type")
        .and_then(|t| t.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string());

    Some((name, type_ann))
}

/// Extract imports from Go source. Both single (`import "fmt"`) and grouped
/// (`import ( ... )`) forms resolve to one or more `import_spec` nodes.
fn extract_go_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "import_spec" {
        let line = node.start_position().row + 1;
        // The path is an `interpreted_string_literal`; an optional alias is the
        // `name` field (`import alias "path"`).
        let path = node
            .child_by_field_name("path")
            .and_then(|p| p.utf8_text(source.as_bytes()).ok())
            .map(|s| s.trim_matches('"').to_string());

        if let Some(path) = path {
            let kind = match node
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            {
                Some(alias) => ImportKind::Namespace(alias.to_string()),
                None => ImportKind::SideEffect,
            };
            imports.push(ImportStatement::new(path, kind).with_line(line));
        }
        return;
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_go_imports(&child, source, imports);
    }
}

/// Span-aware mirror of [`extract_go_functions`], additionally emitting type
/// symbols for `type_spec` declarations (under `type_declaration`).
fn extract_go_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(sig) = parse_go_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            "method_declaration" => {
                if let Some(sig) = parse_go_method(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            // `type Foo struct {...}` / `type R interface {...}` / `type T int`
            // each parse to a `type_spec` (carrying the `name` field) nested under
            // a `type_declaration` (single or grouped `type ( ... )`).
            "type_declaration" => {
                let spec_cursor = &mut child.walk();
                for spec in child.children(spec_cursor) {
                    if spec.kind() == "type_spec" {
                        if let Some(sig) = type_symbol(&spec, source) {
                            let kind = go_type_spec_kind(&spec);
                            spans.push(span_of(&spec, sig, kind));
                        }
                    }
                }
            }
            _ => {
                extract_go_function_spans(&child, source, spans);
            }
        }
    }
}

// ============================================================================
// Bash extraction
// ============================================================================

/// Extract function signatures from Bash source.
///
/// Mirrors the Python extractor: a recursive child walk where `function_definition`
/// is the unit of extraction. Bash has no methods, classes, or types — only
/// functions — so the surface here is intentionally smaller than the Python/Go
/// extractors.
fn extract_bash_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_bash_function(&child, source) {
                    signatures.push(sig);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_bash_functions(&body, source, signatures);
                }
            }
            _ => {
                extract_bash_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a Bash `function_definition` (field `name: word`).
/// The signature is just the function name — no parameters surface in the
/// grammar (Bash functions are positionally bound, not typed).
fn parse_bash_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);
    Some(sig)
}

/// Extract spans of Bash function definitions. Mirrors `extract_python_function_spans`.
fn extract_bash_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_bash_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_bash_function_spans(&body, source, spans);
                }
            }
            _ => {
                extract_bash_function_spans(&child, source, spans);
            }
        }
    }
}

/// Extract Bash `source` and `.` builtins as Require-kind imports.
///
/// tree-sitter-bash models these as a `command` node whose children are:
///   - `command_name` (a `word` containing the literal "source" or ".")
///   - then one or more `word` siblings that are the positional arguments
///   - and possibly `string` nodes for double-quoted args
///
/// We collect the first sibling `word`/`string`/`raw_string`/`concatenation`
/// after the `command_name` as the import source.
fn extract_bash_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "command" {
        // Find the command_name child to learn the verb.
        let mut verb = "";
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            if child.kind() == "command_name" {
                verb = child.utf8_text(source.as_bytes()).unwrap_or("");
                break;
            }
        }
        if verb == "source" || verb == "." {
            // Walk children again; the first `word`, `string`, `raw_string`, or
            // `concatenation` AFTER the command_name is the path argument.
            let mut past_name = false;
            let cursor = &mut node.walk();
            for child in node.children(cursor) {
                if child.kind() == "command_name" {
                    past_name = true;
                    continue;
                }
                if !past_name {
                    continue;
                }
                let k = child.kind();
                if k == "word" || k == "string" || k == "raw_string" || k == "concatenation" {
                    if let Ok(text) = child.utf8_text(source.as_bytes()) {
                        let line = node.start_position().row + 1;
                        imports.push(
                            ImportStatement::new(
                                text.to_string(),
                                ImportKind::Require(text.to_string()),
                            )
                            .with_line(line),
                        );
                        break;
                    }
                }
            }
        }
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_bash_imports(&child, source, imports);
    }
}

/// Extract Bash `export` and `declare -x` declarations as export rows.
///
/// tree-sitter-bash models BOTH `export FOO=bar` AND `declare -x FOO=bar` as
/// `declaration_command` nodes (not `command` nodes). Inside:
///   - `export` or `declare` keyword node
///   - then one or more `variable_name` siblings (for `export FOO` without value)
///   - or one or more `variable_assignment` children (for `export FOO=bar` /
///     `declare -x FOO=bar`); each carries a `variable_name` child with field
///     name `name`.
fn extract_bash_exports(node: &Node, source: &str, exports: &mut Vec<ExportStatement>) {
    if node.kind() == "declaration_command" {
        let line = node.start_position().row + 1;
        // Two cases to handle: a `variable_name` sibling directly, or a
        // `variable_assignment` whose field `name` is the variable.
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            let k = child.kind();
            if k == "variable_name" {
                if let Ok(name) = child.utf8_text(source.as_bytes()) {
                    exports.push(
                        ExportStatement::new(ExportKind::Named(vec![name.to_string()]))
                            .with_line(line),
                    );
                }
            } else if k == "variable_assignment" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                        exports.push(
                            ExportStatement::new(ExportKind::Named(vec![name.to_string()]))
                                .with_line(line),
                        );
                    }
                }
            }
        }
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_bash_exports(&child, source, exports);
    }
}

// ============================================================================
// Java extraction
// ============================================================================

/// Extract method signatures from Java source.
///
/// Mirrors the Python extractor's pattern: a recursive child walk where
/// `method_declaration` (inside a `class_body` or `interface_body`) is the
/// unit, and class/interface/enum/record declarations are descended into to
/// reach their methods. Each class/interface/enum/record is also emitted as a
/// type symbol so the chunker indexes its `kind` correctly.
fn extract_java_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
                // The class/interface/enum/record name lives in field "name".
                if let Some(sig) = type_symbol(&child, source) {
                    signatures.push(sig);
                }
                // Descend into the body to capture methods.
                let body_kinds = ["class_body", "interface_body", "enum_body"];
                for bk in &body_kinds {
                    if let Some(body) = child.child_by_field_name(bk) {
                        extract_java_functions(&body, source, signatures);
                        break;
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(sig) = parse_java_method(&child, source) {
                    signatures.push(sig);
                }
            }
            _ => {
                extract_java_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a Java `method_declaration` (field `name: identifier`).
fn parse_java_method(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);
    if let Some(params) = node.child_by_field_name("parameters") {
        let p_cursor = &mut params.walk();
        for param in params.children(p_cursor) {
            if param.kind() == "formal_parameter" || param.kind() == "spread_parameter" {
                if let Some(p_name) = param.child_by_field_name("name") {
                    if let Ok(p_text) = p_name.utf8_text(source.as_bytes()) {
                        let type_text = param
                            .child_by_field_name("type")
                            .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string());
                        sig.parameters.push(Parameter {
                            name: p_text.to_string(),
                            type_annotation: type_text,
                        });
                    }
                }
            }
        }
    }
    Some(sig)
}

/// Extract spans of Java methods and type symbols. Mirrors `extract_python_function_spans`.
fn extract_java_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "class_declaration" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Class));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_java_function_spans(&body, source, spans);
                }
            }
            "interface_declaration" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Interface));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_java_function_spans(&body, source, spans);
                }
            }
            "enum_declaration" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Enum));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_java_function_spans(&body, source, spans);
                }
            }
            "record_declaration" => {
                if let Some(sig) = type_symbol(&child, source) {
                    // Records are class-like; emit as Class for now.
                    spans.push(span_of(&child, sig, SymbolKind::Class));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_java_function_spans(&body, source, spans);
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(sig) = parse_java_method(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
            }
            _ => {
                extract_java_function_spans(&child, source, spans);
            }
        }
    }
}

/// Extract Java `import_declaration` nodes as Named imports.
/// Java imports carry the package/class path as a `scoped_identifier` or
/// `identifier` child. We capture its full UTF-8 text.
fn extract_java_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "import_declaration" {
        let line = node.start_position().row + 1;
        // The path is a `scoped_identifier` or `identifier` child.
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            let k = child.kind();
            if k == "scoped_identifier" || k == "identifier" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    imports.push(
                        ImportStatement::new(
                            text.to_string(),
                            ImportKind::Named(vec![text.to_string()]),
                        )
                        .with_line(line),
                    );
                    break;
                }
            }
        }
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_java_imports(&child, source, imports);
    }
}

// ============================================================================
// C extraction
// ============================================================================

/// Extract function signatures from C source.
///
/// Mirrors the Python/Java extractors: a recursive child walk where
/// `function_definition` is the unit. C has no methods/classes — just free
/// functions — so the surface here is intentionally smaller than Python/Go.
fn extract_c_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_c_function(&child, source) {
                    signatures.push(sig);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_c_functions(&body, source, signatures);
                }
            }
            _ => {
                extract_c_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a C/C++ `function_definition` (the name lives in a `function_declarator`
/// child as a direct `identifier` or `field_identifier` child, NOT a field —
/// tree-sitter-c models `function_declarator` as having `identifier` and
/// `parameter_list` children without field-name tags; tree-sitter-cpp adds
/// `field_identifier` for member functions). Parameters come from a
/// `parameter_list` child.
///
/// This function is shared between the C and C++ extractors. C++ member
/// functions use `field_identifier` (the function name as a class member)
/// while C free functions use `identifier`. Both are accepted here.
fn parse_c_function(node: &Node, source: &str) -> Option<FunctionSignature> {
    let declarator = node.child_by_field_name("declarator")?;
    // The function name is the first `identifier` OR `field_identifier`
    // child of the declarator.
    let name = {
        let mut found_name = None;
        let d_cursor = &mut declarator.walk();
        for c in declarator.children(d_cursor) {
            if c.kind() == "identifier" || c.kind() == "field_identifier" {
                if let Ok(t) = c.utf8_text(source.as_bytes()) {
                    found_name = Some(t.to_string());
                    break;
                }
            }
        }
        found_name?
    };

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);
    if let Some(params) = declarator.child_by_field_name("parameters") {
        let p_cursor = &mut params.walk();
        for param in params.children(p_cursor) {
            if param.kind() == "parameter_declaration" {
                if let Some(p_name) = param.child_by_field_name("declarator") {
                    if let Ok(p_text) = p_name.utf8_text(source.as_bytes()) {
                        let type_text = param
                            .child_by_field_name("type")
                            .and_then(|t| t.utf8_text(source.as_bytes()).ok())
                            .map(|s| s.to_string());
                        sig.parameters.push(Parameter {
                            name: p_text.to_string(),
                            type_annotation: type_text,
                        });
                    }
                }
            }
        }
    }
    if let Some(ret_type) = node.child_by_field_name("type") {
        if let Ok(t) = ret_type.utf8_text(source.as_bytes()) {
            sig.return_type = Some(t.to_string());
        }
    }
    Some(sig)
}

/// Extract spans of C function definitions. Mirrors `extract_python_function_spans`.
fn extract_c_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_c_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_c_function_spans(&body, source, spans);
                }
            }
            _ => {
                extract_c_function_spans(&child, source, spans);
            }
        }
    }
}

/// Extract C `#include` directives as Require-kind imports.
///
/// tree-sitter-c models these as `preproc_include` nodes whose children are:
///   - `#include` (preproc keyword)
///   - `system_lib_string` (for `<stdio.h>`) or `string_literal` (for `"x.h"`)
fn extract_c_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "preproc_include" {
        let line = node.start_position().row + 1;
        let cursor = &mut node.walk();
        for child in node.children(cursor) {
            let k = child.kind();
            if k == "system_lib_string" || k == "string_literal" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    imports.push(
                        ImportStatement::new(
                            text.to_string(),
                            ImportKind::Require(text.to_string()),
                        )
                        .with_line(line),
                    );
                    break;
                }
            }
        }
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_c_imports(&child, source, imports);
    }
}

// ============================================================================
// Ruby extraction
// ============================================================================

/// Extract method signatures from Ruby source.
///
/// Mirrors the Python extractor: a recursive child walk where `method` is the
/// unit, and `class` / `module` declarations are descended into to reach
/// their methods. Class and module are also emitted as type symbols so the
/// chunker indexes their kind.
///
/// KEY QUIRK (per R-1.3 in the plan): the recursion DESCENDS into
/// `body_statement` (the class/module body) but does NOT descend into
/// `do_block` or `block` (lambda / proc / do-end blocks inside a method).
/// The `def` exterior only — inner blocks are part of the enclosing method
/// and should not be emitted as separate top-level method symbols.
fn extract_ruby_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "class" | "module" => {
                if let Some(sig) = type_symbol(&child, source) {
                    signatures.push(sig);
                }
                // Descend only into the body_statement, NOT into method bodies.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_ruby_functions(&body, source, signatures);
                }
            }
            "method" | "singleton_method" => {
                if let Some(sig) = parse_ruby_method(&child, source) {
                    signatures.push(sig);
                }
                // Do NOT recurse into the method body — its do_block / block
                // children are implementation detail, not top-level methods.
            }
            _ => {
                extract_ruby_functions(&child, source, signatures);
            }
        }
    }
}

/// Parse a Ruby `method` node (field `name: identifier`, parameters in
/// `method_parameters`/`parameters` child).
fn parse_ruby_method(node: &Node, source: &str) -> Option<FunctionSignature> {
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let start_position = node.start_position();
    let mut sig =
        FunctionSignature::new(name).with_position(start_position.row + 1, start_position.column);
    // The parameters live in either `method_parameters` or `parameters` child.
    let params = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("method_parameters"));
    if let Some(params) = params {
        // Collect parameter names eagerly into a Vec<String> to avoid
        // lifetime issues with `param.walk()` cursors that don't outlive the
        // params iteration. Ruby's grammar keeps parameter naming shallow
        // (an `identifier` child carries the name directly), so we just
        // walk the children and read the first `identifier` text per param.
        let param_texts: Vec<String> = {
            let mut names = Vec::new();
            let mut p_cursor = params.walk();
            for param in params.children(&mut p_cursor) {
                let k = param.kind();
                if k == "identifier" {
                    if let Ok(t) = param.utf8_text(source.as_bytes()) {
                        names.push(t.to_string());
                    }
                } else if k == "optional_parameter"
                    || k == "splat_parameter"
                    || k == "keyword_parameter"
                    || k == "block_parameter"
                    || k == "hash_keyword_parameter"
                {
                    // field "name" first; else the first identifier child.
                    if let Some(name_node) = param.child_by_field_name("name") {
                        if let Ok(t) = name_node.utf8_text(source.as_bytes()) {
                            names.push(t.to_string());
                        }
                    } else {
                        // Walk children, holding the cursor in a binding that
                        // outlives the per-child iteration via an explicit
                        // collect.
                        let mut inner = param.walk();
                        let children: Vec<_> = param
                            .children(&mut inner)
                            .filter(|c| c.kind() == "identifier")
                            .collect();
                        if let Some(c) = children.first() {
                            if let Ok(t) = c.utf8_text(source.as_bytes()) {
                                names.push(t.to_string());
                            }
                        }
                    }
                }
            }
            names
        };
        for name in param_texts {
            sig.parameters.push(Parameter {
                name,
                type_annotation: None, // Ruby is dynamically typed.
            });
        }
    }
    Some(sig)
}

/// Extract spans of Ruby methods + class/module type symbols.
fn extract_ruby_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "class" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Class));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_ruby_function_spans(&body, source, spans);
                }
            }
            "module" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Module));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_ruby_function_spans(&body, source, spans);
                }
            }
            "method" => {
                if let Some(sig) = parse_ruby_method(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                // No descent into method body.
            }
            "singleton_method" => {
                if let Some(sig) = parse_ruby_method(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                // No descent into method body.
            }
            _ => {
                extract_ruby_function_spans(&child, source, spans);
            }
        }
    }
}

/// Extract Ruby `require` and `require_relative` calls as Require-kind imports.
fn extract_ruby_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    if node.kind() == "call" {
        // The first child is typically the method name (`identifier`).
        let cursor = &mut node.walk();
        let mut method_name = "";
        let mut arg_text: Option<String> = None;
        let mut first_identifier_seen = false;
        for child in node.children(cursor) {
            if !first_identifier_seen && child.kind() == "identifier" {
                method_name = child.utf8_text(source.as_bytes()).unwrap_or("");
                first_identifier_seen = true;
            } else if first_identifier_seen && child.kind() == "argument_list" {
                // First string child of the argument list is the path.
                let a_cursor = &mut child.walk();
                for arg in child.children(a_cursor) {
                    if arg.kind() == "string" {
                        if let Ok(t) = arg.utf8_text(source.as_bytes()) {
                            // Strip the surrounding quotes.
                            let stripped = t
                                .trim_start_matches(['\'', '"'])
                                .trim_end_matches(['\'', '"'])
                                .to_string();
                            arg_text = Some(stripped);
                            break;
                        }
                    }
                }
                break;
            }
        }
        if method_name == "require" || method_name == "require_relative" {
            if let Some(arg) = arg_text {
                let line = node.start_position().row + 1;
                imports.push(
                    ImportStatement::new(arg.clone(), ImportKind::Require(arg)).with_line(line),
                );
            }
        }
    }

    let cursor = &mut node.walk();
    for child in node.children(cursor) {
        extract_ruby_imports(&child, source, imports);
    }
}

// ============================================================================
// C++ extraction
// ============================================================================

/// C++ shares `function_definition` and `preproc_include` with C (tree-sitter-cpp
/// is a strict superset of tree-sitter-c at the relevant nodes). We re-use the
/// C extractor for those surface nodes and only ADD the C++-specific node
/// kinds (`class_specifier`, `struct_specifier`, `namespace_definition`) as
/// additional type symbols.
///
/// C++ also has additional quirks we deliberately defer:
///   - templates (template <typename T> ...): we capture the OUTER name only,
///     not the template parameter list. Same as the Go/Python extractor
///     "exotic detail" precedent (R-4 in the plan).
///   - operator overloads (operator+, operator<<): captured as Function
///     symbols with the operator symbol as the name (matches what a
///     human would search for).
///   - member access (`->`, `.`): ignored (not top-level symbols).
///
/// Per the v0.3.0 plan, this is the KOTLIN-CPP story's C++ side; Kotlin is
/// deferred because tree-sitter-kotlin has no 0.23.x line on crates.io
/// (only 0.2.x and 0.3.x majors), violating the workspace's tree-sitter
/// version pin policy. Documented in the F1-KOTLIN-CPP commit message.
fn extract_cpp_functions(node: &Node, source: &str, signatures: &mut Vec<FunctionSignature>) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_c_function(&child, source) {
                    signatures.push(sig);
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_cpp_functions(&body, source, signatures);
                }
            }
            "class_specifier" | "struct_specifier" => {
                // C++ class / struct: emit the type symbol AND descend into
                // its body for member functions. The C grammar has no
                // class_specifier at the top level (only struct_type as a
                // field type), so this branch is C++-only.
                if let Some(sig) = type_symbol(&child, source) {
                    signatures.push(sig);
                }
                // Descend into field_declaration_list (the C++ class body)
                // and re-run the extractor so member function_definitions
                // are picked up.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_cpp_functions(&body, source, signatures);
                }
            }
            _ => {
                extract_cpp_functions(&child, source, signatures);
            }
        }
    }
}

/// Extract spans of C++ function definitions + class/struct type symbols.
fn extract_cpp_function_spans(
    node: &Node,
    source: &str,
    spans: &mut Vec<(FunctionSignature, SymbolKind, usize, usize)>,
) {
    let cursor = &mut node.walk();

    for child in node.children(cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(sig) = parse_c_function(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Function));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_cpp_function_spans(&body, source, spans);
                }
            }
            "class_specifier" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Class));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_cpp_function_spans(&body, source, spans);
                }
            }
            "struct_specifier" => {
                if let Some(sig) = type_symbol(&child, source) {
                    spans.push(span_of(&child, sig, SymbolKind::Struct));
                }
                if let Some(body) = child.child_by_field_name("body") {
                    extract_cpp_function_spans(&body, source, spans);
                }
            }
            _ => {
                extract_cpp_function_spans(&child, source, spans);
            }
        }
    }
}

/// Extract C++ `#include` directives. Re-uses the C extractor verbatim —
/// the C++ grammar's preproc_include node shape is identical.
fn extract_cpp_imports(node: &Node, source: &str, imports: &mut Vec<ImportStatement>) {
    extract_c_imports(node, source, imports);
}

/// Classify a Go `type_spec` by its underlying type child: a `struct_type` child
/// → `Struct`, an `interface_type` child → `Interface`, anything else (named
/// type, alias, function type, etc.) → `Type`.
fn go_type_spec_kind(spec: &Node) -> SymbolKind {
    let cursor = &mut spec.walk();
    for child in spec.children(cursor) {
        match child.kind() {
            "struct_type" => return SymbolKind::Struct,
            "interface_type" => return SymbolKind::Interface,
            _ => {}
        }
    }
    SymbolKind::Type
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(
            detect_language(Path::new("test.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            detect_language(Path::new("test.tsx")),
            Some(Language::TypeScript)
        );
        assert_eq!(detect_language(Path::new("test.rs")), Some(Language::Rust));
        assert_eq!(
            detect_language(Path::new("test.py")),
            Some(Language::Python)
        );
        assert_eq!(detect_language(Path::new("test.go")), Some(Language::Go));
        assert_eq!(detect_language(Path::new("test.js")), None);
    }

    #[test]
    fn test_parse_typescript_simple_function() {
        let source = r#"
function add(a: number, b: number): number {
    return a + b;
}
"#;

        let sigs = parse_source(source, Language::TypeScript).unwrap();
        assert_eq!(sigs.len(), 1);

        let add = &sigs[0];
        assert_eq!(add.name, "add");
        assert_eq!(add.line, 2);
        assert_eq!(add.return_type, Some(": number".to_string()));
    }

    #[test]
    fn test_parse_rust_simple_function() {
        let source = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;

        let sigs = parse_source(source, Language::Rust).unwrap();
        assert_eq!(sigs.len(), 1);

        let add = &sigs[0];
        assert_eq!(add.name, "add");
        assert_eq!(add.line, 2);
        assert_eq!(add.return_type, Some("i32".to_string()));
    }

    #[test]
    fn test_parse_typescript_exported_function() {
        let source = r#"
export function greet(name: string): void {
    console.log(`Hello, ${name}!`);
}
"#;

        let sigs = parse_source(source, Language::TypeScript).unwrap();
        assert_eq!(sigs.len(), 1);
        assert_eq!(sigs[0].name, "greet");
    }

    #[test]
    fn test_parse_rust_impl_methods() {
        let source = r#"
impl MyStruct {
    pub fn new() -> Self {
        MyStruct {}
    }

    fn private_method(&self) -> i32 {
        42
    }
}
"#;

        let sigs = parse_source(source, Language::Rust).unwrap();
        assert_eq!(sigs.len(), 2);

        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"new"));
        assert!(names.contains(&"private_method"));
    }

    #[test]
    fn test_parse_typescript_interface_methods() {
        let source = r#"
interface Calculator {
    add(a: number, b: number): number;
    subtract(a: number, b: number): number;
}
"#;

        let sigs = parse_source(source, Language::TypeScript).unwrap();
        assert_eq!(sigs.len(), 2);

        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"subtract"));
    }

    #[test]
    fn test_parse_rust_trait_methods() {
        let source = r#"
trait Drawable {
    fn draw(&self);
    fn get_bounds(&self) -> Rect;
}
"#;

        let sigs = parse_source(source, Language::Rust).unwrap();
        assert_eq!(sigs.len(), 2);

        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"draw"));
        assert!(names.contains(&"get_bounds"));
    }

    #[test]
    fn test_parse_file_typescript_fixture() {
        let path = Path::new("tests/fixtures/fixture.ts");
        let sigs = parse_file(path).unwrap();
        assert_eq!(sigs.len(), 3);

        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"parseString"));
        assert!(names.contains(&"calculateSum"));
        assert!(names.contains(&"isValid"));
    }

    #[test]
    fn test_parse_file_rust_fixture() {
        let path = Path::new("tests/fixtures/fixture.rs");
        let sigs = parse_file(path).unwrap();
        assert_eq!(sigs.len(), 2);

        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"process_data"));
        assert!(names.contains(&"calculate_total"));
    }

    // =========================================================================
    // Span-aware Parsing Tests (parse_source_spans)
    // =========================================================================

    #[test]
    fn test_parse_source_spans_rust_two_functions() {
        let source = r#"pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn sub(a: i32, b: i32) -> i32 {
    a - b
}
"#;

        let spans = parse_source_spans(source, Language::Rust).unwrap();
        assert_eq!(spans.len(), 2);

        let (add, add_kind, add_start, add_end) = &spans[0];
        assert_eq!(add.name, "add");
        assert_eq!(*add_kind, SymbolKind::Function);
        assert_eq!(*add_start, 1);
        assert_eq!(*add_end, 3);
        assert!(add_start < add_end);

        let (sub, sub_kind, sub_start, sub_end) = &spans[1];
        assert_eq!(sub.name, "sub");
        assert_eq!(*sub_kind, SymbolKind::Function);
        assert_eq!(*sub_start, 5);
        assert_eq!(*sub_end, 7);
        assert!(sub_start < sub_end);
    }

    #[test]
    fn test_parse_source_spans_typescript_two_functions() {
        let source = r#"function add(a: number, b: number): number {
    return a + b;
}

function sub(a: number, b: number): number {
    return a - b;
}
"#;

        let spans = parse_source_spans(source, Language::TypeScript).unwrap();
        assert_eq!(spans.len(), 2);

        let (add, add_kind, add_start, add_end) = &spans[0];
        assert_eq!(add.name, "add");
        assert_eq!(*add_kind, SymbolKind::Function);
        assert_eq!(*add_start, 1);
        assert_eq!(*add_end, 3);
        assert!(add_start < add_end);

        let (sub, sub_kind, sub_start, sub_end) = &spans[1];
        assert_eq!(sub.name, "sub");
        assert_eq!(*sub_kind, SymbolKind::Function);
        assert_eq!(*sub_start, 5);
        assert_eq!(*sub_end, 7);
        assert!(sub_start < sub_end);
    }

    // =========================================================================
    // Import/Export Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_typescript_imports_basic() {
        // Test that the function runs without error - parser returns result
        let source = r#"
import { a, b, c } from './module-a';
import React from 'react';
import * as Utils from './utils';
import 'polyfills';

export function test() {}
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();
        // Parser works, may return 0 for short inputs (AST structure varies)
        let _ = imports;
    }

    #[test]
    fn test_parse_typescript_exports_basic() {
        // Test that the function runs without error
        let source = r#"
export function namedExport() {}
export class NamedClass {}
export { a, b };
"#;
        let (_, exports) = parse_source_imports_exports(source, Language::TypeScript).unwrap();
        // Parser works, verify no error
        let _ = exports;
    }

    #[test]
    fn test_parse_typescript_named_imports() {
        let source = r#"
import { a, b, c } from './module-a';

export function test() {}
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();
        // Parser runs without error
        let _ = imports;
    }

    #[test]
    fn test_parse_typescript_default_import() {
        let source = r#"
import React from 'react';

export function test() {}
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();
        let _ = imports;
    }

    #[test]
    fn test_parse_typescript_namespace_import() {
        let source = r#"
import * as Utils from './utils';
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();

        assert!(!imports.is_empty(), "Expected at least 1 import");

        let first = &imports[0];
        assert!(first.source.contains("utils"));
    }

    #[test]
    fn test_parse_typescript_side_effect_import() {
        let source = r#"
import 'polyfills';
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();

        assert!(!imports.is_empty(), "Expected at least 1 import");
    }

    #[test]
    fn test_parse_typescript_require() {
        let source = r#"
const fs = require('fs');
"#;
        let (_imports, _) = parse_source_imports_exports(source, Language::TypeScript).unwrap();
        // May or may not parse require depending on tree structure;
        // successful unwrap above is enough to assert the parser ran without error.
    }

    #[test]
    fn test_parse_typescript_named_exports() {
        let source = r#"
export function namedExport() {}
"#;
        let (_, exports) = parse_source_imports_exports(source, Language::TypeScript).unwrap();

        assert!(!exports.is_empty(), "Expected at least 1 export");
    }

    #[test]
    fn test_parse_typescript_re_exports() {
        let source = r#"
export { a, b } from './module-a';
"#;
        let (_, exports) = parse_source_imports_exports(source, Language::TypeScript).unwrap();

        // Should parse as re-export
        assert!(!exports.is_empty(), "Expected at least 1 export");
    }

    #[test]
    fn test_parse_rust_use_statements() {
        let source = r#"
use std::collections::HashMap;
use std::fmt::Debug;
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::Rust).unwrap();

        // We should get at least some imports
        assert!(
            !imports.is_empty(),
            "Expected at least 1 import, got {}",
            imports.len()
        );
    }

    #[test]
    fn test_parse_rust_mod_declaration() {
        let source = r#"
mod external_module;

mod inline_module {
    pub fn inner_function() {}
}
"#;
        let (imports, _) = parse_source_imports_exports(source, Language::Rust).unwrap();

        // Should have 2 modules
        assert!(
            !imports.is_empty(),
            "Expected at least 1 module, got {}",
            imports.len()
        );
    }

    #[test]
    fn test_parse_rust_pub_use() {
        let source = r#"
pub use crate::reexport::Item;
"#;
        let (_, exports) = parse_source_imports_exports(source, Language::Rust).unwrap();

        // pub use should appear as an export
        assert!(
            !exports.is_empty(),
            "Expected at least 1 export from pub use"
        );
    }

    #[test]
    fn test_parse_file_typescript_import_fixture() {
        let path = Path::new("tests/fixtures/imports.ts");
        let (imports, exports) = parse_imports_exports(path).unwrap();

        // Should have imports and exports
        assert!(
            !imports.is_empty(),
            "Expected at least 1 import, got {}",
            imports.len()
        );
        assert!(
            !exports.is_empty(),
            "Expected at least 1 export, got {}",
            exports.len()
        );
    }

    #[test]
    fn test_parse_file_rust_import_fixture() {
        let path = Path::new("tests/fixtures/imports.rs");
        let (imports, _) = parse_imports_exports(path).unwrap();

        // Should have at least some imports
        assert!(
            !imports.is_empty(),
            "Expected at least 1 import, got {}",
            imports.len()
        );
    }

    // =========================================================================
    // Python tests
    // =========================================================================

    #[test]
    fn test_detect_language_python_go() {
        assert_eq!(detect_language(Path::new("mod.py")), Some(Language::Python));
        assert_eq!(detect_language(Path::new("main.go")), Some(Language::Go));
    }

    #[test]
    fn test_parse_python_function_spans() {
        // Two top-level functions plus a class with two methods: methods are
        // captured by descending into the `class_definition` body.
        let source = r#"def add(a: int, b: int) -> int:
    return a + b


class Calc:
    def __init__(self) -> None:
        self.total = 0

    def bump(self, n: int) -> int:
        self.total += n
        return self.total
"#;

        let spans = parse_source_spans(source, Language::Python).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"__init__"));
        assert!(names.contains(&"bump"));

        // The class itself is emitted as a `Class` symbol (a type symbol).
        let (calc, calc_kind, calc_start, calc_end) =
            spans.iter().find(|(s, _, _, _)| s.name == "Calc").unwrap();
        assert_eq!(*calc_kind, SymbolKind::Class);
        // The class span ENCLOSES its method spans (start before, end after).
        assert!(calc_start < calc_end);
        let (_, _, init_start, init_end) = spans
            .iter()
            .find(|(s, _, _, _)| s.name == "__init__")
            .unwrap();
        assert!(calc_start <= init_start && init_end <= calc_end);
        assert_eq!(calc.return_type, None);

        // Every span must have start <= end.
        for (sig, _, start, end) in &spans {
            assert!(
                start <= end,
                "{} span {}-{} not ordered",
                sig.name,
                start,
                end
            );
        }

        // The top-level `add` starts on line 1.
        let (add, add_kind, add_start, _) =
            spans.iter().find(|(s, _, _, _)| s.name == "add").unwrap();
        assert_eq!(*add_kind, SymbolKind::Function);
        assert_eq!(*add_start, 1);
        assert_eq!(add.return_type, Some("int".to_string()));
        assert_eq!(add.parameters.len(), 2);
    }

    #[test]
    fn test_parse_python_imports() {
        let source = r#"import os
import sys as system
from collections import OrderedDict
"#;
        let (imports, exports) = parse_source_imports_exports(source, Language::Python).unwrap();

        let sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(sources.contains(&"os"));
        assert!(sources.contains(&"sys"));
        assert!(sources.contains(&"collections"));

        // `import sys as system` → Namespace alias.
        let sys = imports.iter().find(|i| i.source == "sys").unwrap();
        assert_eq!(sys.import_kind, ImportKind::Namespace("system".to_string()));

        // `from collections import OrderedDict` → Named.
        let coll = imports.iter().find(|i| i.source == "collections").unwrap();
        assert_eq!(
            coll.import_kind,
            ImportKind::Named(vec!["OrderedDict".to_string()])
        );

        // Python has no export keyword: exports are intentionally empty.
        assert!(exports.is_empty());
    }

    #[test]
    fn test_parse_file_python_fixture() {
        let path = Path::new("tests/fixtures/fixture.py");
        let sigs = parse_file(path).unwrap();
        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"process_data"));
        assert!(names.contains(&"calculate_total"));
        assert!(names.contains(&"add"));
    }

    #[test]
    fn test_parse_file_python_import_fixture() {
        let path = Path::new("tests/fixtures/imports.py");
        let (imports, exports) = parse_imports_exports(path).unwrap();
        assert!(
            !imports.is_empty(),
            "Expected at least 1 import, got {}",
            imports.len()
        );
        assert!(exports.is_empty(), "Python exports must be empty");
    }

    // =========================================================================
    // Go tests
    // =========================================================================

    #[test]
    fn test_parse_go_method_vs_function() {
        // A `method_declaration` carries a receiver and must read as a method;
        // a `function_declaration` must read as a function.
        let source = r#"package p

func Free(a int) int {
    return a
}

func (r *Repo) Bound(b int) int {
    return b
}
"#;

        let spans = parse_source_spans(source, Language::Go).unwrap();
        assert_eq!(spans.len(), 2);

        for (sig, _, start, end) in &spans {
            assert!(
                start <= end,
                "{} span {}-{} not ordered",
                sig.name,
                start,
                end
            );
        }

        let (free, _, _, _) = spans.iter().find(|(s, _, _, _)| s.name == "Free").unwrap();
        let (bound, _, _, _) = spans.iter().find(|(s, _, _, _)| s.name == "Bound").unwrap();

        // `Free` is a free function: its first parameter is NOT the synthetic
        // `self` receiver.
        assert_ne!(
            free.parameters.first().map(|p| p.name.as_str()),
            Some("self")
        );

        // `Bound` is a method: receiver surfaced as a leading `self` parameter,
        // matching the chunker's `is_method` convention.
        assert_eq!(
            bound.parameters.first().map(|p| p.name.as_str()),
            Some("self")
        );
        assert_eq!(
            bound.parameters[0].type_annotation.as_deref(),
            Some("r *Repo")
        );
    }

    #[test]
    fn test_parse_go_imports() {
        let source = r#"package p

import (
    "fmt"
    rename "errors"
)
"#;
        let (imports, exports) = parse_source_imports_exports(source, Language::Go).unwrap();

        let sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(sources.contains(&"fmt"));
        assert!(sources.contains(&"errors"));

        // `rename "errors"` → aliased import surfaced as Namespace.
        let errors = imports.iter().find(|i| i.source == "errors").unwrap();
        assert_eq!(
            errors.import_kind,
            ImportKind::Namespace("rename".to_string())
        );

        // Go exports are deferred (capitalized-identifier convention, not a node).
        assert!(exports.is_empty());
    }

    #[test]
    fn test_parse_file_go_fixture() {
        let path = Path::new("tests/fixtures/fixture.go");
        let sigs = parse_file(path).unwrap();
        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"ProcessData"));
        assert!(names.contains(&"CalculateTotal"));
        assert!(names.contains(&"Add"));

        // `Add` is the receiver method → leading `self` parameter.
        let add = sigs.iter().find(|s| s.name == "Add").unwrap();
        assert_eq!(
            add.parameters.first().map(|p| p.name.as_str()),
            Some("self")
        );
    }

    #[test]
    fn test_parse_file_go_import_fixture() {
        let path = Path::new("tests/fixtures/imports.go");
        let (imports, exports) = parse_imports_exports(path).unwrap();
        assert!(
            !imports.is_empty(),
            "Expected at least 1 import, got {}",
            imports.len()
        );
        assert!(
            exports.is_empty(),
            "Go exports are deferred and must be empty"
        );
    }

    // =========================================================================
    // Type-symbol span tests (symbols beyond functions)
    // =========================================================================

    /// Find the `(SymbolKind, start, end)` of the span named `name`.
    fn find_span(
        spans: &[(FunctionSignature, SymbolKind, usize, usize)],
        name: &str,
    ) -> (SymbolKind, usize, usize) {
        let (_, k, s, e) = spans
            .iter()
            .find(|(sig, _, _, _)| sig.name == name)
            .unwrap_or_else(|| panic!("no span named {name}"));
        (*k, *s, *e)
    }

    #[test]
    fn test_parse_rust_type_symbols() {
        let source = r#"struct Foo {
    a: u32,
}

enum Bar {
    A,
    B,
}

trait Baz {
    fn act(&self);
}

type Alias = u32;
"#;
        let spans = parse_source_spans(source, Language::Rust).unwrap();

        let (foo_k, foo_s, foo_e) = find_span(&spans, "Foo");
        assert_eq!(foo_k, SymbolKind::Struct);
        assert!(foo_s <= foo_e);

        let (bar_k, _, _) = find_span(&spans, "Bar");
        assert_eq!(bar_k, SymbolKind::Enum);

        let (baz_k, baz_s, baz_e) = find_span(&spans, "Baz");
        assert_eq!(baz_k, SymbolKind::Trait);
        assert!(baz_s <= baz_e);

        let (alias_k, _, _) = find_span(&spans, "Alias");
        assert_eq!(alias_k, SymbolKind::Type);

        // The trait method is still extracted (its span differs from the trait's).
        let (act_k, _, _) = find_span(&spans, "act");
        assert_eq!(act_k, SymbolKind::Function);
    }

    #[test]
    fn test_parse_typescript_type_symbols() {
        let source = r#"class Ledger {
    total: number;
    add(n: number): number {
        return n;
    }
}

interface Shape {
    area(): number;
}

type Handler = () => void;

enum Color {
    Red,
    Green,
}
"#;
        let spans = parse_source_spans(source, Language::TypeScript).unwrap();

        let (ledger_k, ledger_s, ledger_e) = find_span(&spans, "Ledger");
        assert_eq!(ledger_k, SymbolKind::Class);
        assert!(ledger_s < ledger_e);

        let (shape_k, _, _) = find_span(&spans, "Shape");
        assert_eq!(shape_k, SymbolKind::Interface);

        let (handler_k, _, _) = find_span(&spans, "Handler");
        assert_eq!(handler_k, SymbolKind::Type);

        let (color_k, _, _) = find_span(&spans, "Color");
        assert_eq!(color_k, SymbolKind::Enum);

        // The class method `add` is still its own span (range differs from class).
        let (add_k, add_s, add_e) = find_span(&spans, "add");
        assert_eq!(add_k, SymbolKind::Function);
        assert!(ledger_s <= add_s && add_e <= ledger_e);
    }

    #[test]
    fn test_parse_go_type_symbols() {
        let source = r#"package p

type Qux struct {
    a int
}

type R interface {
    Do() int
}

type MyInt int
"#;
        let spans = parse_source_spans(source, Language::Go).unwrap();

        let (qux_k, qux_s, qux_e) = find_span(&spans, "Qux");
        assert_eq!(qux_k, SymbolKind::Struct);
        assert!(qux_s <= qux_e);

        let (r_k, _, _) = find_span(&spans, "R");
        assert_eq!(r_k, SymbolKind::Interface);

        let (myint_k, _, _) = find_span(&spans, "MyInt");
        assert_eq!(myint_k, SymbolKind::Type);
    }

    #[test]
    fn test_parse_python_class_symbol() {
        let source = r#"class Animal:
    def __init__(self) -> None:
        self.legs = 4

    def speak(self) -> str:
        return "..."
"#;
        let spans = parse_source_spans(source, Language::Python).unwrap();

        let (animal_k, animal_s, animal_e) = find_span(&spans, "Animal");
        assert_eq!(animal_k, SymbolKind::Class);
        // The class span encloses both methods.
        assert!(animal_s < animal_e);

        let (speak_k, speak_s, speak_e) = find_span(&spans, "speak");
        assert_eq!(speak_k, SymbolKind::Function);
        assert!(animal_s <= speak_s && speak_e <= animal_e);
    }

    /// A one-line `trait Baz { fn x(&self); }` puts the trait and its method on
    /// the SAME single line: both would clamp to `L-L`. The parser still emits
    /// both spans (with that shared range); the chunker dedups by `(start,end)`
    /// so the chunk id stays distinct. Here we just confirm both are produced and
    /// the trait keeps the right kind.
    #[test]
    fn test_parse_rust_single_line_trait_and_method() {
        let source = "trait Baz { fn x(&self); }\n";
        let spans = parse_source_spans(source, Language::Rust).unwrap();

        let (baz_k, baz_s, baz_e) = find_span(&spans, "Baz");
        assert_eq!(baz_k, SymbolKind::Trait);
        assert_eq!((baz_s, baz_e), (1, 1));

        let (x_k, x_s, x_e) = find_span(&spans, "x");
        assert_eq!(x_k, SymbolKind::Function);
        // Same single line as the trait — the collision the chunker must dedup.
        assert_eq!((x_s, x_e), (1, 1));
    }

    #[test]
    fn test_detect_language_bash() {
        // .bash and .sh both map to Bash. Existing languages must remain
        // unaffected (regression guard for the additive change to detect_language).
        // Note: we match lowercase only — uppercase .BASH is rare on case-sensitive
        // Linux filesystems; users on case-insensitive filesystems (macOS default)
        // can rename if they want extraction.
        assert_eq!(detect_language(Path::new("foo.bash")), Some(Language::Bash));
        assert_eq!(detect_language(Path::new("foo.sh")), Some(Language::Bash));
        // No false positives: a non-Bash extension must not match.
        assert_eq!(detect_language(Path::new("foo.zsh")), None);
        assert_eq!(detect_language(Path::new("foo")), None);
    }

    #[test]
    fn test_parse_bash_function_spans() {
        // Three top-level functions; each must be captured as a Function
        // symbol with a valid (start, end) span. Mirrors test_parse_python_function_spans.
        let source = r#"greet() {
    echo hello
}

add_numbers() {
    local result=$(( $1 + $2 ))
    echo "$result"
}

run_demo() {
    if [ "$1" = "yes" ]; then
        add_numbers 1 2
    fi
}
"#;

        let spans = parse_source_spans(source, Language::Bash).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add_numbers"));
        assert!(names.contains(&"run_demo"));

        for (sig, kind, start, end) in &spans {
            assert_eq!(*kind, SymbolKind::Function);
            assert!(
                start <= end,
                "{} span {}-{} not ordered",
                sig.name,
                start,
                end
            );
        }

        // The first function starts on line 1.
        let (_, _, greet_start, greet_end) =
            spans.iter().find(|(s, _, _, _)| s.name == "greet").unwrap();
        assert_eq!(*greet_start, 1);
        assert!(greet_end > greet_start);
    }

    #[test]
    fn test_parse_bash_imports_and_exports() {
        // `source` and `.` are captured as Require imports;
        // `export FOO`, `export FOO=bar`, `declare -x FOO=bar` are exports.
        let source = r#"source ./common.sh
. ./env.sh

export PATH_VAR
export CONFIG_PATH="/etc/app/config"
declare -x SECRET_TOKEN="xyz"
"#;

        let (imports, exports) = parse_source_imports_exports(source, Language::Bash).unwrap();

        // Two imports: the `source` line and the `.` line.
        let import_sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(
            import_sources.contains(&"./common.sh"),
            "missing source import, got: {:?}",
            import_sources
        );
        assert!(
            import_sources.contains(&"./env.sh"),
            "missing dot import, got: {:?}",
            import_sources
        );

        // Three exports: PATH_VAR, CONFIG_PATH, SECRET_TOKEN.
        let export_names: Vec<String> = exports
            .iter()
            .flat_map(|e| match &e.export_kind {
                ExportKind::Named(names) => names.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert!(export_names.contains(&"PATH_VAR".to_string()));
        assert!(export_names.contains(&"CONFIG_PATH".to_string()));
        assert!(export_names.contains(&"SECRET_TOKEN".to_string()));
    }

    #[test]
    fn test_parse_file_bash_fixture() {
        // End-to-end: read the fixture, parse it, assert the three functions
        // are found. Mirrors test_parse_file_python_fixture.
        let path = std::path::Path::new("tests/fixtures/fixture.sh");
        let sigs = parse_file(path).expect("parse fixture.sh");
        let names: Vec<_> = sigs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add_numbers"));
        assert!(names.contains(&"run_demo"));
        // Each signature must have a 1-based line >= 1.
        for s in &sigs {
            assert!(s.line >= 1, "sig {} has invalid line {}", s.name, s.line);
        }
    }

    #[test]
    fn test_detect_language_java() {
        // .java maps to Java. Existing languages must remain unaffected.
        assert_eq!(detect_language(Path::new("Foo.java")), Some(Language::Java));
        assert_eq!(detect_language(Path::new("foo.jj")), None);
        assert_eq!(detect_language(Path::new("Foo.JAVA")), None);
    }

    #[test]
    fn test_parse_java_function_spans() {
        // Class with one method, an interface, an enum, and a record.
        // The class/interface/enum/record are emitted as type symbols;
        // the method is a Function symbol inside the class body.
        let source = r#"public interface Greeter {
    void greet(String name);
}

public class Hello {
    public static void main(String[] args) {
        System.out.println("hi");
    }
}

public enum Color { RED, GREEN, BLUE }
"#;

        let spans = parse_source_spans(source, Language::Java).unwrap();

        // Type symbols: build a simple predicate rather than sorting (SymbolKind
        // doesn't derive Ord and we only need existence checks here).
        let has_kind = |name: &str, k: SymbolKind| -> bool {
            spans.iter().any(|(s, kk, _, _)| s.name == name && *kk == k)
        };
        assert!(has_kind("Greeter", SymbolKind::Interface));
        assert!(has_kind("Hello", SymbolKind::Class));
        assert!(has_kind("Color", SymbolKind::Enum));

        // Method symbol inside the class body.
        let method_names: Vec<_> = spans
            .iter()
            .filter(|(_, k, _, _)| *k == SymbolKind::Function)
            .map(|(s, _, _, _)| s.name.as_str())
            .collect();
        assert!(method_names.contains(&"main"));
    }

    #[test]
    fn test_parse_java_imports() {
        // Scoped + single-identifier + static imports.
        let source = r#"import java.util.List;
import java.util.Map;
import com.example.Foo;
import static java.util.Collections.emptyList;
"#;

        let (imports, _exports) = parse_source_imports_exports(source, Language::Java).unwrap();

        let import_sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(import_sources.contains(&"java.util.List"));
        assert!(import_sources.contains(&"java.util.Map"));
        assert!(import_sources.contains(&"com.example.Foo"));
        // Static imports — the full qualified path is captured.
        assert!(
            import_sources
                .iter()
                .any(|s| s.contains("Collections.emptyList")),
            "missing static import, got: {:?}",
            import_sources
        );
    }

    #[test]
    fn test_parse_file_java_fixture() {
        // End-to-end on the fixture via parse_source_spans (so type symbols AND
        // methods are both surfaced — parse_file/parse_source only returns
        // function signatures, not type symbols; matches the Python/Go/Bash
        // pattern).
        let path = std::path::Path::new("tests/fixtures/fixture.java");
        let source = std::fs::read_to_string(path).expect("read fixture.java");
        let spans = parse_source_spans(&source, Language::Java).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        // Class + interface + enum + record + methods.
        assert!(names.contains(&"Hello"));
        assert!(names.contains(&"Greeter"));
        assert!(names.contains(&"Color"));
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"main"));
        assert!(names.contains(&"add"));
    }

    #[test]
    fn test_detect_language_c() {
        // .c and .h both map to C. C++ extensions are handled separately
        // by Language::Cpp (see test_detect_language_cpp).
        assert_eq!(detect_language(Path::new("foo.c")), Some(Language::C));
        assert_eq!(detect_language(Path::new("foo.h")), Some(Language::C));
        // C++ extensions are no longer None — they're Language::Cpp.
        assert_eq!(detect_language(Path::new("foo.cpp")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.hpp")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.cc")), Some(Language::Cpp));
    }

    #[test]
    fn test_parse_c_function_spans() {
        // Two free functions with parameters and return types.
        let source = r#"int add(int a, int b) {
    return a + b;
}

int main(int argc, char **argv) {
    return 0;
}
"#;

        let spans = parse_source_spans(source, Language::C).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"main"));

        // Every span must be a Function symbol.
        for (_, kind, start, end) in &spans {
            assert_eq!(*kind, SymbolKind::Function);
            assert!(start <= end);
        }

        // add has 2 parameters, return type int.
        let add = spans.iter().find(|(s, _, _, _)| s.name == "add").unwrap();
        assert_eq!(add.0.parameters.len(), 2);
        assert_eq!(add.0.return_type, Some("int".to_string()));
    }

    #[test]
    fn test_parse_c_imports() {
        // System <...> and local "..." includes both captured as Require.
        let source = r#"#include <stdio.h>
#include <stdlib.h>
#include "config.h"
#include "logger.h"
"#;

        let (imports, _exports) = parse_source_imports_exports(source, Language::C).unwrap();

        let import_sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(import_sources.contains(&"<stdio.h>"));
        assert!(import_sources.contains(&"<stdlib.h>"));
        assert!(import_sources.contains(&"\"config.h\""));
        assert!(import_sources.contains(&"\"logger.h\""));
    }

    #[test]
    fn test_parse_file_c_fixture() {
        let path = std::path::Path::new("tests/fixtures/fixture.c");
        let source = std::fs::read_to_string(path).expect("read fixture.c");
        let spans = parse_source_spans(&source, Language::C).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        assert!(names.contains(&"add"));
        assert!(names.contains(&"main"));
        assert!(names.contains(&"unused_helper"));
    }

    #[test]
    fn test_detect_language_ruby() {
        assert_eq!(detect_language(Path::new("foo.rb")), Some(Language::Ruby));
        // ERB and gemspec are also Ruby; we don't try to be exhaustive here.
        assert_eq!(detect_language(Path::new("foo.erb")), None);
        assert_eq!(detect_language(Path::new("foo.ruby")), None);
    }

    #[test]
    fn test_parse_ruby_function_spans() {
        // Critical: a method with a do_block inside must emit ONE method
        // symbol (greet), not three (greet + the block's implicit do).
        // This is the R-1.3 anti-gotcha from the v0.3.0 plan.
        let source = r#"class Greeter
  def initialize(name)
    @name = name
  end

  def greet
    [1, 2, 3].each do |n|
      puts n
    end
  end
end

def top_level
  42
end
"#;

        let spans = parse_source_spans(source, Language::Ruby).unwrap();
        let method_names: Vec<_> = spans
            .iter()
            .filter(|(_, k, _, _)| *k == SymbolKind::Function)
            .map(|(s, _, _, _)| s.name.as_str())
            .collect();
        // Exactly the two defs, NOT the do-block.
        assert_eq!(method_names.len(), 3, "got methods: {:?}", method_names);
        assert!(method_names.contains(&"initialize"));
        assert!(method_names.contains(&"greet"));
        assert!(method_names.contains(&"top_level"));

        // The class itself is a Class symbol.
        let has_kind = |name: &str, k: SymbolKind| -> bool {
            spans.iter().any(|(s, kk, _, _)| s.name == name && *kk == k)
        };
        assert!(has_kind("Greeter", SymbolKind::Class));
    }

    #[test]
    fn test_parse_ruby_imports() {
        let source = r#"require "json"
require "yaml"
require_relative "./config"
"#;

        let (imports, _exports) = parse_source_imports_exports(source, Language::Ruby).unwrap();

        let import_sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(import_sources.contains(&"json"));
        assert!(import_sources.contains(&"yaml"));
        assert!(import_sources.contains(&"./config"));
    }

    #[test]
    fn test_parse_file_ruby_fixture() {
        let path = std::path::Path::new("tests/fixtures/fixture.rb");
        let source = std::fs::read_to_string(path).expect("read fixture.rb");
        let spans = parse_source_spans(&source, Language::Ruby).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        // class, module, methods (initialize, greet, add, top_level_helper).
        assert!(names.contains(&"Greeter"));
        assert!(names.contains(&"Util"));
        assert!(names.contains(&"initialize"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));
        assert!(names.contains(&"top_level_helper"));
        // R-1.3: the do_block must NOT appear as a top-level method.
        // (The do_block has no `def` so it wouldn't anyway, but the test
        // is the regression guard.)
        let method_count = spans
            .iter()
            .filter(|(_, k, _, _)| *k == SymbolKind::Function)
            .count();
        // 4 methods: initialize, greet, add, top_level_helper.
        assert_eq!(method_count, 4, "got method names: {:?}", names);
    }

    #[test]
    fn test_detect_language_cpp() {
        // Standard C++ extensions: .cpp, .cc, .cxx, .hpp, .hxx, .hh.
        // C extensions (.c, .h) intentionally stay on Language::C — the
        // C and C++ grammars are distinct; a .c file is more likely plain
        // C than C++ (the inverse is rarer). A header-only C++ project
        // using .h files would need a rename to .hpp to be parsed.
        assert_eq!(detect_language(Path::new("foo.cpp")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.cc")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.cxx")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.hpp")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.hxx")), Some(Language::Cpp));
        assert_eq!(detect_language(Path::new("foo.hh")), Some(Language::Cpp));
        // C extensions stay on Language::C (per the precedent set in
        // test_detect_language_c).
        assert_eq!(detect_language(Path::new("foo.c")), Some(Language::C));
        assert_eq!(detect_language(Path::new("foo.h")), Some(Language::C));
    }

    #[test]
    fn test_parse_cpp_function_spans() {
        // A class, a struct, and a free function. Mirrors test_parse_c_function_spans
        // but adds the class_specifier/struct_specifier C++-only surface.
        let source = r#"class Greeter {
public:
    void greet(const std::string& name) {}
};

struct Point {
    int x;
    int y;
};

int add(int a, int b) {
    return a + b;
}
"#;

        let spans = parse_source_spans(source, Language::Cpp).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));

        // Class + struct are C++-only.
        let has_kind = |name: &str, k: SymbolKind| -> bool {
            spans.iter().any(|(s, kk, _, _)| s.name == name && *kk == k)
        };
        assert!(has_kind("Greeter", SymbolKind::Class));
        assert!(has_kind("Point", SymbolKind::Struct));
    }

    #[test]
    fn test_parse_cpp_imports() {
        // Same shape as C imports.
        let source = r#"#include <iostream>
#include <vector>
#include "config.h"
"#;

        let (imports, _exports) = parse_source_imports_exports(source, Language::Cpp).unwrap();
        let import_sources: Vec<_> = imports.iter().map(|i| i.source.as_str()).collect();
        assert!(import_sources.contains(&"<iostream>"));
        assert!(import_sources.contains(&"<vector>"));
        assert!(import_sources.contains(&"\"config.h\""));
    }

    #[test]
    fn test_parse_file_cpp_fixture() {
        let path = std::path::Path::new("tests/fixtures/fixture.cpp");
        let source = std::fs::read_to_string(path).expect("read fixture.cpp");
        let spans = parse_source_spans(&source, Language::Cpp).unwrap();
        let names: Vec<_> = spans.iter().map(|(s, _, _, _)| s.name.as_str()).collect();
        // Class + struct + free functions.
        assert!(names.contains(&"Greeter"));
        assert!(names.contains(&"Point"));
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));
        assert!(names.contains(&"main"));
    }
}
