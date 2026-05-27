use std::collections::BTreeSet;

use regex::Regex;
use tree_sitter::{Node, Parser};

use crate::repo_graph::{ParsedRustFile, RustCall, RustSymbol};
use crate::repo_readiness::LanguageKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSitterSupportSummary {
    pub wired: bool,
    pub supported_languages: Vec<String>,
    pub active_languages: Vec<String>,
}

pub fn support_summary_for_languages<I>(languages: I) -> TreeSitterSupportSummary
where
    I: IntoIterator<Item = LanguageKind>,
{
    let mut requested = BTreeSet::new();
    for language in languages {
        requested.insert(language);
    }
    let supported_languages: Vec<String> = requested
        .iter()
        .filter(|kind| language_is_supported(kind))
        .map(|kind| kind.as_str().to_string())
        .collect();
    TreeSitterSupportSummary {
        wired: true,
        active_languages: supported_languages.clone(),
        supported_languages,
    }
}

pub fn language_is_supported(kind: &LanguageKind) -> bool {
    matches!(
        kind,
        LanguageKind::Rust
            | LanguageKind::Python
            | LanguageKind::TypeScript
            | LanguageKind::JavaScript
            | LanguageKind::C
            | LanguageKind::Cpp
            | LanguageKind::Java
            | LanguageKind::Kotlin
            | LanguageKind::Html
            | LanguageKind::Vue
            | LanguageKind::Xml
            | LanguageKind::Json
            | LanguageKind::Yaml
    )
}

pub(crate) fn enrich_parsed_file_with_tree_sitter(
    language: &str,
    text: &str,
    rel_path: &str,
    parsed: &mut ParsedRustFile,
) {
    let Ok(ts_parsed) = parse_source_with_tree_sitter(language, text, rel_path) else {
        return;
    };
    for import_path in ts_parsed.imports {
        if !parsed
            .imports
            .iter()
            .any(|existing| existing == &import_path)
        {
            parsed.imports.push(import_path);
        }
    }
    for ts_symbol in ts_parsed.symbols {
        if let Some(existing) = parsed
            .symbols
            .iter_mut()
            .find(|existing| existing.name == ts_symbol.name && existing.line == ts_symbol.line)
        {
            if existing.kind.is_empty() {
                existing.kind = ts_symbol.kind.clone();
            }
            if existing.signature.is_empty() {
                existing.signature = ts_symbol.signature.clone();
            }
            merge_calls(&mut existing.calls, ts_symbol.calls);
        } else {
            parsed.symbols.push(ts_symbol);
        }
    }
}

pub(crate) fn parse_source_with_tree_sitter_only(
    language: &str,
    text: &str,
    rel_path: &str,
) -> Result<ParsedRustFile, ()> {
    parse_source_with_tree_sitter(language, text, rel_path)
}

fn merge_calls(existing: &mut Vec<RustCall>, incoming: Vec<RustCall>) {
    let mut seen: BTreeSet<(String, usize)> = existing
        .iter()
        .map(|call| (call.name.clone(), call.line))
        .collect();
    for call in incoming {
        let key = (call.name.clone(), call.line);
        if seen.insert(key) {
            existing.push(call);
        }
    }
}

