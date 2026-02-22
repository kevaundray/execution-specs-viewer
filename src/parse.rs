use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: &'static str,
    pub text: String,
    pub line: usize, // 0-based
    pub col: usize,
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.text == other.text
    }
}

impl Eq for Token {}

impl PartialOrd for Token {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Token {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind
            .cmp(other.kind)
            .then_with(|| self.text.cmp(&other.text))
    }
}

impl std::hash::Hash for Token {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.text.hash(state);
    }
}

pub struct ParsedFile {
    pub source: String,
    pub tokens: Vec<Token>,
    pub lines: Vec<String>,
    pub blocks: Vec<SourceBlock>,
    pub definitions: Vec<Definition>,
    /// Module-level docstring (first expression_statement containing a string).
    pub module_docstring: Option<ModuleDocstring>,
}

#[derive(Debug, Clone)]
pub struct ModuleDocstring {
    pub lines: LineRange,
    pub text: String, // raw text with quotes stripped
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefKind {
    Function,
    Class,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineRange {
    pub start: usize, // 0-based inclusive
    pub end: usize,   // 0-based inclusive
}

#[derive(Debug, Clone)]
pub struct Definition {
    pub kind: DefKind,
    pub name: String,
    pub decorator_lines: Option<LineRange>,
    pub signature_lines: LineRange,
    pub docstring_lines: Option<LineRange>,
    pub docstring_text: Option<String>,
    pub body_lines: Option<LineRange>,
}

#[derive(Debug, Clone)]
pub struct SourceBlock {
    pub name: Option<String>,
    pub start_line: usize, // 0-based inclusive
    pub end_line: usize,   // 0-based inclusive
}

fn is_skippable(kind: &str) -> bool {
    matches!(
        kind,
        "comment"
            | "newline"
            | "NEWLINE"
            | "indent"
            | "dedent"
            | "INDENT"
            | "DEDENT"
            | "\n"
    )
}

/// Intern a tree-sitter node kind string to a `&'static str`.
///
/// tree-sitter node kind strings are already static in the grammar,
/// but the Rust API returns `&str` tied to the tree lifetime. We
/// intern them into a global set so tokens can outlive the tree.
fn intern_kind(kind: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;

    static INTERNED: std::sync::LazyLock<Mutex<HashSet<&'static str>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

    let mut set = INTERNED.lock().unwrap();
    if let Some(&existing) = set.get(kind) {
        existing
    } else {
        let leaked: &'static str = Box::leak(kind.to_string().into_boxed_str());
        set.insert(leaked);
        leaked
    }
}

fn extract_tokens(root: tree_sitter::Node, source: &[u8]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut cursor = root.walk();
    let mut reached_root = false;

    loop {
        let node = cursor.node();

        if node.child_count() == 0 {
            let kind = node.kind();
            if !is_skippable(kind) {
                let text = &source[node.byte_range()];
                let text = String::from_utf8_lossy(text).into_owned();
                let start = node.start_position();
                tokens.push(Token {
                    kind: intern_kind(kind),
                    text,
                    line: start.row,
                    col: start.column,
                });
            }
        }

        // Depth-first: go to first child if possible.
        if cursor.goto_first_child() {
            continue;
        }

        // Otherwise, try next sibling.
        if cursor.goto_next_sibling() {
            continue;
        }

        // Go up until we find a sibling or reach root.
        loop {
            if !cursor.goto_parent() {
                reached_root = true;
                break;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }

        if reached_root {
            break;
        }
    }

    tokens
}

/// Extract the module-level docstring: the first top-level expression_statement
/// that contains a string node.
fn extract_module_docstring(root: tree_sitter::Node, source: &[u8]) -> Option<ModuleDocstring> {
    let mut cursor = root.walk();
    if !cursor.goto_first_child() {
        return None;
    }

    // The module docstring must be the first statement. In Python the first
    // top-level node is an expression_statement wrapping a string.
    let node = cursor.node();
    if node.kind() != "expression_statement" {
        return None;
    }

    let string_node = node.named_child(0)?;
    if string_node.kind() != "string" && string_node.kind() != "concatenated_string" {
        return None;
    }

    let raw = String::from_utf8_lossy(&source[string_node.byte_range()]).into_owned();
    let text = strip_docstring_quotes(&raw);
    let lines = LineRange {
        start: node.start_position().row,
        end: node.end_position().row,
    };
    Some(ModuleDocstring { lines, text })
}

fn extract_definitions(root: tree_sitter::Node, source: &[u8]) -> Vec<Definition> {
    let mut defs = Vec::new();
    let mut cursor = root.walk();

    if !cursor.goto_first_child() {
        return defs;
    }

    loop {
        let node = cursor.node();
        extract_definition_from_node(&node, source, &mut defs);
        if !cursor.goto_next_sibling() {
            break;
        }
    }

    defs
}

fn extract_definition_from_node(
    node: &tree_sitter::Node,
    source: &[u8],
    defs: &mut Vec<Definition>,
) {
    match node.kind() {
        "function_definition" | "class_definition" => {
            if let Some(def) = build_definition(node, None, source) {
                defs.push(def);
            }
        }
        "decorated_definition" => {
            // Find decorator lines and the inner definition
            let mut decorator_start = None;
            let mut decorator_end = None;
            let mut inner_def_node = None;

            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if child.kind() == "decorator" {
                        let start = child.start_position().row;
                        let end = child.end_position().row;
                        if decorator_start.is_none() {
                            decorator_start = Some(start);
                        }
                        decorator_end = Some(end);
                    } else if child.kind() == "function_definition"
                        || child.kind() == "class_definition"
                    {
                        inner_def_node = Some(child);
                    }
                }
            }

            if let Some(inner) = inner_def_node {
                let dec_lines = match (decorator_start, decorator_end) {
                    (Some(s), Some(e)) => Some(LineRange { start: s, end: e }),
                    _ => None,
                };
                if let Some(def) = build_definition(&inner, dec_lines, source) {
                    defs.push(def);
                }
            }
        }
        _ => {}
    }
}

fn build_definition(
    node: &tree_sitter::Node,
    decorator_lines: Option<LineRange>,
    source: &[u8],
) -> Option<Definition> {
    let kind = match node.kind() {
        "function_definition" => DefKind::Function,
        "class_definition" => DefKind::Class,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .map(|n| String::from_utf8_lossy(&source[n.byte_range()]).into_owned())?;

    // Signature: from the definition node's start line to the `:` line.
    let sig_start = node.start_position().row;
    let mut sig_end = sig_start;

    // Find the `:` that ends the signature (the `body` field's preceding `:`)
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == ":" {
                sig_end = child.start_position().row;
                break;
            }
        }
    }

