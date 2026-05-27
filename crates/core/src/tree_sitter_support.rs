use std::collections::BTreeSet;

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
    matches!(kind, LanguageKind::Rust)
}

pub(crate) fn enrich_parsed_rust_file_with_tree_sitter(
    text: &str,
    rel_path: &str,
    parsed: &mut ParsedRustFile,
) {
    let Ok(ts_parsed) = parse_rust_with_tree_sitter(text, rel_path) else {
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

fn parse_rust_with_tree_sitter(text: &str, rel_path: &str) -> Result<ParsedRustFile, ()> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| ())?;
    let tree = parser.parse(text, None).ok_or(())?;
    if tree.root_node().has_error() {
        return Err(());
    }
    let mut parsed = ParsedRustFile::default();
    walk_rust_node(tree.root_node(), text.as_bytes(), rel_path, &mut parsed);
    Ok(parsed)
}

fn walk_rust_node(node: Node<'_>, source: &[u8], rel_path: &str, parsed: &mut ParsedRustFile) {
    match node.kind() {
        "use_declaration" => {
            if let Some(argument) = node.child_by_field_name("argument") {
                if let Ok(raw) = argument.utf8_text(source) {
                    let import_path = raw
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .replace(" ::", "::")
                        .replace(":: ", "::");
                    if !import_path.is_empty() {
                        parsed.imports.push(import_path);
                    }
                }
            }
        }
        "function_item" => {
            if let Some(symbol) = extract_function_symbol(node, source, rel_path) {
                parsed.symbols.push(symbol);
            }
        }
        "struct_item" | "enum_item" | "trait_item" => {
            if let Some(symbol) = extract_type_symbol(node, source, rel_path) {
                parsed.symbols.push(symbol);
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_node(child, source, rel_path, parsed);
    }
}

fn extract_function_symbol(node: Node<'_>, source: &[u8], rel_path: &str) -> Option<RustSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();
    let line = name_node.start_position().row + 1;
    let mut calls = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        collect_call_expressions(body, source, &name, &mut calls);
    }
    Some(RustSymbol {
        name: name.clone(),
        kind: "function".into(),
        line,
        signature: format!("{rel_path}::{name}"),
        calls,
    })
}

fn extract_type_symbol(node: Node<'_>, source: &[u8], rel_path: &str) -> Option<RustSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();
    let kind = match node.kind() {
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        _ => return None,
    };
    Some(RustSymbol {
        name: name.clone(),
        kind: kind.to_string(),
        line: name_node.start_position().row + 1,
        signature: format!("{rel_path}::{name}"),
        calls: Vec::new(),
    })
}

fn collect_call_expressions(
    node: Node<'_>,
    source: &[u8],
    owner_name: &str,
    out: &mut Vec<RustCall>,
) {
    if node.kind() == "call_expression" {
        if let Some(function_node) = node.child_by_field_name("function") {
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
        "identifier" | "field_identifier" => Some(node.utf8_text(source).ok()?.to_string()),
        "scoped_identifier" => node
            .child_by_field_name("name")
            .and_then(|name| callable_name(name, source)),
        "field_expression" => node
            .child_by_field_name("field")
            .and_then(|field| callable_name(field, source)),
        "generic_function" => {
            let mut cursor = node.walk();
            let match_child = node
                .children(&mut cursor)
                .find_map(|child| callable_name(child, source));
            match_child
        }
        "reference_expression"
        | "await_expression"
        | "try_expression"
        | "parenthesized_expression" => {
            let mut cursor = node.walk();
            let match_child = node
                .children(&mut cursor)
                .find_map(|child| callable_name(child, source));
            match_child
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_sitter_support_marks_rust_as_supported() {
        let summary = support_summary_for_languages([LanguageKind::Rust, LanguageKind::Python]);
        assert!(summary.wired);
        assert_eq!(summary.supported_languages, vec!["rust"]);
        assert_eq!(summary.active_languages, vec!["rust"]);
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
        enrich_parsed_rust_file_with_tree_sitter(text, "src/lib.rs", &mut parsed);
        assert!(parsed
            .imports
            .iter()
            .any(|value| value.contains("crate::worker::run_job")));
        let function = parsed
            .symbols
            .iter()
            .find(|symbol| symbol.name == "api_entry")
            .expect("function symbol");
        assert!(function.calls.iter().any(|call| call.name == "run_job"));
        assert!(parsed
            .symbols
            .iter()
            .any(|symbol| symbol.name == "Demo" && symbol.kind == "struct"));
    }
}