fn parse_source_with_tree_sitter(
    language: &str,
    text: &str,
    rel_path: &str,
) -> Result<ParsedRustFile, ()> {
    let mut parser = Parser::new();
    match language {
        "rust" => parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|_| ())?,
        "python" => parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|_| ())?,
        "typescript" => {
            let language = if rel_path.ends_with(".tsx") {
                tree_sitter_typescript::LANGUAGE_TSX
            } else {
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT
            };
            parser.set_language(&language.into()).map_err(|_| ())?
        }
        "javascript" => parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|_| ())?,
        "c" => parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .map_err(|_| ())?,
        "cpp" => parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .map_err(|_| ())?,
        "java" => parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .map_err(|_| ())?,
        "kotlin" => parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .map_err(|_| ())?,
        "html" => parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .map_err(|_| ())?,
        "vue" => parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .map_err(|_| ())?,
        "xml" => parser
            .set_language(&tree_sitter_xml::LANGUAGE_XML.into())
            .map_err(|_| ())?,
        "json" => parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .map_err(|_| ())?,
        "yaml" => parser
            .set_language(&tree_sitter_yaml::LANGUAGE.into())
            .map_err(|_| ())?,
        _ => return Err(()),
    };
    let tree = parser.parse(text, None).ok_or(())?;
    if tree.root_node().has_error() {
        return Err(());
    }
    let mut parsed = ParsedRustFile::default();
    match language {
        "rust" => walk_rust_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "python" => walk_python_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "typescript" | "javascript" => {
            walk_js_like_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed)
        }
        "c" | "cpp" => walk_c_like_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "java" => walk_java_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "kotlin" => walk_kotlin_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "html" | "vue" => walk_markup_node(
            language,
            tree.root_node(),
            text.as_bytes(),
            rel_path,
            &mut parsed,
        ),
        "xml" => walk_xml_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "json" => walk_json_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        "yaml" => walk_yaml_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed),
        _ => return Err(()),
    }
    match language {
        "rust" => extract_rust_imports(text, &mut parsed.imports),
        "typescript" | "javascript" => extract_js_like_imports(text, &mut parsed.imports),
        "java" | "kotlin" => extract_jvm_imports(text, &mut parsed.imports),
        _ => {}
    }
    if matches!(language, "html" | "vue" | "typescript" | "javascript") {
        extract_tailwind_tokens(text, &mut parsed.imports);
    }
    if matches!(language, "html" | "vue" | "xml") {
        extract_markup_symbols_from_text(
            text,
            rel_path,
            &mut parsed.symbols,
            language == "xml",
        );
    }
    Ok(parsed)
}

fn walk_rust_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "use_declaration" => {
            if let Some(raw) = text_of(node, source) {
                let normalized = raw
                    .trim()
                    .trim_start_matches("use ")
                    .trim_end_matches(';')
                    .to_string();
                push_import(parsed, normalize_import_path(&normalized));
            }
        }
        "function_item" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "function") {
                parsed.symbols.push(symbol);
            }
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(symbol) =
                extract_named_symbol(node, source, rel_path, type_kind(node.kind()))
            {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_rust_node(child, source, rel_path, parsed)
    });
}

fn walk_python_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            if let Some(raw) = text_of(node, source) {
                push_import(parsed, raw.replace('\n', " ").trim().to_string());
            }
        }
        "function_definition" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "function") {
                parsed.symbols.push(symbol);
            }
        }
        "class_definition" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "class") {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_python_node(child, source, rel_path, parsed)
    });
}

fn walk_js_like_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "import_statement" | "export_statement" => {
            if let Some(source_node) = node.child_by_field_name("source") {
                if let Some(raw) = text_of(source_node, source) {
                    push_import(parsed, trim_quotes(&raw));
                }
            } else if let Some(raw) = text_of(node, source) {
                if let Some(module) = extract_quoted_literal(&raw) {
                    push_import(parsed, module);
                }
            }
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "function") {
                parsed.symbols.push(symbol);
            }
        }
        "class_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "class") {
                parsed.symbols.push(symbol);
            }
        }
        "method_definition" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "method") {
                parsed.symbols.push(symbol);
            }
        }
        "interface_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "interface") {
                parsed.symbols.push(symbol);
            }
        }
        "type_alias_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "type") {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_js_like_node(child, source, rel_path, parsed)
    });
}

fn walk_c_like_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "preproc_include" => {
            if let Some(raw) = text_of(node, source) {
                push_import(parsed, raw.trim().to_string());
            }
        }
        "function_definition" => {
            if let Some(symbol) = extract_c_like_function(node, source, rel_path) {
                parsed.symbols.push(symbol);
            }
        }
        "struct_specifier" | "class_specifier" | "enum_specifier" => {
            if let Some(symbol) =
                extract_named_symbol(node, source, rel_path, c_like_type_kind(node.kind()))
            {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_c_like_node(child, source, rel_path, parsed)
    });
}

