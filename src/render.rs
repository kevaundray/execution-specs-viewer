use crate::diff::{
    DefStatus, DefinitionDiff, DiffLine, FileDefDiff, FileDiff, LineStatus, ModuleDocDiff,
    RenderedToken, TokenHighlight,
};
use crate::parse::{DefKind, LineRange, ParsedFile};

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn token_css_class(kind: &str) -> &'static str {
    match kind {
        "def" | "class" | "return" | "if" | "else" | "elif" | "for" | "while" | "with"
        | "as" | "import" | "from" | "try" | "except" | "finally" | "raise" | "pass"
        | "break" | "continue" | "yield" | "assert" | "global" | "nonlocal" | "del"
        | "lambda" | "and" | "or" | "not" | "in" | "is" | "True" | "False" | "None"
        | "async" | "await" => "kw",
        "identifier" => "ident",
        "string" | "string_start" | "string_content" | "string_end" | "concatenated_string" => {
            "str"
        }
        "integer" | "float" => "num",
        "+" | "-" | "*" | "/" | "%" | "**" | "//" | "==" | "!=" | "<" | ">" | "<=" | ">="
        | "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "//=" | "&" | "|" | "^" | "~"
        | "<<" | ">>" | "&=" | "|=" | "^=" | "<<=" | ">>=" | "->" | ":=" => "op",
        "@" => "decorator",
        _ => "",
    }
}

fn highlight_class(highlight: &TokenHighlight) -> &'static str {
    match highlight {
        TokenHighlight::Unchanged => "",
        TokenHighlight::Added => "tok-add",
        TokenHighlight::Deleted => "tok-del",
    }
}

fn render_token(token: &RenderedToken) -> String {
    let text = escape_html(&token.text);
    let css = token_css_class(token.kind);
    let hl = highlight_class(&token.highlight);

    let mut classes = String::new();
    if !css.is_empty() {
        classes.push_str(css);
    }
    if !hl.is_empty() {
        if !classes.is_empty() {
            classes.push(' ');
        }
        classes.push_str(hl);
    }

    if classes.is_empty() {
        text
    } else {
        format!("<span class=\"{classes}\">{text}</span>")
    }
}

fn render_tokens(tokens: &[RenderedToken]) -> String {
    tokens.iter().map(render_token).collect::<Vec<_>>().join(" ")
}

fn line_status_class(status: &LineStatus) -> &'static str {
    match status {
        LineStatus::Unchanged => "",
        LineStatus::Added => "add",
        LineStatus::Removed => "del",
        LineStatus::Modified => "mod",
    }
}

/// Render the source line with token highlights overlaid.
///
/// We use the original source text as the base and wrap changed tokens in
/// highlight spans. Tokens that don't appear in the change list are rendered
/// as plain source text.
fn render_line_with_highlights(
    source_line: &str,
    tokens: &[RenderedToken],
    status: &LineStatus,
) -> String {
    if tokens.is_empty() {
        return escape_html(source_line);
    }

    // If the line is unchanged, just escape and return.
    if *status == LineStatus::Unchanged {
        return escape_html(source_line);
    }

    // For changed lines, render with token-level highlighting.
    render_tokens(tokens)
}

fn render_line_number(num: Option<usize>) -> String {
    match num {
        Some(n) => n.to_string(),
        None => String::new(),
    }
}

/// Render a single DiffLine as two `<tr>` cells.
fn render_diff_row(line: &DiffLine, before_lines: &[String], after_lines: &[String]) -> String {
    let status_class = line_status_class(&line.status);
    let left_num = render_line_number(line.left_number);
    let right_num = render_line_number(line.right_number);

    let left_content = match line.left_number {
        Some(n) => {
            let src = before_lines.get(n - 1).map(|s| s.as_str()).unwrap_or("");
            render_line_with_highlights(src, &line.left_tokens, &line.status)
        }
        None => String::new(),
    };

    let right_content = match line.right_number {
        Some(n) => {
            let src = after_lines.get(n - 1).map(|s| s.as_str()).unwrap_or("");
            render_line_with_highlights(src, &line.right_tokens, &line.status)
        }
        None => String::new(),
    };

    let left_class = match line.status {
        LineStatus::Removed | LineStatus::Modified => format!(" class=\"{status_class}\""),
        _ => String::new(),
    };
    let right_class = match line.status {
        LineStatus::Added | LineStatus::Modified => format!(" class=\"{status_class}\""),
        _ => String::new(),
    };

    format!(
        "<tr>\
         <td class=\"ln\">{left_num}</td>\
         <td{left_class}><pre>{left_content}</pre></td>\
         <td class=\"ln\">{right_num}</td>\
         <td{right_class}><pre>{right_content}</pre></td>\
         </tr>\n"
    )
}