    let signature_lines = LineRange {
        start: sig_start,
        end: sig_end,
    };

    // Find the block/body child
    let block_node = node.child_by_field_name("body");

    let (docstring_lines, docstring_text, body_lines) = if let Some(block) = block_node {
        extract_docstring_and_body(&block, source)
    } else {
        (None, None, None)
    };

    Some(Definition {
        kind,
        name,
        decorator_lines,
        signature_lines,
        docstring_lines,
        docstring_text,
        body_lines,
    })
}

/// Extract docstring and body from a block node.
///
/// The docstring is the first child of the block if it's an expression_statement
/// containing a string node. The body is everything after the docstring.
fn extract_docstring_and_body(
    block: &tree_sitter::Node,
    source: &[u8],
) -> (Option<LineRange>, Option<String>, Option<LineRange>) {
    let block_start = block.start_position().row;
    let block_end = block.end_position().row;

    // Look for docstring: first named child that is expression_statement containing a string
    let first_named = block.named_child(0);
    let (doc_lines, doc_text, body_start_line) = match first_named {
        Some(expr_stmt) if expr_stmt.kind() == "expression_statement" => {
            // Check if it contains a string (docstring)
            let string_node = expr_stmt.named_child(0);
            match string_node {
                Some(s)
                    if s.kind() == "string"
                        || s.kind() == "concatenated_string" =>
                {
                    let doc_start = expr_stmt.start_position().row;
                    let doc_end = expr_stmt.end_position().row;
                    let raw = String::from_utf8_lossy(&source[s.byte_range()]).into_owned();
                    let text = strip_docstring_quotes(&raw);
                    let body_start = doc_end + 1;
                    (
                        Some(LineRange {
                            start: doc_start,
                            end: doc_end,
                        }),
                        Some(text),
                        body_start,
                    )
                }
                _ => (None, None, block_start),
            }
        }
        _ => (None, None, block_start),
    };

    let body = if body_start_line <= block_end {
        // Check if there's actually any content after docstring
        let has_body_content = if doc_lines.is_some() {
            // There's content after the docstring
            block.named_child_count() > 1
        } else {
            block.named_child_count() > 0
        };
        if has_body_content {
            Some(LineRange {
                start: body_start_line,
                end: block_end,
            })
        } else {
            None
        }
    } else {
        None
    };

    (doc_lines, doc_text, body)
}