fn walk_java_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "package_declaration" | "import_declaration" => {
            if let Some(raw) = text_of(node, source) {
                push_import(parsed, raw.replace('\n', " ").trim().to_string());
            }
        }
        "class_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "class") {
                parsed.symbols.push(symbol);
            }
        }
        "interface_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "interface") {
                parsed.symbols.push(symbol);
            }
        }
        "enum_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "enum") {
                parsed.symbols.push(symbol);
            }
        }
        "annotation_type_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "annotation") {
                parsed.symbols.push(symbol);
            }
        }
        "method_declaration" | "constructor_declaration" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "method") {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_java_node(child, source, rel_path, parsed)
    });
}

fn walk_kotlin_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "package_header" | "import_header" => {
            if let Some(raw) = text_of(node, source) {
                push_import(parsed, raw.replace('\n', " ").trim().to_string());
            }
        }
        "class_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "class") {
                parsed.symbols.push(symbol);
            }
        }
        "object_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "object") {
                parsed.symbols.push(symbol);
            }
        }
        "interface_declaration" => {
            if let Some(symbol) = extract_named_symbol(node, source, rel_path, "interface") {
                parsed.symbols.push(symbol);
            }
        }
        "function_declaration" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path, "function") {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }
    recurse(node, |child| {
        walk_kotlin_node(child, source, rel_path, parsed)
    });
}

fn walk_markup_node(
    language: &str,
    node: Node<'_>,
    source: &[u8],
    rel_path: &str,
    parsed: &mut ParsedRustFile,
) {
    match node.kind() {
        "start_tag" | "self_closing_tag" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(name_node, source) {
                    let normalized = name.trim().to_string();
                    if is_interesting_markup_symbol(&normalized) {
                        parsed.symbols.push(RustSymbol {
                            name: normalized.clone(),
                            kind: if is_component_name(&normalized) {
                                "component".into()
                            } else {
                                "element".into()
                            },
                            line: name_node.start_position().row + 1,
                            signature: format!("{rel_path}::{normalized}"),
                            calls: Vec::new(),
                        });
                    }
                }
            }
        }
        _ => {}
    }
    if language == "vue" && node.kind() == "script_element" {
        if let Some(raw) = text_of(node, source) {
            // light-weight import discovery inside <script> blocks
            for line in raw.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("import ") {
                    push_import(parsed, trimmed.to_string());
                }
            }
        }
    }
    recurse(node, |child| {
        walk_markup_node(language, child, source, rel_path, parsed)
    });
}

fn walk_xml_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "STag" | "EmptyElemTag" | "start_tag" | "self_closing_tag" => {
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| first_named_child(node));
            if let Some(name_node) = name_node {
                if let Some(name) = text_of(name_node, source) {
                    let normalized = name.trim_matches(&['<', '>', '/'][..]).trim().to_string();
                    if is_interesting_markup_symbol(&normalized) {
                        parsed.symbols.push(RustSymbol {
                            name: normalized.clone(),
                            kind: "element".into(),
                            line: name_node.start_position().row + 1,
                            signature: format!("{rel_path}::{normalized}"),
                            calls: Vec::new(),
                        });
                    }
                }
            }
        }
        _ => {}
    }
    recurse(node, |child| walk_xml_node(child, source, rel_path, parsed));
}

fn walk_json_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    if node.kind() == "pair" {
        if let Some(key_node) = node
            .child_by_field_name("key")
            .or_else(|| first_named_child(node))
        {
            if let Some(raw) = text_of(key_node, source) {
                let key = trim_quotes(&raw);
                if !key.is_empty() {
                    parsed.symbols.push(RustSymbol {
                        name: key.clone(),
                        kind: "key".into(),
                        line: key_node.start_position().row + 1,
                        signature: format!("{rel_path}::{key}"),
                        calls: Vec::new(),
                    });
                }
            }
        }
    }
    recurse(node, |child| {
        walk_json_node(child, source, rel_path, parsed)
    });
}

