//! Language aware symbol search for `fs.find_definition` and `fs.find_references`.
//! Port of the C# trio `Core/TreeSitterSymbols.cs` (real grammars),
//! `Core/SymbolIndex.cs` (lexical fallback) and `Core/CodeSearch.cs` (the facade
//! that prefers the grammar and degrades to the fallback).
//!
//! The grammars are linked statically here, so unlike the C# there is no
//! "grammar unavailable on this platform" case for the ten supported languages.
//! The lexical fallback is still exercised on every file whose extension has no
//! grammar mapping, which keeps behaviour identical for callers.

use crate::util::text::split_lines;
use regex::Regex;
use std::sync::OnceLock;
use tree_sitter::{Language, Node, Parser};

/// A definition hit. Same field set as the C# `CodeMatch`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Definition {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
}

/// A reference hit. Carries `name` too so callers can build either the C#
/// `find_definition` payload (path/name/kind/line) or the `find_references`
/// payload (path/line/kind) without a second lookup.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Reference {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: usize,
}

/// Extension to language, in the C# declaration order. `.h` maps to `c` exactly
/// like the C# table (a C++ header is parsed as C, which is good enough for
/// declarations and keeps the mapping single valued).
const EXTENSION_LANGUAGE: &[(&str, &str)] = &[
    (".py", "python"),
    (".js", "javascript"),
    (".jsx", "javascript"),
    (".ts", "typescript"),
    (".tsx", "tsx"),
    (".go", "go"),
    (".rs", "rust"),
    (".java", "java"),
    (".c", "c"),
    (".h", "c"),
    (".cpp", "cpp"),
    (".rb", "ruby"),
];

/// Definition node kinds per language, matching the C# `DefinitionKinds` map.
const DEFINITION_KINDS: &[(&str, &[&str])] = &[
    ("python", &["function_definition", "class_definition"]),
    (
        "javascript",
        &["function_declaration", "class_declaration", "method_definition", "variable_declarator"],
    ),
    (
        "typescript",
        &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
        ],
    ),
    (
        "tsx",
        &[
            "function_declaration",
            "class_declaration",
            "method_definition",
            "interface_declaration",
        ],
    ),
    ("go", &["function_declaration", "method_declaration", "type_spec"]),
    ("rust", &["function_item", "struct_item", "enum_item", "trait_item"]),
    ("java", &["method_declaration", "class_declaration", "interface_declaration"]),
    ("c", &["function_definition", "struct_specifier"]),
    ("cpp", &["function_definition", "class_specifier", "struct_specifier"]),
    ("ruby", &["method", "class", "module"]),
];

/// The language of a file, or `None` when no grammar and no lexical patterns
/// apply. Callers skip such files entirely (the C# does the same).
pub fn language_for(path: &str) -> Option<&'static str> {
    // Longest extension first so `.tsx` is not swallowed by a `.ts` suffix test.
    let mut best: Option<(&'static str, usize)> = None;
    for (ext, lang) in EXTENSION_LANGUAGE {
        if path.ends_with(ext) && best.is_none_or(|(_, len)| ext.len() > len) {
            best = Some((lang, ext.len()));
        }
    }
    best.map(|(lang, _)| lang)
}

fn grammar(language: &str) -> Option<Language> {
    match language {
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "javascript" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "c" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "ruby" => Some(tree_sitter_ruby::LANGUAGE.into()),
        _ => None,
    }
}

/// True when a real grammar backs this file, i.e. results come from the parser
/// and not from the lexical fallback.
pub fn grammar_available(path: &str) -> bool {
    language_for(path).and_then(grammar).is_some()
}

fn definition_kinds(language: &str) -> Option<&'static [&'static str]> {
    DEFINITION_KINDS.iter().find(|(l, _)| *l == language).map(|(_, k)| *k)
}