/// Strip triple-quote delimiters from a docstring.
fn strip_docstring_quotes(raw: &str) -> String {
    let s = raw.trim();
    for delim in &["\"\"\"", "'''", "r\"\"\"", "r'''"] {
        if s.starts_with(delim) {
            let base_delim = if delim.starts_with('r') {
                &delim[1..]
            } else {
                delim
            };
            if s.ends_with(base_delim) {
                return s[delim.len()..s.len() - base_delim.len()].to_string();
            }
        }
    }
    // Single-quoted strings
    for delim in &["\"", "'"] {
        if s.starts_with(delim) && s.ends_with(delim) && s.len() >= 2 {
            return s[delim.len()..s.len() - delim.len()].to_string();
        }
    }
    raw.to_string()
}

pub fn parse_source(source: &str) -> Result<ParsedFile> {
    let mut parser = tree_sitter::Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser
        .set_language(&language.into())
        .context("failed to set tree-sitter-python language")?;

    let tree = parser
        .parse(source.as_bytes(), None)
        .context("tree-sitter parse returned None")?;

    let tokens = extract_tokens(tree.root_node(), source.as_bytes());
    let lines = source.lines().map(String::from).collect();
    let blocks = extract_blocks(tree.root_node(), source.as_bytes());
    let definitions = extract_definitions(tree.root_node(), source.as_bytes());
    let module_docstring = extract_module_docstring(tree.root_node(), source.as_bytes());

    Ok(ParsedFile {
        source: source.to_string(),
        tokens,
        lines,
        blocks,
        definitions,
        module_docstring,
    })
}