/// Render a full file diff as an HTML table.
pub fn render_diff_table(diff: &FileDiff, before: &ParsedFile, after: &ParsedFile) -> String {
    let mut html = String::from("<table class=\"diff\">\n");
    for line in &diff.lines {
        html.push_str(&render_diff_row(line, &before.lines, &after.lines));
    }
    html.push_str("</table>\n");
    html
}

/// Render a diff for an added file (no before side).
pub fn render_added_table(diff: &FileDiff, after: &ParsedFile) -> String {
    let empty: Vec<String> = Vec::new();
    let mut html = String::from("<table class=\"diff\">\n");
    for line in &diff.lines {
        html.push_str(&render_diff_row(line, &empty, &after.lines));
    }
    html.push_str("</table>\n");
    html
}

/// Render a diff for a removed file (no after side).
pub fn render_removed_table(diff: &FileDiff, before: &ParsedFile) -> String {
    let empty: Vec<String> = Vec::new();
    let mut html = String::from("<table class=\"diff\">\n");
    for line in &diff.lines {
        html.push_str(&render_diff_row(line, &before.lines, &empty));
    }
    html.push_str("</table>\n");
    html
}

// ---------------------------------------------------------------------------
// Card-based rendering for definition-level diffs
// ---------------------------------------------------------------------------

/// Render a full definition-level diff as HTML cards.
pub fn render_definition_cards(
    diff: &FileDefDiff,
    before: &ParsedFile,
    after: &ParsedFile,
    before_file_url: Option<&str>,
    after_file_url: Option<&str>,
) -> String {
    let mut html = String::new();

    // Expand/collapse all controls
    html.push_str(
        "<div class=\"card-controls\">\
         <button onclick=\"expandAllCards()\">Expand all</button> \
         <button onclick=\"collapseAllCards()\">Collapse all</button>\
         </div>\n",
    );

    // Module docstring card
    if let Some(ref mod_doc) = diff.module_doc_diff {
        html.push_str(&render_module_docstring_card(mod_doc, before, after));
    }

    // Preamble section (imports/constants)
    if let Some(ref preamble) = diff.preamble_diff {
        if !preamble.lines.is_empty() {
            let status_class = if preamble.changed { "modified" } else { "unchanged" };
            let collapsed = if preamble.changed { "" } else { " collapsed" };
            html.push_str(&format!(
                "<div class=\"def-card {status_class}\">\
                 <div class=\"def-header\" onclick=\"toggleCard(this)\">\
                 <span class=\"def-badge {status_class}\">{label}</span>\
                 <span class=\"def-name\">imports / constants</span>\
                 </div>\
                 <div class=\"def-body{collapsed}\">\
                 <div class=\"def-section\">\
                 {table}\
                 </div>\
                 </div>\
                 </div>\n",
                status_class = status_class,
                label = if preamble.changed { "modified" } else { "unchanged" },
                collapsed = collapsed,
                table = render_section_table(&preamble.lines, before, after),
            ));
        }
    }

    // Definition cards
    for def_diff in &diff.definitions {
        html.push_str(&render_definition_card(
            def_diff,
            before,
            after,
            before_file_url,
            after_file_url,
        ));
    }

    html
}

/// Render the module-level docstring as a card with formatted prose.
fn render_module_docstring_card(
    mod_doc: &ModuleDocDiff,
    before: &ParsedFile,
    after: &ParsedFile,
) -> String {
    let changed = mod_doc.section_diff.changed;
    let status_class = if changed { "modified" } else { "unchanged" };
    let collapsed = if changed { "" } else { " collapsed" };

    // Render the docstring text as formatted HTML (prefer after, fall back to before)
    let doc_text = mod_doc
        .after_text
        .as_deref()
        .or(mod_doc.before_text.as_deref());

    let mut body = String::new();

    if let Some(text) = doc_text {
        body.push_str("<div class=\"def-docstring module-docstring\">");
        body.push_str(&render_module_docstring_html(text));
        body.push_str("</div>\n");
    }

    if changed {
        body.push_str("<div class=\"def-section\">");
        body.push_str("<h4>Docstring diff</h4>");
        body.push_str(&render_section_table(
            &mod_doc.section_diff.lines,
            before,
            after,
        ));
        body.push_str("</div>\n");
    }

    format!(
        "<div class=\"def-card {status_class}\">\
         <div class=\"def-header\" onclick=\"toggleCard(this)\">\
         <span class=\"def-badge {status_class}\">{label}</span>\
         <span class=\"def-name\">Module Documentation</span>\
         </div>\
         <div class=\"def-body{collapsed}\">\
         {body}\
         </div>\
         </div>\n",
        status_class = status_class,
        label = if changed { "modified" } else { "unchanged" },
        collapsed = collapsed,
        body = body,
    )
}