/// Definitions of `name` in `source`. An empty `name` matches any name (the C#
/// passes `null` for that case); `kind` is a substring filter on the node kind.
///
/// Falls back to the lexical matcher when the file has no grammar or when the
/// parser cannot produce a tree.
pub fn find_definitions(
    path: &str,
    source: &str,
    name: &str,
    kind: Option<&str>,
) -> Vec<Definition> {
    let Some(language) = language_for(path) else {
        return Vec::new();
    };
    let Some(kinds) = definition_kinds(language) else {
        return Vec::new();
    };
    match grammar(language).and_then(|g| parse(&g, source)) {
        Some(tree) => {
            let mut out = Vec::new();
            let bytes = source.as_bytes();
            walk(tree.root_node(), &mut |node| {
                let node_kind = node.kind();
                if !kinds.contains(&node_kind) {
                    return;
                }
                let Some(found) = name_of(node, bytes) else { return };
                let name_ok = name.is_empty() || found == name;
                let kind_ok = kind.is_none_or(|k| node_kind.contains(k));
                if name_ok && kind_ok {
                    out.push(Definition {
                        path: path.to_string(),
                        name: found,
                        kind: node_kind.to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            });
            out
        }
        None => lexical_definitions(path, source, name, kind),
    }
}

/// References to `name` in `source`: every identifier token equal to `name`.
pub fn find_references(path: &str, source: &str, name: &str) -> Vec<Reference> {
    let Some(language) = language_for(path) else {
        return Vec::new();
    };
    match grammar(language).and_then(|g| parse(&g, source)) {
        Some(tree) => {
            let mut out = Vec::new();
            let bytes = source.as_bytes();
            walk(tree.root_node(), &mut |node| {
                if !node.kind().ends_with("identifier") {
                    return;
                }
                if node.utf8_text(bytes).ok() != Some(name) {
                    return;
                }
                out.push(Reference {
                    path: path.to_string(),
                    name: name.to_string(),
                    kind: node.kind().to_string(),
                    line: node.start_position().row + 1,
                });
            });
            out
        }
        None => lexical_references(path, source, name),
    }
}

fn parse(language: &Language, source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(source, None)
}

/// Visit the root and every NAMED descendant, exactly like the C# `Walk`
/// (anonymous tokens such as punctuation carry no symbol information). An explicit
/// stack, not recursion: a source nested 100 000 levels would overflow the 2 MiB
/// thread stack (SPEC-0092 DEC-007). Children are pushed reversed so the leftmost
/// pops next, which is the pre order of the recursive walk.
/// Time O(n) in the nodes, space O(depth + widest fan out) on the heap.
fn walk<'t, F: FnMut(Node<'t>)>(root: Node<'t>, visit: &mut F) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        visit(node);
        push_named_children(node, &mut stack);
    }
}

fn push_named_children<'t>(node: Node<'t>, stack: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
    stack.extend(children.into_iter().rev());
}

/// Name of a definition node: the `name` field when the grammar has one,
/// otherwise the first identifier in the subtree (C and C++ hide the name inside
/// the declarator subtree).
fn name_of(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(named) = node.child_by_field_name("name")
        && let Ok(text) = named.utf8_text(source)
    {
        return Some(text.to_string());
    }
    let mut found = None;
    walk_descendants(node, &mut |d| {
        if matches!(d.kind(), "identifier" | "field_identifier" | "type_identifier")
            && let Ok(text) = d.utf8_text(source)
        {
            found = Some(text.to_string());
            return false;
        }
        true
    });
    found
}

/// Depth first over named descendants, excluding the node itself, in the pre order of
/// the C# `Descendants` iterator so "first identifier" means the same thing. `visit`
/// returns `false` to stop. Time O(visited), space as [`walk`].
fn walk_descendants<'t>(node: Node<'t>, visit: &mut impl FnMut(Node<'t>) -> bool) {
    let mut stack = Vec::new();
    push_named_children(node, &mut stack);
    while let Some(next) = stack.pop() {
        if !visit(next) {
            return;
        }
        push_named_children(next, &mut stack);
    }
}

// ── lexical fallback (port of Core/SymbolIndex.cs) ───────────────────────────

struct LexPattern {
    kind: &'static str,
    rx: Regex,
}