fn walk_yaml_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    if matches!(node.kind(), "block_mapping_pair" | "flow_pair") {
        if let Some(key_node) = node
            .child_by_field_name("key")
            .or_else(|| first_named_child(node))
        {
            if let Some(raw) = text_of(key_node, source) {
                let key = raw.trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() {
                    parsed.symbols.push(RustSymbol {
                        name: key.clone(),
                        kind: "key".into(),
                        line: key_node.start_position().row + 1,
                        signature: format!("{rel_path}::{key}"),
                        calls: Vec::new(),
                    });
                }
            }
        }
    }
    recurse(node, |child| {
        walk_yaml_node(child, source, rel_path, parsed)
    });
}

fn recurse(node: Node<'_>, mut f: impl FnMut(Node<'_>)) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        f(child);
    }
}

fn extract_function_symbol(
    node: Node<'_>,
    source: &[u8],
    rel_path: &str,
    kind: &str,
) -> Option<RustSymbol> {
    let name_node = function_name_node(node)?;
    let name = text_of(name_node, source)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let line = name_node.start_position().row + 1;
    let mut calls = Vec::new();
    collect_call_expressions(node, source, &name, &mut calls);
    Some(RustSymbol {
        name: name.clone(),
        kind: kind.into(),
        line,
        signature: format!("{rel_path}::{name}"),
        calls,
    })
}

fn extract_c_like_function(node: Node<'_>, source: &[u8], rel_path: &str) -> Option<RustSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name_node = find_identifier_like_node(declarator, source)?;
    let name = text_of(name_node, source)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let line = name_node.start_position().row + 1;
    let mut calls = Vec::new();
    collect_call_expressions(node, source, &name, &mut calls);
    Some(RustSymbol {
        name: name.clone(),
        kind: "function".into(),
        line,
        signature: format!("{rel_path}::{name}"),
        calls,
    })
}

fn extract_named_symbol(
    node: Node<'_>,
    source: &[u8],
    rel_path: &str,
    kind: &str,
) -> Option<RustSymbol> {
    let name_node = named_symbol_node(node, source)?;
    let name = text_of(name_node, source)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(RustSymbol {
        name: name.clone(),
        kind: kind.to_string(),
        line: name_node.start_position().row + 1,
        signature: format!("{rel_path}::{name}"),
        calls: Vec::new(),
    })
}

fn function_name_node(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("name")
        .or_else(|| {
            node.child_by_field_name("declarator")
                .and_then(|n| find_identifier_like_node(n, &[]))
        })
        .or_else(|| find_identifier_like_node(node, &[]))
}

fn named_symbol_node<'a>(node: Node<'a>, _source: &[u8]) -> Option<Node<'a>> {
    node.child_by_field_name("name")
        .or_else(|| find_identifier_like_node(node, &[]))
}

fn find_identifier_like_node<'a>(node: Node<'a>, source: &[u8]) -> Option<Node<'a>> {
    if is_identifier_kind(node.kind()) {
        return Some(node);
    }
    if matches!(
        node.kind(),
        "scoped_identifier" | "qualified_identifier" | "field_expression" | "member_expression"
    ) {
        if let Some(name) = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("field"))
        {
            if is_identifier_kind(name.kind()) {
                return Some(name);
            }
            if let Some(deeper) = find_identifier_like_node(name, source) {
                return Some(deeper);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_identifier_like_node(child, source) {
            return Some(found);
        }
    }
    None
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "simple_identifier"
            | "name"
            | "tag_name"
    )
}