/// Render module-level docstring content as HTML.
///
/// These use a mix of markdown-ish syntax:
/// - `### Heading` → `<h4>`
/// - `- item` → list items
/// - `[text](url)` and `[text]: url` → links
/// - ReStructuredText `.. contents::` directives → stripped
/// - Backtick-wrapped text → `<code>`
fn render_module_docstring_html(text: &str) -> String {
    let mut html = String::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut in_list = false;
    let mut in_table = false;
    let mut first_para = true;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Skip RST directives
        if trimmed.starts_with(".. ") {
            i += 1;
            // Skip continuation lines (indented)
            while i < lines.len() && (lines[i].starts_with("    ") || lines[i].trim().is_empty()) {
                if lines[i].trim().is_empty() {
                    // Empty line might end the directive
                    if i + 1 < lines.len() && !lines[i + 1].starts_with("    ") {
                        i += 1;
                        break;
                    }
                }
                i += 1;
            }
            continue;
        }

        // Markdown headings
        if trimmed.starts_with("### ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            if in_table {
                html.push_str("</table>\n");
                in_table = false;
            }
            html.push_str(&format!(
                "<h4 class=\"doc-heading\">{}</h4>\n",
                render_markdown_inline(&escape_html(&trimmed[4..]))
            ));
            i += 1;
            continue;
        }

        // Table rows (pipe-delimited)
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            // Check if this is a separator row
            let is_separator = trimmed
                .trim_matches('|')
                .split('|')
                .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':'));
            if !in_table {
                html.push_str("<table class=\"doc-table\">\n");
                in_table = true;
            }
            if !is_separator {
                html.push_str("<tr>");
                for cell in trimmed.trim_matches('|').split('|') {
                    html.push_str(&format!(
                        "<td>{}</td>",
                        render_markdown_inline(&escape_html(cell.trim()))
                    ));
                }
                html.push_str("</tr>\n");
            }
            i += 1;
            continue;
        } else if in_table {
            html.push_str("</table>\n");
            in_table = false;
        }

        // List items
        if trimmed.starts_with("- ") {
            if !in_list {
                html.push_str("<ul class=\"doc-list\">\n");
                in_list = true;
            }
            html.push_str(&format!(
                "<li>{}</li>\n",
                render_markdown_inline(&escape_html(&trimmed[2..]))
            ));
            i += 1;
            continue;
        } else if in_list && !trimmed.is_empty() && !trimmed.starts_with("- ") {
            html.push_str("</ul>\n");
            in_list = false;
        }

        // Reference-style links [name]: url — render as compact link
        if trimmed.starts_with('[') && trimmed.contains("]: ") {
            if let Some(bracket_end) = trimmed.find("]: ") {
                let name = &trimmed[1..bracket_end];
                let url = &trimmed[bracket_end + 3..];
                html.push_str(&format!(
                    "<p class=\"doc-ref\"><a href=\"{}\">{}</a></p>\n",
                    escape_html(url),
                    escape_html(name),
                ));
                i += 1;
                continue;
            }
        }

        // Empty lines
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            i += 1;
            continue;
        }

        // Regular paragraph text
        let tag = if first_para { "p class=\"doc-summary\"" } else { "p" };
        html.push_str(&format!(
            "<{tag}>{}</p>\n",
            render_markdown_inline(&escape_html(trimmed)),
            tag = tag,
        ));
        first_para = false;
        i += 1;
    }

    if in_list {
        html.push_str("</ul>\n");
    }
    if in_table {
        html.push_str("</table>\n");
    }

    html
}