/// Per language regex sets. Built once: compiling these on every file would
/// dominate the cost of a repository wide scan.
fn lex_patterns() -> &'static Vec<(&'static str, Vec<LexPattern>)> {
    static PATTERNS: OnceLock<Vec<(&'static str, Vec<LexPattern>)>> = OnceLock::new();
    PATTERNS.get_or_init(build_lex_patterns)
}

fn build_lex_patterns() -> Vec<(&'static str, Vec<LexPattern>)> {
    const ID: &str = r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)";
    // A bad pattern here is a programming error, so panicking at first use is
    // the right failure mode: it shows up on the very first test run.
    let p = |kind: &'static str, pattern: String| LexPattern {
        kind,
        rx: Regex::new(&pattern).expect("lexical symbol pattern must compile"),
    };
    vec![
        (
            "python",
            vec![
                p("function_definition", format!(r"^\s*(?:async\s+)?def\s+{ID}\s*\(")),
                p("class_definition", format!(r"^\s*class\s+{ID}\b")),
            ],
        ),
        (
            "javascript",
            vec![
                p(
                    "function_declaration",
                    format!(r"^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*{ID}\s*\("),
                ),
                p("class_declaration", format!(r"^\s*(?:export\s+)?class\s+{ID}\b")),
                p(
                    "variable_declarator",
                    format!(r"^\s*(?:export\s+)?(?:const|let|var)\s+{ID}\s*="),
                ),
                p(
                    "method_definition",
                    format!(r"^\s*(?:static\s+)?(?:async\s+)?{ID}\s*\([^)]*\)\s*\{{"),
                ),
            ],
        ),
        (
            "typescript",
            vec![
                p(
                    "function_declaration",
                    format!(r"^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*{ID}\s*[<(]"),
                ),
                p(
                    "class_declaration",
                    format!(r"^\s*(?:export\s+)?(?:abstract\s+)?class\s+{ID}\b"),
                ),
                p("interface_declaration", format!(r"^\s*(?:export\s+)?interface\s+{ID}\b")),
                p(
                    "method_definition",
                    format!(
                        r"^\s*(?:public|private|protected|static|async|\s)*{ID}\s*\([^)]*\)\s*[:{{]"
                    ),
                ),
            ],
        ),
        (
            "tsx",
            vec![
                p(
                    "function_declaration",
                    format!(r"^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*{ID}\s*[<(]"),
                ),
                p(
                    "class_declaration",
                    format!(r"^\s*(?:export\s+)?(?:abstract\s+)?class\s+{ID}\b"),
                ),
                p("interface_declaration", format!(r"^\s*(?:export\s+)?interface\s+{ID}\b")),
            ],
        ),
        (
            "go",
            vec![
                p("function_declaration", format!(r"^\s*func\s+{ID}\s*\(")),
                p("method_declaration", format!(r"^\s*func\s*\([^)]*\)\s*{ID}\s*\(")),
                p("type_spec", format!(r"^\s*type\s+{ID}\b")),
            ],
        ),
        (
            "rust",
            vec![
                p("function_item", format!(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+{ID}\b")),
                p("struct_item", format!(r"^\s*(?:pub\s+)?struct\s+{ID}\b")),
                p("enum_item", format!(r"^\s*(?:pub\s+)?enum\s+{ID}\b")),
                p("trait_item", format!(r"^\s*(?:pub\s+)?trait\s+{ID}\b")),
            ],
        ),
        (
            "java",
            vec![
                p(
                    "class_declaration",
                    format!(
                        r"^\s*(?:public|private|protected|static|final|abstract|\s)*class\s+{ID}\b"
                    ),
                ),
                p(
                    "interface_declaration",
                    format!(r"^\s*(?:public|private|protected|\s)*interface\s+{ID}\b"),
                ),
                p(
                    "method_declaration",
                    format!(
                        r"^\s*(?:public|private|protected|static|final|abstract|synchronized|\s)+[\w<>\[\],.\s]+\s+{ID}\s*\([^)]*\)\s*[{{;]"
                    ),
                ),
            ],
        ),
        (
            "c",
            vec![
                p(
                    "function_definition",
                    format!(r"^\s*[\w\*\s]+?\s+\*?{ID}\s*\([^;]*\)\s*\{{?\s*$"),
                ),
                p("struct_specifier", format!(r"^\s*struct\s+{ID}\b")),
            ],
        ),
        (
            "cpp",
            vec![
                p(
                    "function_definition",
                    format!(r"^\s*[\w\*\s:<>,]+?\s+\*?{ID}\s*\([^;]*\)\s*\{{?\s*$"),
                ),
                p("class_specifier", format!(r"^\s*class\s+{ID}\b")),
                p("struct_specifier", format!(r"^\s*struct\s+{ID}\b")),
            ],
        ),
        (
            "ruby",
            vec![
                p("method", format!(r"^\s*def\s+(?:self\.)?{ID}\b")),
                p("class", format!(r"^\s*class\s+{ID}\b")),
                p("module", format!(r"^\s*module\s+{ID}\b")),
            ],
        ),
    ]
}

fn identifier_rx() -> &'static Regex {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| {
        Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("identifier pattern must compile")
    })
}