fn collect_call_expressions(
    node: Node<'_>,
    source: &[u8],
    owner_name: &str,
    out: &mut Vec<RustCall>,
) {
    if matches!(
        node.kind(),
        "call_expression" | "method_invocation" | "call" | "navigation_expression"
    ) {
        if let Some(function_node) = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| first_named_child(node))
        {
            if let Some(name) = callable_name(function_node, source) {
                if !crate::repo_graph::should_skip_call_name(&name) && name != owner_name {
                    let line = function_node.start_position().row + 1;
                    let key = (name.clone(), line);
                    if !out
                        .iter()
                        .any(|call| call.name == key.0 && call.line == key.1)
                    {
                        out.push(RustCall {
                            name: key.0,
                            line: key.1,
                        });
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_call_expressions(child, source, owner_name, out);
    }
}

fn callable_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        kind if is_identifier_kind(kind) => Some(text_of(node, source)?.to_string()),
        "string" => None,
        "scoped_identifier" | "qualified_identifier" => node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("field"))
            .and_then(|name| callable_name(name, source)),
        "field_expression" | "member_expression" => node
            .child_by_field_name("field")
            .or_else(|| node.child_by_field_name("property"))
            .or_else(|| node.child_by_field_name("name"))
            .and_then(|field| callable_name(field, source)),
        "navigation_expression"
        | "call_suffix"
        | "reference_expression"
        | "await_expression"
        | "try_expression"
        | "parenthesized_expression"
        | "generic_function" => {
            let mut cursor = node.walk();
            let result = node
                .children(&mut cursor)
                .find_map(|child| callable_name(child, source));
            result
        }
        _ => {
            let mut cursor = node.walk();
            let result = node
                .children(&mut cursor)
                .find_map(|child| callable_name(child, source));
            result
        }
    }
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let result = node.named_children(&mut cursor).next();
    result
}

fn text_of(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.utf8_text(source).ok().map(|s| s.to_string())
}

fn normalize_import_path(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ::", "::")
        .replace(":: ", "::")
}

fn trim_quotes(raw: &str) -> String {
    raw.trim().trim_matches('"').trim_matches('\'').to_string()
}

fn extract_quoted_literal(raw: &str) -> Option<String> {
    let start = raw.find(['"', '\''])?;
    let quote = raw.as_bytes()[start] as char;
    let rest = &raw[start + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn push_import(parsed: &mut ParsedRustFile, import_path: String) {
    if import_path.is_empty()
        || parsed
            .imports
            .iter()
            .any(|existing| existing == &import_path)
    {
        return;
    }
    parsed.imports.push(import_path);
}

fn type_kind(kind: &str) -> &'static str {
    match kind {
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        _ => "type",
    }
}

fn c_like_type_kind(kind: &str) -> &'static str {
    match kind {
        "struct_specifier" => "struct",
        "class_specifier" => "class",
        "enum_specifier" => "enum",
        _ => "type",
    }
}

fn is_component_name(name: &str) -> bool {
    name.contains('-')
        || name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
}

fn is_interesting_markup_symbol(name: &str) -> bool {
    !name.is_empty()
        && (is_component_name(name)
            || matches!(name, "template" | "script" | "style" | "body" | "svg"))
}