/// Render inline markdown: `code`, [links](url), **bold**.
fn render_markdown_inline(text: &str) -> String {
    let text = render_inline_code(text);
    // [text](url) links
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut link_text = String::new();
            let mut found_bracket = false;
            for c2 in chars.by_ref() {
                if c2 == ']' {
                    found_bracket = true;
                    break;
                }
                link_text.push(c2);
            }
            if found_bracket && chars.peek() == Some(&'(') {
                chars.next(); // skip '('
                let mut url = String::new();
                let mut found_paren = false;
                for c3 in chars.by_ref() {
                    if c3 == ')' {
                        found_paren = true;
                        break;
                    }
                    url.push(c3);
                }
                if found_paren {
                    result.push_str(&format!("<a href=\"{}\">{}</a>", url, link_text));
                } else {
                    result.push('[');
                    result.push_str(&link_text);
                    result.push_str("](");
                    result.push_str(&url);
                }
            } else {
                result.push('[');
                result.push_str(&link_text);
                if found_bracket {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Render a single definition card.
fn render_definition_card(
    def: &DefinitionDiff,
    before: &ParsedFile,
    after: &ParsedFile,
    before_file_url: Option<&str>,
    after_file_url: Option<&str>,
) -> String {
    let status_class = match def.status {
        DefStatus::Unchanged => "unchanged",
        DefStatus::Added => "added",
        DefStatus::Removed => "removed",
        DefStatus::Modified => "modified",
    };
    let status_label = match def.status {
        DefStatus::Unchanged => "unchanged",
        DefStatus::Added => "added",
        DefStatus::Removed => "removed",
        DefStatus::Modified => "modified",
    };

    let kind_label = match def.kind {
        DefKind::Function => "def",
        DefKind::Class => "class",
    };

    let collapsed = match def.status {
        DefStatus::Unchanged => " collapsed",
        _ => "",
    };

    // Build the signature display for the header
    let sig_preview = build_signature_preview(def, before, after);

    let mut body = String::new();

    // Docstring section
    if let Some(ref doc_diff) = def.docstring_diff {
        let docstring_text = get_docstring_text(def, before, after);
        if let Some(text) = docstring_text {
            body.push_str("<div class=\"def-docstring\">");
            body.push_str(&render_docstring_html(&text));
            body.push_str("</div>\n");
        }
        if doc_diff.changed {
            body.push_str("<div class=\"def-section\">");
            body.push_str("<h4>Docstring diff</h4>");
            body.push_str(&render_section_table(&doc_diff.lines, before, after));
            body.push_str("</div>\n");
        }
    }

    // Decorator diff (if changed)
    if let Some(ref dec_diff) = def.decorator_diff {
        if dec_diff.changed {
            body.push_str("<div class=\"def-section\">");
            body.push_str("<h4>Decorators</h4>");
            body.push_str(&render_section_table(&dec_diff.lines, before, after));
            body.push_str("</div>\n");
        }
    }

    // Signature diff (if changed)
    if def.signature_diff.changed {
        body.push_str("<div class=\"def-section sig-diff\">");
        body.push_str("<h4>Signature</h4>");
        body.push_str(&render_section_table(&def.signature_diff.lines, before, after));
        body.push_str("</div>\n");
    }

    // Body diff
    if let Some(ref body_diff) = def.body_diff {
        let section_label = if body_diff.changed {
            "Implementation (changed)"
        } else {
            "Implementation"
        };
        body.push_str("<div class=\"def-section code-diff\">");
        body.push_str(&format!("<h4>{}</h4>", section_label));
        body.push_str(&render_section_table(&body_diff.lines, before, after));
        body.push_str("</div>\n");
    }

    // Build the source link for the right edge of the header.
    // For removed defs, link to before; for others, link to after.
    let source_link = {
        let (file_url, line_number) = match def.status {
            DefStatus::Removed => {
                let line = def.signature_diff.lines.first().and_then(|l| l.left_number);
                (before_file_url, line)
            }
            _ => {
                let line = def.signature_diff.lines.first().and_then(|l| l.right_number);
                (after_file_url, line)
            }
        };
        match (file_url, line_number) {
            (Some(url), Some(line)) => format!(
                "<a class=\"source-link\" href=\"{url}#L{line}\" target=\"_blank\" \
                 title=\"View source on GitHub\" onclick=\"event.stopPropagation()\">github</a>",
                url = escape_html(url),
                line = line,
            ),
            _ => String::new(),
        }
    };

    format!(
        "<div class=\"def-card {status_class}\">\
         <div class=\"def-header\" onclick=\"toggleCard(this)\">\
         <span class=\"def-badge {status_class}\">{status_label}</span>\
         <span class=\"def-kind\">{kind_label}</span>\
         <span class=\"def-name\">{name}</span>\
         <span class=\"def-sig\">{sig_preview}</span>\
         {source_link}\
         </div>\
         <div class=\"def-body{collapsed}\">\
         {body}\
         </div>\
         </div>\n",
        status_class = status_class,
        status_label = status_label,
        kind_label = kind_label,
        name = escape_html(&def.name),
        sig_preview = sig_preview,
        source_link = source_link,
        collapsed = collapsed,
        body = body,
    )
}

/// Build a short signature preview for the card header.
fn build_signature_preview(
    def: &DefinitionDiff,
    before: &ParsedFile,
    after: &ParsedFile,
) -> String {
    // Use the after-file's signature lines (or before if removed)
    let (lines, sig) = match def.status {
        DefStatus::Removed => (&before.lines, &def.signature_diff),
        _ => (&after.lines, &def.signature_diff),
    };

    // Collect the source line text for signature lines
    let mut sig_text = String::new();
    for line in &sig.lines {
        let line_num = match def.status {
            DefStatus::Removed => line.left_number,
            _ => line.right_number,
        };
        if let Some(n) = line_num {
            if let Some(src) = lines.get(n - 1) {
                let trimmed = src.trim();
                // Skip the "def name" or "class name" part, just get params
                if sig_text.is_empty() {
                    if let Some(paren_start) = trimmed.find('(') {
                        let rest = &trimmed[paren_start..];
                        // Strip trailing colon
                        let rest = rest.strip_suffix(':').unwrap_or(rest);
                        sig_text.push_str(rest);
                    }
                } else {
                    let trimmed = trimmed.strip_suffix(':').unwrap_or(trimmed);
                    if !sig_text.is_empty() {
                        sig_text.push(' ');
                    }
                    sig_text.push_str(trimmed);
                }
            }
        }
    }

    escape_html(&sig_text)
}

/// Get the docstring text to render, preferring the after-file version.
fn get_docstring_text(
    def: &DefinitionDiff,
    before: &ParsedFile,
    after: &ParsedFile,
) -> Option<String> {
    match def.status {
        DefStatus::Removed => {
            // Use before-file's definition
            before
                .definitions
                .iter()
                .find(|d| d.name == def.name)
                .and_then(|d| d.docstring_text.clone())
        }
        _ => {
            // Use after-file's definition
            after
                .definitions
                .iter()
                .find(|d| d.name == def.name)
                .and_then(|d| d.docstring_text.clone())
        }
    }
}

/// Render a section's diff lines as an HTML diff table.
fn render_section_table(lines: &[DiffLine], before: &ParsedFile, after: &ParsedFile) -> String {
    let mut html = String::from("<table class=\"diff\">\n");
    for line in lines {
        html.push_str(&render_diff_row(line, &before.lines, &after.lines));
    }
    html.push_str("</table>\n");
    html
}

// ---------------------------------------------------------------------------
// Docstring-to-HTML rendering (NumPy-style)
// ---------------------------------------------------------------------------

/// Parse a NumPy-style docstring into HTML.
fn render_docstring_html(docstring: &str) -> String {
    let mut html = String::new();
    let sections = parse_docstring_sections(docstring);

    for section in &sections {
        match section {
            DocSection::Summary(text) => {
                let text = text.trim();
                if !text.is_empty() {
                    html.push_str(&format!(
                        "<p class=\"doc-summary\">{}</p>\n",
                        render_inline_code(&escape_html(text))
                    ));
                }
            }
            DocSection::Section { name, entries } => {
                html.push_str(&format!("<h5 class=\"doc-section-title\">{}</h5>\n", escape_html(name)));
                if !entries.is_empty() {
                    html.push_str("<dl class=\"doc-params\">\n");
                    for entry in entries {
                        html.push_str(&format!(
                            "<dt>{}</dt><dd>{}</dd>\n",
                            render_inline_code(&escape_html(&entry.name)),
                            render_inline_code(&escape_html(&entry.description)),
                        ));
                    }
                    html.push_str("</dl>\n");
                }
            }
        }
    }

    html
}

#[derive(Debug)]
enum DocSection {
    Summary(String),
    Section {
        name: String,
        entries: Vec<DocEntry>,
    },
}

#[derive(Debug)]
struct DocEntry {
    name: String,
    description: String,
}

/// Parse a NumPy-style docstring into sections.
fn parse_docstring_sections(docstring: &str) -> Vec<DocSection> {
    let lines: Vec<&str> = docstring.lines().collect();
    let mut sections = Vec::new();
    let mut i = 0;

    // Find summary: everything before the first section header
    let mut summary_lines = Vec::new();
    while i < lines.len() {
        if is_section_header(&lines, i) {
            break;
        }
        summary_lines.push(lines[i]);
        i += 1;
    }
    let summary = summary_lines.join("\n").trim().to_string();
    if !summary.is_empty() {
        sections.push(DocSection::Summary(summary));
    }

    // Parse remaining sections
    while i < lines.len() {
        if is_section_header(&lines, i) {
            let section_name = lines[i].trim().to_string();
            i += 2; // skip header and underline

            let mut entries = Vec::new();
            while i < lines.len() && !is_section_header(&lines, i) {
                let line = lines[i];
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    i += 1;
                    continue;
                }

                // Check if this looks like an entry name (not deeply indented)
                if !line.starts_with("        ") || trimmed.contains(" : ") || trimmed.contains(" :") {
                    let entry_name = trimmed.to_string();
                    i += 1;

                    // Collect description lines (more indented)
                    let mut desc_lines = Vec::new();
                    while i < lines.len()
                        && !is_section_header(&lines, i)
                        && (lines[i].trim().is_empty()
                            || lines[i].starts_with("        ")
                            || (lines[i].starts_with("    ")
                                && !lines[i].trim().contains(" :")
                                && desc_lines.is_empty()))
                    {
                        let l = lines[i].trim();
                        if l.is_empty() && desc_lines.is_empty() {
                            i += 1;
                            continue;
                        }
                        desc_lines.push(l);
                        i += 1;
                    }

                    entries.push(DocEntry {
                        name: entry_name,
                        description: desc_lines.join(" "),
                    });
                } else {
                    i += 1;
                }
            }

            sections.push(DocSection::Section {
                name: section_name,
                entries,
            });
        } else {
            i += 1;
        }
    }

    sections
}

/// Check if lines[i] is a section header (name followed by dashes underline).
fn is_section_header(lines: &[&str], i: usize) -> bool {
    if i + 1 >= lines.len() {
        return false;
    }
    let name = lines[i].trim();
    let underline = lines[i + 1].trim();
    if name.is_empty() || underline.is_empty() {
        return false;
    }
    // Underline must be all dashes and roughly match the header length
    underline.chars().all(|c| c == '-') && underline.len() >= 3
}

/// Replace `backtick-wrapped` text with `<code>` tags.
fn render_inline_code(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '`' {
            let mut code = String::new();
            let mut found_close = false;
            for c2 in chars.by_ref() {
                if c2 == '`' {
                    found_close = true;
                    break;
                }
                code.push(c2);
            }
            if found_close && !code.is_empty() {
                result.push_str(&format!("<code>{}</code>", code));
            } else {
                result.push('`');
                result.push_str(&code);
            }
        } else {
            result.push(c);
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Spec-view rendering (docs.rs-style annotated view)
// ---------------------------------------------------------------------------

/// Render syntax-highlighted source lines for a given line range.
///
/// Walks the tokens in the parsed file, picks those within the range, and
/// produces highlighted `<pre>` content with line numbers.
pub fn render_source_lines(parsed: &ParsedFile, range: &LineRange) -> String {
    let mut html = String::new();

    for line_idx in range.start..=range.end {
        let line_src = parsed.lines.get(line_idx).map(|s| s.as_str()).unwrap_or("");
        let line_num = line_idx + 1; // 1-based display

        // Collect tokens on this line
        let line_tokens: Vec<&crate::parse::Token> = parsed
            .tokens
            .iter()
            .filter(|t| t.line == line_idx)
            .collect();

        let content = if line_tokens.is_empty() {
            escape_html(line_src)
        } else {
            render_source_line_tokens(line_src, &line_tokens)
        };

        html.push_str(&format!(
            "<span class=\"spec-line\"><span class=\"spec-ln\">{num}</span>{content}</span>\n",
            num = line_num,
            content = content,
        ));
    }

    html
}

/// Render a single source line with syntax highlighting from tokens.
fn render_source_line_tokens(source_line: &str, tokens: &[&crate::parse::Token]) -> String {
    let mut result = String::new();
    let mut last_end: usize = 0;

    for token in tokens {
        let col = token.col;
        // Add any gap (whitespace/untokenized text) before this token
        if col > last_end {
            result.push_str(&escape_html(&source_line[last_end..col]));
        }

        let token_end = col + token.text.len();
        let css = token_css_class(token.kind);
        let text = escape_html(&token.text);

        if css.is_empty() {
            result.push_str(&text);
        } else {
            result.push_str(&format!("<span class=\"{css}\">{text}</span>"));
        }

        last_end = token_end;
    }

    // Append any trailing text
    if last_end < source_line.len() {
        result.push_str(&escape_html(&source_line[last_end..]));
    }

    result
}

/// Render a docs.rs-style annotated spec view for a single parsed file.
pub fn render_spec_file(parsed: &ParsedFile, file_url: Option<&str>) -> String {
    let mut html = String::new();

    // Expand/collapse all controls
    html.push_str(
        "<div class=\"card-controls\">\
         <button onclick=\"expandAllSpecCards()\">Expand all</button> \
         <button onclick=\"collapseAllSpecCards()\">Collapse all</button>\
         </div>\n",
    );

    // 1. Module docstring at top
    if let Some(ref mod_doc) = parsed.module_docstring {
        html.push_str("<div class=\"spec-module-doc\">");
        html.push_str(&render_module_docstring_html(&mod_doc.text));
        html.push_str("</div>\n");
    }

    // 2. Preamble: everything before the first definition (imports/constants)
    let first_def_line = parsed
        .definitions
        .iter()
        .map(|d| {
            d.decorator_lines
                .as_ref()
                .map(|dl| dl.start)
                .unwrap_or(d.signature_lines.start)
        })
        .min();

    // Module docstring end line (if any)
    let after_docstring = parsed
        .module_docstring
        .as_ref()
        .map(|md| md.lines.end + 1)
        .unwrap_or(0);

    let preamble_end = first_def_line.unwrap_or(parsed.lines.len()).saturating_sub(1);

    if after_docstring <= preamble_end && preamble_end < parsed.lines.len() {
        // Check if there's actual content in the preamble range
        let has_content = (after_docstring..=preamble_end)
            .any(|i| !parsed.lines.get(i).map(|l| l.trim().is_empty()).unwrap_or(true));
        if has_content {
            let range = LineRange {
                start: after_docstring,
                end: preamble_end,
            };
            html.push_str(
                "<details class=\"spec-preamble\">\
                 <summary>Imports &amp; Constants</summary>\
                 <pre class=\"spec-code\">",
            );
            html.push_str(&render_source_lines(parsed, &range));
            html.push_str("</pre></details>\n");
        }
    }

    // 3. Definitions grouped by type
    let classes: Vec<&crate::parse::Definition> = parsed
        .definitions
        .iter()
        .filter(|d| d.kind == DefKind::Class)
        .collect();
    let functions: Vec<&crate::parse::Definition> = parsed
        .definitions
        .iter()
        .filter(|d| d.kind == DefKind::Function)
        .collect();

    if !classes.is_empty() {
        html.push_str("<h3 class=\"spec-section-heading\">Classes</h3>\n");
        for def in &classes {
            html.push_str(&render_spec_definition_card(parsed, def, file_url));
        }
    }

    if !functions.is_empty() {
        html.push_str("<h3 class=\"spec-section-heading\">Functions</h3>\n");
        for def in &functions {
            html.push_str(&render_spec_definition_card(parsed, def, file_url));
        }
    }

    html
}

/// Render a single definition as a spec-view card.
fn render_spec_definition_card(
    parsed: &ParsedFile,
    def: &crate::parse::Definition,
    file_url: Option<&str>,
) -> String {
    let kind_label = match def.kind {
        DefKind::Function => "def",
        DefKind::Class => "class",
    };

    // Build the signature display
    let sig = build_spec_signature(parsed, def);

    let mut body = String::new();

    // Docstring
    if let Some(ref text) = def.docstring_text {
        body.push_str("<div class=\"def-docstring\">");
        body.push_str(&render_docstring_html(text));
        body.push_str("</div>\n");
    }

    // Collapsible source
    let source_range = compute_def_full_range(def);
    body.push_str("<details class=\"spec-source\">");
    body.push_str("<summary>Source</summary>");
    body.push_str("<pre class=\"spec-code\">");
    body.push_str(&render_source_lines(parsed, &source_range));
    body.push_str("</pre></details>\n");

    // Build source link for the right edge of the header.
    // signature_lines.start is 0-based; GitHub lines are 1-based.
    let source_link = match file_url {
        Some(url) => {
            let line = def.signature_lines.start + 1;
            format!(
                "<a class=\"source-link\" href=\"{url}#L{line}\" target=\"_blank\" \
                 title=\"View source on GitHub\" onclick=\"event.stopPropagation()\">github</a>",
                url = escape_html(url),
                line = line,
            )
        }
        None => String::new(),
    };

    format!(
        "<div class=\"spec-card\">\
         <div class=\"spec-card-header\" onclick=\"toggleSpecCard(this)\">\
         <span class=\"spec-kind\">{kind}</span>\
         <span class=\"spec-name\">{name}</span>\
         <span class=\"spec-sig\">{sig}</span>\
         {source_link}\
         </div>\
         <div class=\"spec-card-body\">\
         {body}\
         </div>\
         </div>\n",
        kind = kind_label,
        name = escape_html(&def.name),
        sig = sig,
        source_link = source_link,
        body = body,
    )
}

/// Build a short signature preview for a spec card header.
fn build_spec_signature(parsed: &ParsedFile, def: &crate::parse::Definition) -> String {
    let mut sig_text = String::new();
    for line_idx in def.signature_lines.start..=def.signature_lines.end {
        if let Some(src) = parsed.lines.get(line_idx) {
            let trimmed = src.trim();
            if sig_text.is_empty() {
                // Skip "def name" or "class name", get from first paren
                if let Some(paren_start) = trimmed.find('(') {
                    let rest = &trimmed[paren_start..];
                    let rest = rest.strip_suffix(':').unwrap_or(rest);
                    sig_text.push_str(rest);
                }
            } else {
                let trimmed = trimmed.strip_suffix(':').unwrap_or(trimmed);
                sig_text.push(' ');
                sig_text.push_str(trimmed);
            }
        }
    }
    escape_html(&sig_text)
}

/// Compute the full source range of a definition (decorators through body end).
fn compute_def_full_range(def: &crate::parse::Definition) -> LineRange {
    let start = def
        .decorator_lines
        .as_ref()
        .map(|dl| dl.start)
        .unwrap_or(def.signature_lines.start);

    let end = def
        .body_lines
        .as_ref()
        .map(|bl| bl.end)
        .unwrap_or(def.signature_lines.end);

    LineRange { start, end }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{diff_added, diff_definitions, diff_files};
    use crate::parse::parse_source;

    #[test]
    fn test_render_unchanged() {
        let source = "x = 1\n";
        let before = parse_source(source).unwrap();
        let after = parse_source(source).unwrap();
        let diff = diff_files(&before, &after);
        let html = render_diff_table(&diff, &before, &after);
        assert!(html.contains("<table class=\"diff\">"));
        assert!(html.contains("</table>"));
        assert!(html.contains("<td class=\"ln\">1</td>"));
    }

    #[test]
    fn test_render_modified_has_classes() {
        let before = parse_source("x = 1\n").unwrap();
        let after = parse_source("y = 1\n").unwrap();
        let diff = diff_files(&before, &after);
        let html = render_diff_table(&diff, &before, &after);
        assert!(html.contains("class=\"mod\""), "should have modified class");
        assert!(
            html.contains("tok-del") || html.contains("tok-add"),
            "should have token-level highlighting"
        );
    }

    #[test]
    fn test_render_added_file() {
        let after = parse_source("x = 1\n").unwrap();
        let diff = diff_added(&after);
        let html = render_added_table(&diff, &after);
        assert!(html.contains("class=\"add\""));
    }

    #[test]
    fn test_html_escaping() {
        let source = "x = '<>&\"'\n";
        let before = parse_source(source).unwrap();
        let after = parse_source(source).unwrap();
        let diff = diff_files(&before, &after);
        let html = render_diff_table(&diff, &before, &after);
        // Unchanged lines use raw escaped source
        assert!(html.contains("&lt;"));
        assert!(html.contains("&gt;"));
        assert!(html.contains("&amp;"));
    }

    #[test]
    fn test_render_definition_cards_structure() {
        let before_src = "\
def foo(a: int) -> bool:
    \"\"\"Check something.\"\"\"
    return True
";
        let after_src = "\
def foo(a: int) -> bool:
    \"\"\"Check something.\"\"\"
    return False
";
        let before = parse_source(before_src).unwrap();
        let after = parse_source(after_src).unwrap();
        let diff = diff_definitions(&before, &after);
        let html = render_definition_cards(&diff, &before, &after, None, None);

        assert!(html.contains("def-card"), "should contain card");
        assert!(html.contains("def-header"), "should contain header");
        assert!(html.contains("def-name"), "should contain name");
        assert!(html.contains("foo"), "should contain function name");
        assert!(html.contains("def-docstring"), "should contain docstring section");
        assert!(html.contains("Check something"), "should render docstring text");
    }

    #[test]
    fn test_render_docstring_html_numpy() {
        let docstring = "\
Check if a equals b.

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
";
        let html = render_docstring_html(docstring);
        assert!(html.contains("doc-summary"), "should have summary");
        assert!(html.contains("Check if a equals b"), "summary text");
        assert!(html.contains("doc-params"), "should have params");
        assert!(html.contains("a : int"), "should have param name");
        assert!(html.contains("The first value"), "should have param desc");
        assert!(html.contains("Returns"), "should have Returns section");
    }

    #[test]
    fn test_render_inline_code() {
        assert_eq!(
            render_inline_code("use `foo` here"),
            "use <code>foo</code> here"
        );
        assert_eq!(render_inline_code("no backticks"), "no backticks");
    }

    #[test]
    fn test_card_unchanged_collapsed() {
        let source = "def foo():\n    pass\n";
        let before = parse_source(source).unwrap();
        let after = parse_source(source).unwrap();
        let diff = diff_definitions(&before, &after);
        let html = render_definition_cards(&diff, &before, &after, None, None);
        assert!(
            html.contains("def-card unchanged"),
            "unchanged card should have unchanged class"
        );
        assert!(
            html.contains("def-body collapsed"),
            "unchanged card body should start collapsed"
        );
    }
}