fn extract_blocks(root: tree_sitter::Node, source: &[u8]) -> Vec<SourceBlock> {
    let mut blocks = Vec::new();
    let mut cursor = root.walk();

    if !cursor.goto_first_child() {
        return blocks;
    }

    loop {
        let node = cursor.node();
        if node.is_named() {
            let name = definition_name(&node, source);
            blocks.push(SourceBlock {
                name,
                start_line: node.start_position().row,
                end_line: node.end_position().row,
            });
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }

    // Merge consecutive unnamed blocks (imports, assignments, etc.)
    let mut merged: Vec<SourceBlock> = Vec::new();
    for block in blocks {
        if block.name.is_none() {
            if let Some(last) = merged.last_mut() {
                if last.name.is_none() {
                    last.end_line = block.end_line;
                    continue;
                }
            }
        }
        merged.push(block);
    }

    merged
}

fn definition_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "function_definition" | "class_definition" => node
            .child_by_field_name("name")
            .map(|n| String::from_utf8_lossy(&source[n.byte_range()]).into_owned()),
        "decorated_definition" => {
            for i in 0..node.named_child_count() {
                if let Some(child) = node.named_child(i) {
                    if child.kind() == "function_definition"
                        || child.kind() == "class_definition"
                    {
                        return child
                            .child_by_field_name("name")
                            .map(|n| {
                                String::from_utf8_lossy(&source[n.byte_range()]).into_owned()
                            });
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub fn parse_file(path: &Path) -> Result<ParsedFile> {
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_source(&source).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let source = "x = 1\n";
        let parsed = parse_source(source).unwrap();
        let kinds: Vec<&str> = parsed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec!["identifier", "=", "integer"]);
        assert_eq!(parsed.tokens[0].text, "x");
        assert_eq!(parsed.tokens[2].text, "1");
    }

    #[test]
    fn test_function_tokens() {
        let source = "def foo(a, b):\n    return a + b\n";
        let parsed = parse_source(source).unwrap();
        let kinds: Vec<&str> = parsed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "def", "identifier", "(", "identifier", ",", "identifier", ")",
                ":", "return", "identifier", "+", "identifier",
            ]
        );
    }

    #[test]
    fn test_comments_excluded() {
        let source = "# this is a comment\nx = 1\n";
        let parsed = parse_source(source).unwrap();
        assert!(parsed.tokens.iter().all(|t| t.kind != "comment"));
        assert_eq!(parsed.tokens.len(), 3); // x, =, 1
    }

    #[test]
    fn test_whitespace_only_difference() {
        let source1 = "x = 1\ny = 2\n";
        let source2 = "x  =  1\ny  =  2\n";
        let parsed1 = parse_source(source1).unwrap();
        let parsed2 = parse_source(source2).unwrap();
        assert_eq!(parsed1.tokens, parsed2.tokens);
    }

    #[test]
    fn test_lines() {
        let source = "line one\nline two\nline three\n";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.lines, vec!["line one", "line two", "line three"]);
    }

    #[test]
    fn test_token_positions() {
        let source = "x = 1\ny = 2\n";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.tokens[0].line, 0);
        assert_eq!(parsed.tokens[0].col, 0);
        // y is on line 1
        assert_eq!(parsed.tokens[3].line, 1);
        assert_eq!(parsed.tokens[3].text, "y");
    }

    #[test]
    fn test_class_def() {
        let source = "class Foo:\n    pass\n";
        let parsed = parse_source(source).unwrap();
        let kinds: Vec<&str> = parsed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec!["class", "identifier", ":", "pass"]);
    }

    #[test]
    fn test_block_extraction() {
        let source = "import os\n\ndef foo():\n    pass\n\ndef bar():\n    pass\n";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.blocks.len(), 3);
        assert_eq!(parsed.blocks[0].name, None); // import
        assert_eq!(parsed.blocks[1].name, Some("foo".to_string()));
        assert_eq!(parsed.blocks[2].name, Some("bar".to_string()));
    }

    #[test]
    fn test_block_extraction_class() {
        let source = "class Foo:\n    pass\n\ndef bar():\n    pass\n";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.blocks.len(), 2);
        assert_eq!(parsed.blocks[0].name, Some("Foo".to_string()));
        assert_eq!(parsed.blocks[1].name, Some("bar".to_string()));
    }

    #[test]
    fn test_string_literal() {
        let source = "s = \"hello\"\n";
        let parsed = parse_source(source).unwrap();
        // tree-sitter splits strings into start, content, end tokens
        let kinds: Vec<&str> = parsed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec!["identifier", "=", "string_start", "string_content", "string_end"]
        );
        assert_eq!(parsed.tokens[3].text, "hello");
    }

    #[test]
    fn test_extract_definition_with_docstring() {
        let source = "\
def foo(a: int, b: str) -> bool:
    \"\"\"Check if a equals b.

    Parameters
    ----------
    a : int
        The first value.
    b : str
        The second value.

    Returns
    -------
    bool
        True if equal.
    \"\"\"
    return str(a) == b
";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.definitions.len(), 1);

        let def = &parsed.definitions[0];
        assert_eq!(def.name, "foo");
        assert_eq!(def.kind, DefKind::Function);
        assert_eq!(def.signature_lines, LineRange { start: 0, end: 0 });
        assert!(def.docstring_lines.is_some());
        assert!(def.docstring_text.is_some());
        let doc = def.docstring_text.as_ref().unwrap();
        assert!(doc.contains("Check if a equals b"));
        assert!(doc.contains("Parameters"));
        assert!(def.body_lines.is_some());
        assert!(def.decorator_lines.is_none());
    }

    #[test]
    fn test_extract_decorated_definition() {
        let source = "\
@dataclass
class Foo:
    \"\"\"A foo thing.\"\"\"
    x: int
    y: str
";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.definitions.len(), 1);

        let def = &parsed.definitions[0];
        assert_eq!(def.name, "Foo");
        assert_eq!(def.kind, DefKind::Class);
        assert!(def.decorator_lines.is_some());
        let dec = def.decorator_lines.as_ref().unwrap();
        assert_eq!(dec.start, 0);
        assert_eq!(dec.end, 0);
        assert!(def.docstring_text.is_some());
        assert!(def.docstring_text.as_ref().unwrap().contains("A foo thing"));
    }

    #[test]
    fn test_extract_no_definitions_from_imports() {
        let source = "import os\nimport sys\n\nX = 42\n";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.definitions.len(), 0);
    }

    #[test]
    fn test_extract_multiple_definitions() {
        let source = "\
def foo():
    pass

def bar():
    \"\"\"Bar docstring.\"\"\"
    return 1
";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.definitions.len(), 2);
        assert_eq!(parsed.definitions[0].name, "foo");
        assert_eq!(parsed.definitions[1].name, "bar");
        assert!(parsed.definitions[0].docstring_text.is_none());
        assert!(parsed.definitions[1].docstring_text.is_some());
    }

    #[test]
    fn test_multiline_signature() {
        let source = "\
def foo(
    a: int,
    b: str,
) -> bool:
    return True
";
        let parsed = parse_source(source).unwrap();
        assert_eq!(parsed.definitions.len(), 1);
        let def = &parsed.definitions[0];
        assert_eq!(def.signature_lines.start, 0);
        // The `:` is on line 3 (0-based)
        assert_eq!(def.signature_lines.end, 3);
    }
}