fn extract_tailwind_tokens(text: &str, imports: &mut Vec<String>) {
    let class_re =
        Regex::new(r#"(?:class|className)\s*=\s*["']([^"']+)["']"#).expect("valid class regex");
    let mut seen: BTreeSet<String> = imports.iter().cloned().collect();
    for caps in class_re.captures_iter(text) {
        for token in caps[1].split_whitespace() {
            if token.contains(':') || token.contains('-') {
                let module = format!("tailwind:{token}");
                if seen.insert(module.clone()) {
                    imports.push(module);
                }
            }
        }
    }
}

fn extract_rust_imports(text: &str, imports: &mut Vec<String>) {
    let import_re = Regex::new(r"(?m)^\s*use\s+([^;]+);").expect("valid rust import regex");
    let mut seen: BTreeSet<String> = imports.iter().cloned().collect();
    for caps in import_re.captures_iter(text) {
        let import_path = normalize_import_path(&caps[1]);
        if seen.insert(import_path.clone()) {
            imports.push(import_path);
        }
    }
}

fn extract_js_like_imports(text: &str, imports: &mut Vec<String>) {
    let import_re = Regex::new(
        r#"(?m)^\s*(?:import|export)\b[^"'`\n]*["']([^"']+)["']"#,
    )
    .expect("valid js import regex");
    let mut seen: BTreeSet<String> = imports.iter().cloned().collect();
    for caps in import_re.captures_iter(text) {
        let import_path = caps[1].to_string();
        if seen.insert(import_path.clone()) {
            imports.push(import_path);
        }
    }
}

fn extract_jvm_imports(text: &str, imports: &mut Vec<String>) {
    let import_re =
        Regex::new(r"(?m)^\s*(?:package|import)\s+([A-Za-z0-9_.*]+)").expect("valid jvm import regex");
    let mut seen: BTreeSet<String> = imports.iter().cloned().collect();
    for caps in import_re.captures_iter(text) {
        let import_path = caps[0].trim().to_string();
        if seen.insert(import_path.clone()) {
            imports.push(import_path);
        }
    }
}

fn extract_markup_symbols_from_text(
    text: &str,
    rel_path: &str,
    symbols: &mut Vec<RustSymbol>,
    allow_all_tags: bool,
) {
    let tag_re = Regex::new(r#"<([A-Za-z][A-Za-z0-9:_-]*)"#).expect("valid markup tag regex");
    let mut seen: BTreeSet<(String, usize)> = symbols
        .iter()
        .map(|symbol| (symbol.name.clone(), symbol.line))
        .collect();
    for caps in tag_re.captures_iter(text) {
        let Some(full) = caps.get(0) else { continue };
        let Some(name) = caps.get(1) else { continue };
        let normalized = name.as_str().to_string();
        if !allow_all_tags && !is_interesting_markup_symbol(&normalized) {
            continue;
        }
        let line = text[..full.start()].bytes().filter(|b| *b == b'\n').count() + 1;
        if seen.insert((normalized.clone(), line)) {
            symbols.push(RustSymbol {
                name: normalized.clone(),
                kind: if is_component_name(&normalized) {
                    "component".into()
                } else {
                    "element".into()
                },
                line,
                signature: format!("{rel_path}::{normalized}"),
                calls: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_sitter_support_marks_common_languages_as_supported() {
        let summary = support_summary_for_languages([
            LanguageKind::Rust,
            LanguageKind::Python,
            LanguageKind::TypeScript,
            LanguageKind::Json,
            LanguageKind::Yaml,
        ]);
        assert!(summary.wired);
        assert_eq!(
            summary.supported_languages,
            vec!["rust", "python", "typescript", "json", "yaml"]
        );
    }

    #[test]
    fn tree_sitter_enrichment_extracts_rust_symbols_and_calls() {
        let text = r#"
        use crate::worker::run_job;

        pub extern "C" fn api_entry() {
            run_job();
        }

        pub struct Demo;
        "#;
        let mut parsed = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter("rust", text, "src/lib.rs", &mut parsed);
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.contains("run_job")));
        assert!(parsed
            .symbols
            .iter()
            .any(|symbol| symbol.name == "api_entry"));
        assert!(parsed.symbols.iter().any(|symbol| symbol.name == "Demo"));
        let entry = parsed
            .symbols
            .iter()
            .find(|symbol| symbol.name == "api_entry")
            .unwrap();
        assert!(entry.calls.iter().any(|call| call.name == "run_job"));
    }

    #[test]
    fn tree_sitter_enrichment_extracts_python_symbols() {
        let text = r#"
import os
from demo.worker import run_job

class Demo:
    pass

def api_entry():
    run_job()
"#;
        let mut parsed = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter("python", text, "demo.py", &mut parsed);
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.contains("import os")));
        assert!(parsed.symbols.iter().any(|symbol| symbol.name == "Demo"));
        let entry = parsed
            .symbols
            .iter()
            .find(|symbol| symbol.name == "api_entry")
            .unwrap();
        assert!(entry.calls.iter().any(|call| call.name == "run_job"));
    }

    #[test]
    fn tree_sitter_enrichment_extracts_typescript_and_tailwind_tokens() {
        let text = r#"
import { runJob } from './worker';

export function apiEntry() {
  runJob();
}

export class Demo {}
const cls = <div className="px-4 text-sm md:flex"></div>;
"#;
        let mut parsed = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter("typescript", text, "src/demo.tsx", &mut parsed);
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.contains("./worker")));
        assert!(parsed
            .imports
            .iter()
            .any(|import| import == "tailwind:px-4"));
        assert!(parsed
            .symbols
            .iter()
            .any(|symbol| symbol.name == "apiEntry"));
        assert!(parsed.symbols.iter().any(|symbol| symbol.name == "Demo"));
    }

    #[test]
    fn tree_sitter_enrichment_extracts_json_and_yaml_keys() {
        let mut json = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter("json", r#"{ "name": "demo", "scripts": {} }"#, "package.json", &mut json);
        assert!(json.symbols.iter().any(|symbol| symbol.name == "name"));
        assert!(json.symbols.iter().any(|symbol| symbol.name == "scripts"));

        let mut yaml = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "yaml",
            "apiVersion: v1\nkind: Deployment\nmetadata:\n  name: demo\n",
            "deploy.yaml",
            &mut yaml,
        );
        assert!(yaml
            .symbols
            .iter()
            .any(|symbol| symbol.name == "apiVersion"));
        assert!(yaml.symbols.iter().any(|symbol| symbol.name == "metadata"));
    }

    #[test]
    fn tree_sitter_enrichment_extracts_c_family_and_jvm_symbols() {
        let mut c = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "c",
            "#include <stdio.h>\nstruct Demo { int x; };\nint helper(){ puts(\"hi\"); return 0; }\n",
            "demo.c",
            &mut c,
        );
        assert!(c.imports.iter().any(|import| import.contains("#include")));
        assert!(c.symbols.iter().any(|symbol| symbol.name == "Demo"));
        assert!(c.symbols.iter().any(|symbol| symbol.name == "helper"));

        let mut cpp = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "cpp",
            "#include <vector>\nclass Demo {};\nint helper(){ runJob(); return 0; }\n",
            "demo.cpp",
            &mut cpp,
        );
        assert!(cpp.symbols.iter().any(|symbol| symbol.name == "Demo"));
        assert!(cpp.symbols.iter().any(|symbol| symbol.name == "helper"));

        let mut java = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "java",
            "package demo;\nimport java.util.List;\nclass Demo { void run(){ helper(); } }\n",
            "Demo.java",
            &mut java,
        );
        assert!(java.imports.iter().any(|import| import.contains("package demo")));
        assert!(java.symbols.iter().any(|symbol| symbol.name == "Demo"));

        let mut kotlin = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "kotlin",
            "package demo\nimport foo.bar\nclass Demo\nfun run(){ helper() }\n",
            "Demo.kt",
            &mut kotlin,
        );
        assert!(kotlin.imports.iter().any(|import| import.contains("import foo.bar")));
        assert!(kotlin.symbols.iter().any(|symbol| symbol.name == "Demo"));
    }

    #[test]
    fn tree_sitter_enrichment_extracts_markup_symbols() {
        let mut html = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "html",
            "<body><app-shell class=\"px-4 text-sm\"></app-shell></body>",
            "index.html",
            &mut html,
        );
        assert!(html.symbols.iter().any(|symbol| symbol.name == "app-shell"));
        assert!(html.imports.iter().any(|import| import == "tailwind:px-4"));

        let mut vue = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter(
            "vue",
            "<template><AppShell class=\"px-4\"></AppShell></template><script setup>const a = 1;</script>",
            "App.vue",
            &mut vue,
        );
        assert!(vue.symbols.iter().any(|symbol| symbol.name == "AppShell"));

        let mut xml = ParsedRustFile::default();
        enrich_parsed_file_with_tree_sitter("xml", "<root><item /></root>", "demo.xml", &mut xml);
        assert!(xml.symbols.iter().any(|symbol| symbol.name == "root"));
    }
}