/// Line oriented definition search. Public so callers can force the fallback
/// (the tests do, to prove the two paths agree on shape).
pub fn lexical_definitions(
    path: &str,
    source: &str,
    name: &str,
    kind: Option<&str>,
) -> Vec<Definition> {
    let Some(language) = language_for(path) else {
        return Vec::new();
    };
    let Some((_, patterns)) = lex_patterns().iter().find(|(l, _)| *l == language) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, line) in split_lines(source).iter().enumerate() {
        for pattern in patterns {
            let Some(m) = pattern.rx.captures(line) else { continue };
            let found = m.name("name").map(|g| g.as_str()).unwrap_or_default();
            let name_ok = name.is_empty() || found == name;
            let kind_ok = kind.is_none_or(|k| pattern.kind.contains(k));
            if name_ok && kind_ok {
                out.push(Definition {
                    path: path.to_string(),
                    name: found.to_string(),
                    kind: pattern.kind.to_string(),
                    line: i + 1,
                });
            }
        }
    }
    out
}

/// Line oriented reference search: every identifier token equal to `name`,
/// reported with kind `identifier` like the C#.
pub fn lexical_references(path: &str, source: &str, name: &str) -> Vec<Reference> {
    if language_for(path).is_none() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, line) in split_lines(source).iter().enumerate() {
        for m in identifier_rx().find_iter(line) {
            if m.as_str() == name {
                out.push(Reference {
                    path: path.to_string(),
                    name: name.to_string(),
                    kind: "identifier".to_string(),
                    line: i + 1,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_mapping_covers_the_csharp_table() {
        assert_eq!(language_for("/a/b.py"), Some("python"));
        assert_eq!(language_for("/a/b.js"), Some("javascript"));
        assert_eq!(language_for("/a/b.jsx"), Some("javascript"));
        assert_eq!(language_for("/a/b.ts"), Some("typescript"));
        assert_eq!(language_for("/a/b.tsx"), Some("tsx"));
        assert_eq!(language_for("/a/b.go"), Some("go"));
        assert_eq!(language_for("/a/b.rs"), Some("rust"));
        assert_eq!(language_for("/a/b.java"), Some("java"));
        assert_eq!(language_for("/a/b.c"), Some("c"));
        assert_eq!(language_for("/a/b.h"), Some("c"));
        assert_eq!(language_for("/a/b.cpp"), Some("cpp"));
        assert_eq!(language_for("/a/b.rb"), Some("ruby"));
        assert_eq!(language_for("/a/b.txt"), None);
        assert_eq!(language_for("/a/Makefile"), None);
    }

    #[test]
    fn every_mapped_language_has_a_grammar_and_kinds() {
        for (_, lang) in EXTENSION_LANGUAGE {
            assert!(grammar(lang).is_some(), "grammar missing for {lang}");
            assert!(definition_kinds(lang).is_some(), "kinds missing for {lang}");
        }
    }

    #[test]
    fn rust_definition_via_tree_sitter() {
        let src = "pub struct Volume;\n\npub fn open_volume(id: &str) -> Volume {\n    Volume\n}\n";
        let defs = find_definitions("/x.rs", src, "open_volume", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function_item");
        assert_eq!(defs[0].line, 3);
        assert_eq!(defs[0].name, "open_volume");
        assert_eq!(defs[0].path, "/x.rs");
    }

    #[test]
    fn python_definition_via_tree_sitter() {
        let src =
            "class Store:\n    def put(self, k):\n        return k\n\ndef helper():\n    pass\n";
        let defs = find_definitions("/x.py", src, "helper", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function_definition");
        assert_eq!(defs[0].line, 5);

        let cls = find_definitions("/x.py", src, "Store", None);
        assert_eq!(cls.len(), 1);
        assert_eq!(cls[0].kind, "class_definition");
        assert_eq!(cls[0].line, 1);
    }

    #[test]
    fn go_definition_via_tree_sitter() {
        let src = "package main\n\nfunc Serve(addr string) error {\n\treturn nil\n}\n";
        let defs = find_definitions("/x.go", src, "Serve", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function_declaration");
        assert_eq!(defs[0].line, 3);
    }

    #[test]
    fn typescript_interface_definition_via_tree_sitter() {
        let src = "export interface Node {\n  id: string;\n}\n\nexport function make(): Node {\n  return { id: '' };\n}\n";
        let defs = find_definitions("/x.ts", src, "Node", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "interface_declaration");
        let fns = find_definitions("/x.ts", src, "make", None);
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].kind, "function_declaration");
    }

    #[test]
    fn java_method_definition_via_tree_sitter() {
        let src = "class A {\n  public int add(int a, int b) {\n    return a + b;\n  }\n}\n";
        let defs = find_definitions("/A.java", src, "add", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "method_declaration");
        assert_eq!(defs[0].line, 2);
    }

    #[test]
    fn c_function_name_comes_from_the_declarator_subtree() {
        let src = "#include <stdio.h>\n\nint compute_sum(int a, int b) {\n  return a + b;\n}\n";
        let defs = find_definitions("/x.c", src, "compute_sum", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function_definition");
        assert_eq!(defs[0].line, 3);
    }

    #[test]
    fn cpp_class_definition_via_tree_sitter() {
        let src = "class Widget {\npublic:\n  void draw();\n};\n";
        let defs = find_definitions("/x.cpp", src, "Widget", None);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "class_specifier");
    }

    #[test]
    fn ruby_method_definition_via_tree_sitter() {
        let src = "module M\n  class C\n    def run\n    end\n  end\nend\n";
        assert_eq!(find_definitions("/x.rb", src, "run", None)[0].kind, "method");
        assert_eq!(find_definitions("/x.rb", src, "C", None)[0].kind, "class");
        assert_eq!(find_definitions("/x.rb", src, "M", None)[0].kind, "module");
    }

    #[test]
    fn empty_name_matches_every_definition() {
        let src = "fn a() {}\nfn b() {}\nstruct S;\n";
        let defs = find_definitions("/x.rs", src, "", None);
        assert_eq!(defs.len(), 3);
    }

    #[test]
    fn kind_filter_is_a_substring_match() {
        let src = "fn a() {}\nstruct S;\nenum E { X }\n";
        let structs = find_definitions("/x.rs", src, "", Some("struct"));
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].kind, "struct_item");
        // "item" is a substring of every rust definition kind
        assert_eq!(find_definitions("/x.rs", src, "", Some("item")).len(), 3);
        assert_eq!(find_definitions("/x.rs", src, "", Some("nope")).len(), 0);
    }

    #[test]
    fn references_count_every_identifier_occurrence() {
        let src = "fn helper() {}\nfn main() {\n    helper();\n    helper();\n}\n";
        let refs = find_references("/x.rs", src, "helper");
        assert_eq!(refs.len(), 3, "declaration plus two calls");
        assert_eq!(refs.iter().map(|r| r.line).collect::<Vec<_>>(), vec![1, 3, 4]);
        assert!(refs.iter().all(|r| r.kind.ends_with("identifier")));
        assert!(refs.iter().all(|r| r.name == "helper"));
    }

    #[test]
    fn references_ignore_substring_matches() {
        let src = "fn helper() {}\nfn helper_two() {}\n";
        assert_eq!(find_references("/x.rs", src, "helper").len(), 1);
    }

    #[test]
    fn unknown_extension_yields_nothing() {
        let src = "def whatever():\n    pass\n";
        assert!(find_definitions("/x.unknownext", src, "whatever", None).is_empty());
        assert!(find_references("/x.unknownext", src, "whatever").is_empty());
    }

    #[test]
    fn lexical_fallback_kinds_match_the_csharp_strings() {
        let py = "def run():\n    pass\nclass K:\n    pass\n";
        let defs = lexical_definitions("/x.py", py, "", None);
        assert_eq!(defs[0].kind, "function_definition");
        assert_eq!(defs[1].kind, "class_definition");

        let js = "export function go() {}\nconst v = 1;\nclass C {}\n";
        let jdefs = lexical_definitions("/x.js", js, "", None);
        assert!(jdefs.iter().any(|d| d.kind == "function_declaration" && d.name == "go"));
        assert!(jdefs.iter().any(|d| d.kind == "variable_declarator" && d.name == "v"));
        assert!(jdefs.iter().any(|d| d.kind == "class_declaration" && d.name == "C"));

        let go = "func Handle(w int) {\n}\ntype Conf struct{}\n";
        let gdefs = lexical_definitions("/x.go", go, "", None);
        assert!(gdefs.iter().any(|d| d.kind == "function_declaration" && d.name == "Handle"));
        assert!(gdefs.iter().any(|d| d.kind == "type_spec" && d.name == "Conf"));
    }

    #[test]
    fn lexical_fallback_returns_nothing_for_an_unmapped_language() {
        assert!(lexical_definitions("/x.zzz", "fn a() {}", "a", None).is_empty());
        assert!(lexical_references("/x.zzz", "a a a", "a").is_empty());
    }

    #[test]
    fn lexical_references_agree_with_tree_sitter_on_line_numbers() {
        let src = "fn helper() {}\nfn main() { helper(); }\n";
        let ts = find_references("/x.rs", src, "helper");
        let lex = lexical_references("/x.rs", src, "helper");
        assert_eq!(
            ts.iter().map(|r| r.line).collect::<Vec<_>>(),
            lex.iter().map(|r| r.line).collect::<Vec<_>>()
        );
    }

    #[test]
    fn grammar_available_reflects_the_static_link() {
        assert!(grammar_available("/x.rs"));
        assert!(grammar_available("/x.tsx"));
        assert!(!grammar_available("/x.zzz"));
    }

    #[test]
    fn broken_source_still_parses_and_finds_valid_definitions() {
        // tree-sitter is error tolerant: a truncated file must not lose the
        // definitions that precede the syntax error
        let src = "fn good() {}\nfn broken(\n";
        let defs = find_definitions("/x.rs", src, "good", None);
        assert_eq!(defs.len(), 1);
    }

    #[test]
    fn definitions_report_one_based_lines() {
        let src = "\n\n\nfn late() {}\n";
        assert_eq!(find_definitions("/x.rs", src, "late", None)[0].line, 4);
    }

    #[test]
    fn all_lexical_patterns_compile() {
        // build_lex_patterns panics on a bad regex, so simply forcing the lazy
        // init is the assertion
        assert_eq!(lex_patterns().len(), DEFINITION_KINDS.len());
    }

    // ── SPEC-0092 DEC-007 parity: the explicit stacks visit what the recursion visited ──

    /// The recursive walks and name search of before DEC-007, kept as the oracle.
    fn old_walk<'t, F: FnMut(Node<'t>)>(root: Node<'t>, visit: &mut F) {
        visit(root);
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            old_walk(child, visit);
        }
    }

    fn old_walk_descendants<'t, F: FnMut(Node<'t>)>(node: Node<'t>, visit: &mut F) {
        let mut cursor = node.walk();
        let children: Vec<Node<'t>> = node.named_children(&mut cursor).collect();
        for child in children {
            visit(child);
            old_walk_descendants(child, visit);
        }
    }

    fn old_name_of(node: Node<'_>, source: &[u8]) -> Option<String> {
        if let Some(named) = node.child_by_field_name("name")
            && let Ok(text) = named.utf8_text(source)
        {
            return Some(text.to_string());
        }
        let mut found = None;
        old_walk_descendants(node, &mut |d| {
            if found.is_some() {
                return;
            }
            if matches!(d.kind(), "identifier" | "field_identifier" | "type_identifier")
                && let Ok(text) = d.utf8_text(source)
            {
                found = Some(text.to_string());
            }
        });
        found
    }

    type Visit = (&'static str, usize, usize);

    fn visit_of(n: Node<'_>) -> Visit {
        (n.kind(), n.start_byte(), n.end_byte())
    }

    fn old_definitions(
        path: &str,
        source: &str,
        name: &str,
        kind: Option<&str>,
    ) -> Vec<Definition> {
        let language = language_for(path).expect("a mapped language");
        let kinds = definition_kinds(language).expect("kinds");
        let tree = parse(&grammar(language).expect("grammar"), source).expect("a tree");
        let mut out = Vec::new();
        old_walk(tree.root_node(), &mut |node| {
            let node_kind = node.kind();
            if !kinds.contains(&node_kind) {
                return;
            }
            let Some(found) = old_name_of(node, source.as_bytes()) else { return };
            let name_ok = name.is_empty() || found == name;
            let kind_ok = kind.is_none_or(|k| node_kind.contains(k));
            if name_ok && kind_ok {
                out.push(Definition {
                    path: path.to_string(),
                    name: found,
                    kind: node_kind.to_string(),
                    line: node.start_position().row + 1,
                });
            }
        });
        out
    }

    fn old_references(path: &str, source: &str, name: &str) -> Vec<Reference> {
        let language = language_for(path).expect("a mapped language");
        let tree = parse(&grammar(language).expect("grammar"), source).expect("a tree");
        let mut out = Vec::new();
        old_walk(tree.root_node(), &mut |node| {
            if node.kind().ends_with("identifier")
                && node.utf8_text(source.as_bytes()).ok() == Some(name)
            {
                out.push(Reference {
                    path: path.to_string(),
                    name: name.to_string(),
                    kind: node.kind().to_string(),
                    line: node.start_position().row + 1,
                });
            }
        });
        out
    }

    /// Normal sources of every mapped language, then 1 000 deep nestings of each.
    fn parity_corpus() -> Vec<(String, String)> {
        let deep = |head: &str, tail: &str| {
            crate::tools::nesting_testkit::deep_source(head, '(', "1", ')', 1_000, tail)
        };
        let mut corpus: Vec<(&str, String)> = vec![
            ("/a.py", "class Store:\n    def put(self, k):\n        return k\n\ndef helper():\n    total = 1\n    return helper(total)\n".into()),
            ("/a.js", "class A { m(x) { return x; } }\nfunction g(a) { const h = [a, [a]]; return h; }\n".into()),
            ("/a.jsx", "const C = () => <div id={a}>{a}</div>;\nfunction g(a) { return a; }\n".into()),
            ("/a.ts", "interface I { m(): void }\nclass A { m(x: number) { return x; } }\nfunction g(a: I) { return a; }\n".into()),
            ("/a.tsx", "interface P { a: number }\nfunction C(p: P) { return <b>{p.a}</b>; }\n".into()),
            ("/a.go", "package p\ntype T struct{ a int }\nfunc (t T) m() int { return t.a }\nfunc g(a int) int { return a }\n".into()),
            ("/a.rs", "pub struct Volume;\nenum E { X }\ntrait T { fn t(&self); }\npub fn open(id: &str) -> Volume {\n    Volume\n}\n".into()),
            ("/a.java", "interface I { void m(); }\nclass A implements I { public void m() { int a = 1; a++; } }\n".into()),
            ("/a.c", "struct S { int a; };\nint f(int a) { return a; }\nstatic char *g(void) { return 0; }\n".into()),
            ("/a.h", "int f(int a);\nstruct S { int a; };\n".into()),
            ("/a.cpp", "class K { public: int m(int a) { return a; } };\nstruct S { int a; };\nint f(int a) { return a; }\n".into()),
            ("/a.rb", "module M\n  class C\n    def m(a)\n      a\n    end\n  end\nend\n".into()),
            ("/broken.rs", "fn good() {}\nfn broken(\n".into()),
            ("/empty.py", String::new()),
        ];
        corpus.extend([
            ("/n.py", deep("def f():\n    return ", "\n")),
            (
                "/n.js",
                crate::tools::nesting_testkit::deep_source("const g = ", '[', "g", ']', 1_000, ";"),
            ),
            ("/n.ts", deep("function f(): number { return ", "; }\n")),
            ("/n.tsx", deep("function f(): number { return ", "; }\n")),
            ("/n.go", deep("package p\nfunc f() int { return ", " }\n")),
            ("/n.rs", deep("fn f() -> i32 { ", " }\n")),
            ("/n.java", deep("class A { int f() { return ", "; } }\n")),
            ("/n.c", deep("int f(void) { return ", "; }")),
            ("/n.cpp", deep("int f(void) { return ", "; }")),
            ("/n.rb", deep("def f\n  ", "\nend\n")),
        ]);
        corpus.into_iter().map(|(p, s)| (p.to_string(), s)).collect()
    }

    #[test]
    fn explicit_stack_walks_match_the_recursive_walks_on_every_language() {
        let corpus = parity_corpus();
        let mut visited = 0;
        for (path, source) in &corpus {
            let language = language_for(path).expect("a mapped language");
            let tree = parse(&grammar(language).expect("grammar"), source).expect("a tree");
            let (mut old, mut new) = (Vec::new(), Vec::new());
            old_walk(tree.root_node(), &mut |n| old.push(visit_of(n)));
            walk(tree.root_node(), &mut |n| new.push(visit_of(n)));
            assert_eq!(old, new, "walk order differs on {path}");
            visited += old.len();

            // Every node's descendants, in order: what `name_of` scans.
            let mut roots = Vec::new();
            old_walk(tree.root_node(), &mut |n| roots.push(n));
            for node in roots {
                let (mut o, mut n2) = (Vec::new(), Vec::new());
                old_walk_descendants(node, &mut |d| o.push(visit_of(d)));
                walk_descendants_all(node, &mut |d| n2.push(visit_of(d)));
                assert_eq!(o, n2, "descendant order differs under {} on {path}", node.kind());
            }

            for name in ["", "f", "g", "total", "helper", "a", "Store", "m", "T", "S"] {
                assert_eq!(
                    find_definitions(path, source, name, None),
                    old_definitions(path, source, name, None),
                    "definitions of {name:?} differ on {path}"
                );
                assert_eq!(
                    find_definitions(path, source, name, Some("struct")),
                    old_definitions(path, source, name, Some("struct")),
                    "struct definitions of {name:?} differ on {path}"
                );
                if !name.is_empty() {
                    assert_eq!(
                        find_references(path, source, name),
                        old_references(path, source, name),
                        "references to {name:?} differ on {path}"
                    );
                }
            }
        }
        assert!(visited > 10_000, "the corpus is too small to prove anything: {visited}");
    }

    /// The new descendants visit run to the end, like the old one.
    fn walk_descendants_all<'t>(node: Node<'t>, visit: &mut impl FnMut(Node<'t>)) {
        walk_descendants(node, &mut |d| {
            visit(d);
            true
        });
    }
}
