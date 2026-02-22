use std::collections::HashSet;

use crate::parse::{DefKind, Definition, LineRange, ParsedFile, SourceBlock, Token};
use similar::{capture_diff_slices, Algorithm};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenHighlight {
    Unchanged,
    Added,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct RenderedToken {
    pub text: String,
    pub kind: &'static str,
    pub highlight: TokenHighlight,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineStatus {
    Unchanged,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub left_number: Option<usize>,  // 1-based
    pub right_number: Option<usize>, // 1-based
    pub left_tokens: Vec<RenderedToken>,
    pub right_tokens: Vec<RenderedToken>,
    pub status: LineStatus,
}

#[derive(Debug, Clone, Default)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
    pub modified: usize,
    pub unchanged: usize,
}

#[derive(Debug)]
pub struct FileDiff {
    pub lines: Vec<DiffLine>,
    pub stats: DiffStats,
}

#[derive(Debug)]
struct TaggedToken<'a> {
    token: &'a Token,
    tag: similar::ChangeTag,
}

// ---------------------------------------------------------------------------
// Block-aware diff: match top-level definitions by name, then diff within.
// ---------------------------------------------------------------------------

/// Diff two parsed files using block-level matching to handle moved definitions.
pub fn diff_files(before: &ParsedFile, after: &ParsedFile) -> FileDiff {
    if before.blocks.is_empty() && after.blocks.is_empty() {
        return diff_files_flat(before, after);
    }

    let mut all_lines = Vec::new();
    let mut matched_before: HashSet<usize> = HashSet::new();

    // Collect before unnamed blocks for positional matching.
    let before_unnamed: Vec<usize> = before
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.name.is_none())
        .map(|(i, _)| i)
        .collect();
    let mut unnamed_idx = 0usize;

    // Walk after-file blocks in order.
    for after_block in &after.blocks {
        if let Some(after_name) = &after_block.name {
            // Match named block by name.
            if let Some((bi, _)) = before.blocks.iter().enumerate().find(|(i, b)| {
                !matched_before.contains(i) && b.name.as_deref() == Some(after_name.as_str())
            }) {
                matched_before.insert(bi);
                diff_block_pair(
                    before,
                    &before.blocks[bi],
                    after,
                    after_block,
                    &mut all_lines,
                );
            } else {
                // New definition — all lines added.
                emit_added_block(after, after_block, &mut all_lines);
            }
        } else {
            // Unnamed block — match by position.
            if unnamed_idx < before_unnamed.len() {
                let bi = before_unnamed[unnamed_idx];
                matched_before.insert(bi);
                unnamed_idx += 1;
                diff_block_pair(before, &before.blocks[bi], after, after_block, &mut all_lines);
            } else {
                emit_added_block(after, after_block, &mut all_lines);
            }
        }
    }

    // Emit unmatched before blocks as removed.
    for (i, before_block) in before.blocks.iter().enumerate() {
        if !matched_before.contains(&i) {
            emit_removed_block(before, before_block, &mut all_lines);
        }
    }

    let stats = compute_stats(&all_lines);
    FileDiff {
        lines: all_lines,
        stats,
    }
}

/// Get the slice of tokens whose line numbers fall within [block.start_line, block.end_line].
fn tokens_for_block<'a>(tokens: &'a [Token], block: &SourceBlock) -> &'a [Token] {
    let start = tokens
        .partition_point(|t| t.line < block.start_line);
    let end = tokens
        .partition_point(|t| t.line <= block.end_line);
    &tokens[start..end]
}

/// Diff two matched blocks and append DiffLines.
fn diff_block_pair(
    before: &ParsedFile,
    before_block: &SourceBlock,
    after: &ParsedFile,
    after_block: &SourceBlock,
    output: &mut Vec<DiffLine>,
) {
    let before_toks = tokens_for_block(&before.tokens, before_block);
    let after_toks = tokens_for_block(&after.tokens, after_block);

    diff_token_regions(
        before_toks,
        after_toks,
        before_block.start_line,
        before_block.end_line,
        after_block.start_line,
        after_block.end_line,
        output,
    );
}

fn emit_added_block(file: &ParsedFile, block: &SourceBlock, output: &mut Vec<DiffLine>) {
    let toks = tokens_for_block(&file.tokens, block);
    let mut tok_idx = 0;
    for line in block.start_line..=block.end_line {
        let mut right_tokens = Vec::new();
        while tok_idx < toks.len() && toks[tok_idx].line == line {
            right_tokens.push(RenderedToken {
                text: toks[tok_idx].text.clone(),
                kind: toks[tok_idx].kind,
                highlight: TokenHighlight::Added,
            });
            tok_idx += 1;
        }
        output.push(DiffLine {
            left_number: None,
            right_number: Some(line + 1),
            left_tokens: vec![],
            right_tokens,
            status: LineStatus::Added,
        });
    }
}

fn emit_removed_block(file: &ParsedFile, block: &SourceBlock, output: &mut Vec<DiffLine>) {
    let toks = tokens_for_block(&file.tokens, block);
    let mut tok_idx = 0;
    for line in block.start_line..=block.end_line {
        let mut left_tokens = Vec::new();
        while tok_idx < toks.len() && toks[tok_idx].line == line {
            left_tokens.push(RenderedToken {
                text: toks[tok_idx].text.clone(),
                kind: toks[tok_idx].kind,
                highlight: TokenHighlight::Deleted,
            });
            tok_idx += 1;
        }
        output.push(DiffLine {
            left_number: Some(line + 1),
            right_number: None,
            left_tokens,
            right_tokens: vec![],
            status: LineStatus::Removed,
        });
    }
}

// ---------------------------------------------------------------------------
// Core token-region diff: diffs two token slices and maps back to lines.
// ---------------------------------------------------------------------------

pub fn diff_token_regions(
    before_tokens: &[Token],
    after_tokens: &[Token],
    before_start_line: usize,
    before_end_line: usize,
    after_start_line: usize,
    after_end_line: usize,
    output: &mut Vec<DiffLine>,
) {
    let ops = capture_diff_slices(Algorithm::Myers, before_tokens, after_tokens);

    let mut before_tagged: Vec<TaggedToken> = Vec::new();
    let mut after_tagged: Vec<TaggedToken> = Vec::new();

    for op in &ops {
        match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..*len {
                    before_tagged.push(TaggedToken {
                        token: &before_tokens[old_index + i],
                        tag: similar::ChangeTag::Equal,
                    });
                    after_tagged.push(TaggedToken {
                        token: &after_tokens[new_index + i],
                        tag: similar::ChangeTag::Equal,
                    });
                }
            }
            similar::DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..*old_len {
                    before_tagged.push(TaggedToken {
                        token: &before_tokens[old_index + i],
                        tag: similar::ChangeTag::Delete,
                    });
                }
            }
            similar::DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..*new_len {
                    after_tagged.push(TaggedToken {
                        token: &after_tokens[new_index + i],
                        tag: similar::ChangeTag::Insert,
                    });
                }
            }
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                for i in 0..*old_len {
                    before_tagged.push(TaggedToken {
                        token: &before_tokens[old_index + i],
                        tag: similar::ChangeTag::Delete,
                    });
                }
                for i in 0..*new_len {
                    after_tagged.push(TaggedToken {
                        token: &after_tokens[new_index + i],
                        tag: similar::ChangeTag::Insert,
                    });
                }
            }
        }
    }

    let before_num = before_end_line - before_start_line + 1;
    let after_num = after_end_line - after_start_line + 1;

    let before_by_line = group_by_line_offset(&before_tagged, before_num, before_start_line);
    let after_by_line = group_by_line_offset(&after_tagged, after_num, after_start_line);

    let before_status: Vec<LineClass> = before_by_line.iter().map(classify_line_tokens).collect();
    let after_status: Vec<LineClass> = after_by_line.iter().map(classify_line_tokens).collect();

    let mut li = 0usize;
    let mut ri = 0usize;

    while li < before_num || ri < after_num {
        let l_class = if li < before_status.len() {
            before_status[li]
        } else {
            LineClass::AllChanged
        };
        let r_class = if ri < after_status.len() {
            after_status[ri]
        } else {
            LineClass::AllChanged
        };

        match (li < before_num, ri < after_num) {
            (true, true) => match (l_class, r_class) {
                (LineClass::Unchanged, LineClass::Unchanged) => {
                    output.push(DiffLine {
                        left_number: Some(before_start_line + li + 1),
                        right_number: Some(after_start_line + ri + 1),
                        left_tokens: render_line_tokens(&before_by_line[li]),
                        right_tokens: render_line_tokens(&after_by_line[ri]),
                        status: LineStatus::Unchanged,
                    });
                    li += 1;
                    ri += 1;
                }
                (LineClass::AllChanged, LineClass::AllChanged)
                | (LineClass::Mixed, LineClass::Mixed)
                | (LineClass::Mixed, LineClass::AllChanged)
                | (LineClass::AllChanged, LineClass::Mixed) => {
                    output.push(DiffLine {
                        left_number: Some(before_start_line + li + 1),
                        right_number: Some(after_start_line + ri + 1),
                        left_tokens: render_line_tokens(&before_by_line[li]),
                        right_tokens: render_line_tokens(&after_by_line[ri]),
                        status: LineStatus::Modified,
                    });
                    li += 1;
                    ri += 1;
                }
                (LineClass::AllChanged, _) | (LineClass::Mixed, LineClass::Unchanged) => {
                    output.push(DiffLine {
                        left_number: Some(before_start_line + li + 1),
                        right_number: None,
                        left_tokens: render_line_tokens(&before_by_line[li]),
                        right_tokens: vec![],
                        status: LineStatus::Removed,
                    });
                    li += 1;
                }
                (_, LineClass::AllChanged) | (LineClass::Unchanged, LineClass::Mixed) => {
                    output.push(DiffLine {
                        left_number: None,
                        right_number: Some(after_start_line + ri + 1),
                        left_tokens: vec![],
                        right_tokens: render_line_tokens(&after_by_line[ri]),
                        status: LineStatus::Added,
                    });
                    ri += 1;
                }
            },
            (true, false) => {
                output.push(DiffLine {
                    left_number: Some(before_start_line + li + 1),
                    right_number: None,
                    left_tokens: render_line_tokens(&before_by_line[li]),
                    right_tokens: vec![],
                    status: LineStatus::Removed,
                });
                li += 1;
            }
            (false, true) => {
                output.push(DiffLine {
                    left_number: None,
                    right_number: Some(after_start_line + ri + 1),
                    left_tokens: vec![],
                    right_tokens: render_line_tokens(&after_by_line[ri]),
                    status: LineStatus::Added,
                });
                ri += 1;
            }
            (false, false) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// Flat file diff (no block matching) — used as fallback.
// ---------------------------------------------------------------------------

fn diff_files_flat(before: &ParsedFile, after: &ParsedFile) -> FileDiff {
    let mut lines = Vec::new();
    let before_end = if before.lines.is_empty() {
        0
    } else {
        before.lines.len() - 1
    };
    let after_end = if after.lines.is_empty() {
        0
    } else {
        after.lines.len() - 1
    };
    diff_token_regions(
        &before.tokens,
        &after.tokens,
        0,
        before_end,
        0,
        after_end,
        &mut lines,
    );
    let stats = compute_stats(&lines);
    FileDiff { lines, stats }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum LineClass {
    Unchanged,
    AllChanged,
    Mixed,
}

fn group_by_line_offset<'a>(
    tagged: &'a [TaggedToken<'a>],
    num_lines: usize,
    line_offset: usize,
) -> Vec<Vec<&'a TaggedToken<'a>>> {
    let mut groups: Vec<Vec<&TaggedToken>> = vec![vec![]; num_lines];
    for tt in tagged {
        let local = tt.token.line.wrapping_sub(line_offset);
        if local < num_lines {
            groups[local].push(tt);
        }
    }
    groups
}

fn classify_line_tokens(tokens: &Vec<&TaggedToken>) -> LineClass {
    if tokens.is_empty() {
        return LineClass::Unchanged;
    }
    let all_equal = tokens.iter().all(|t| t.tag == similar::ChangeTag::Equal);
    let all_changed = tokens.iter().all(|t| t.tag != similar::ChangeTag::Equal);
    if all_equal {
        LineClass::Unchanged
    } else if all_changed {
        LineClass::AllChanged
    } else {
        LineClass::Mixed
    }
}

fn render_line_tokens(tokens: &[&TaggedToken]) -> Vec<RenderedToken> {
    tokens
        .iter()
        .map(|tt| RenderedToken {
            text: tt.token.text.clone(),
            kind: tt.token.kind,
            highlight: match tt.tag {
                similar::ChangeTag::Equal => TokenHighlight::Unchanged,
                similar::ChangeTag::Delete => TokenHighlight::Deleted,
                similar::ChangeTag::Insert => TokenHighlight::Added,
            },
        })
        .collect()
}

pub fn compute_stats(lines: &[DiffLine]) -> DiffStats {
    let mut stats = DiffStats::default();
    for line in lines {
        match line.status {
            LineStatus::Unchanged => stats.unchanged += 1,
            LineStatus::Added => stats.added += 1,
            LineStatus::Removed => stats.removed += 1,
            LineStatus::Modified => stats.modified += 1,
        }
    }
    stats
}

/// Create a FileDiff representing an entirely new file.
pub fn diff_added(file: &ParsedFile) -> FileDiff {
    let tokens_by_line = {
        let mut groups: Vec<Vec<&Token>> = vec![vec![]; file.lines.len()];
        for tok in &file.tokens {
            if tok.line < file.lines.len() {
                groups[tok.line].push(tok);
            }
        }
        groups
    };

    let lines: Vec<DiffLine> = file
        .lines
        .iter()
        .enumerate()
        .map(|(i, _)| DiffLine {
            left_number: None,
            right_number: Some(i + 1),
            left_tokens: vec![],
            right_tokens: tokens_by_line[i]
                .iter()
                .map(|t| RenderedToken {
                    text: t.text.clone(),
                    kind: t.kind,
                    highlight: TokenHighlight::Added,
                })
                .collect(),
            status: LineStatus::Added,
        })
        .collect();

    let stats = compute_stats(&lines);
    FileDiff { lines, stats }
}

/// Create a FileDiff representing an entirely deleted file.
pub fn diff_removed(file: &ParsedFile) -> FileDiff {
    let tokens_by_line = {
        let mut groups: Vec<Vec<&Token>> = vec![vec![]; file.lines.len()];
        for tok in &file.tokens {
            if tok.line < file.lines.len() {
                groups[tok.line].push(tok);
            }
        }
        groups
    };

    let lines: Vec<DiffLine> = file
        .lines
        .iter()
        .enumerate()
        .map(|(i, _)| DiffLine {
            left_number: Some(i + 1),
            right_number: None,
            left_tokens: tokens_by_line[i]
                .iter()
                .map(|t| RenderedToken {
                    text: t.text.clone(),
                    kind: t.kind,
                    highlight: TokenHighlight::Deleted,
                })
                .collect(),
            right_tokens: vec![],
            status: LineStatus::Removed,
        })
        .collect();

    let stats = compute_stats(&lines);
    FileDiff { lines, stats }
}

// ---------------------------------------------------------------------------
// Definition-level diff: match definitions by name, diff each section.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum DefStatus {
    Unchanged,
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone)]
pub struct SectionDiff {
    pub lines: Vec<DiffLine>,
    pub changed: bool,
}

#[derive(Debug, Clone)]
pub struct DefinitionDiff {
    pub name: String,
    pub kind: DefKind,
    pub status: DefStatus,
    pub decorator_diff: Option<SectionDiff>,
    pub signature_diff: SectionDiff,
    pub docstring_diff: Option<SectionDiff>,
    pub body_diff: Option<SectionDiff>,
}

/// Module-level docstring diff with rendered text for both sides.
#[derive(Debug, Clone)]
pub struct ModuleDocDiff {
    pub section_diff: SectionDiff,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
}

#[derive(Debug)]
pub struct FileDefDiff {
    pub definitions: Vec<DefinitionDiff>,
    pub module_doc_diff: Option<ModuleDocDiff>,
    pub preamble_diff: Option<SectionDiff>,
    pub stats: DiffStats,
}

/// Diff two files at the definition level, producing per-section diffs for each definition.
pub fn diff_definitions(before: &ParsedFile, after: &ParsedFile) -> FileDefDiff {
    let mut def_diffs = Vec::new();
    let mut matched_before: HashSet<usize> = HashSet::new();

    // First definition start line (or end of file)
    let before_first_def = before
        .definitions
        .first()
        .map(|d| {
            d.decorator_lines
                .as_ref()
                .map(|dl| dl.start)
                .unwrap_or(d.signature_lines.start)
        })
        .unwrap_or(before.lines.len());
    let after_first_def = after
        .definitions
        .first()
        .map(|d| {
            d.decorator_lines
                .as_ref()
                .map(|dl| dl.start)
                .unwrap_or(d.signature_lines.start)
        })
        .unwrap_or(after.lines.len());

    // Module docstring: diff line-by-line, separate from preamble
    let module_doc_diff = match (&before.module_docstring, &after.module_docstring) {
        (Some(bd), Some(ad)) => {
            let sd = diff_docstring_lines(before, &bd.lines, after, &ad.lines);
            Some(ModuleDocDiff {
                section_diff: sd,
                before_text: Some(bd.text.clone()),
                after_text: Some(ad.text.clone()),
            })
        }
        (None, Some(ad)) => {
            let sd = diff_optional_section(before, None, after, Some(&ad.lines))
                .unwrap_or(SectionDiff {
                    lines: vec![],
                    changed: true,
                });
            Some(ModuleDocDiff {
                section_diff: sd,
                before_text: None,
                after_text: Some(ad.text.clone()),
            })
        }
        (Some(bd), None) => {
            let sd = diff_optional_section(before, Some(&bd.lines), after, None)
                .unwrap_or(SectionDiff {
                    lines: vec![],
                    changed: true,
                });
            Some(ModuleDocDiff {
                section_diff: sd,
                before_text: Some(bd.text.clone()),
                after_text: None,
            })
        }
        (None, None) => None,
    };

    // Preamble: lines between module docstring (if any) and first definition
    let before_preamble_start = before
        .module_docstring
        .as_ref()
        .map(|d| d.lines.end + 1)
        .unwrap_or(0);
    let after_preamble_start = after
        .module_docstring
        .as_ref()
        .map(|d| d.lines.end + 1)
        .unwrap_or(0);

    let preamble_diff = if before_preamble_start < before_first_def
        || after_preamble_start < after_first_def
    {
        let b_start = before_preamble_start;
        let b_end = before_first_def.saturating_sub(1);
        let a_start = after_preamble_start;
        let a_end = after_first_def.saturating_sub(1);
        if b_start <= b_end || a_start <= a_end {
            let before_toks = if b_start <= b_end {
                tokens_in_range(&before.tokens, b_start, b_end)
            } else {
                &[]
            };
            let after_toks = if a_start <= a_end {
                tokens_in_range(&after.tokens, a_start, a_end)
            } else {
                &[]
            };
            let mut lines = Vec::new();
            diff_token_regions(
                before_toks,
                after_toks,
                b_start,
                b_end.max(b_start),
                a_start,
                a_end.max(a_start),
                &mut lines,
            );
            let changed = lines.iter().any(|l| l.status != LineStatus::Unchanged);
            Some(SectionDiff { lines, changed })
        } else {
            None
        }
    } else {
        None
    };

    // Match after definitions to before definitions by name
    for after_def in &after.definitions {
        if let Some((bi, before_def)) = before
            .definitions
            .iter()
            .enumerate()
            .find(|(i, d)| !matched_before.contains(i) && d.name == after_def.name)
        {
            matched_before.insert(bi);
            let dd = diff_definition_pair(before, before_def, after, after_def);
            def_diffs.push(dd);
        } else {
            // New definition
            def_diffs.push(make_added_def(after, after_def));
        }
    }

    // Emit unmatched before definitions as removed
    for (i, before_def) in before.definitions.iter().enumerate() {
        if !matched_before.contains(&i) {
            def_diffs.push(make_removed_def(before, before_def));
        }
    }

    // Compute aggregate stats
    let mut stats = DiffStats::default();
    if let Some(ref md) = module_doc_diff {
        let s = compute_stats(&md.section_diff.lines);
        stats.added += s.added;
        stats.removed += s.removed;
        stats.modified += s.modified;
        stats.unchanged += s.unchanged;
    }
    if let Some(ref p) = preamble_diff {
        let s = compute_stats(&p.lines);
        stats.added += s.added;
        stats.removed += s.removed;
        stats.modified += s.modified;
        stats.unchanged += s.unchanged;
    }
    for dd in &def_diffs {
        for section in [
            dd.decorator_diff.as_ref(),
            Some(&dd.signature_diff),
            dd.docstring_diff.as_ref(),
            dd.body_diff.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let s = compute_stats(&section.lines);
            stats.added += s.added;
            stats.removed += s.removed;
            stats.modified += s.modified;
            stats.unchanged += s.unchanged;
        }
    }

    FileDefDiff {
        definitions: def_diffs,
        module_doc_diff,
        preamble_diff,
        stats,
    }
}

fn tokens_in_range<'a>(tokens: &'a [Token], start_line: usize, end_line: usize) -> &'a [Token] {
    let start = tokens.partition_point(|t| t.line < start_line);
    let end = tokens.partition_point(|t| t.line <= end_line);
    &tokens[start..end]
}

fn diff_section(
    before: &ParsedFile,
    before_range: &LineRange,
    after: &ParsedFile,
    after_range: &LineRange,
) -> SectionDiff {
    let before_toks = tokens_in_range(&before.tokens, before_range.start, before_range.end);
    let after_toks = tokens_in_range(&after.tokens, after_range.start, after_range.end);
    let mut lines = Vec::new();
    diff_token_regions(
        before_toks,
        after_toks,
        before_range.start,
        before_range.end,
        after_range.start,
        after_range.end,
        &mut lines,
    );
    let changed = lines.iter().any(|l| l.status != LineStatus::Unchanged);
    SectionDiff { lines, changed }
}

fn diff_optional_section(
    before: &ParsedFile,
    before_range: Option<&LineRange>,
    after: &ParsedFile,
    after_range: Option<&LineRange>,
) -> Option<SectionDiff> {
    match (before_range, after_range) {
        (Some(br), Some(ar)) => Some(diff_section(before, br, after, ar)),
        (None, Some(ar)) => {
            let toks = tokens_in_range(&after.tokens, ar.start, ar.end);
            let mut lines = Vec::new();
            for line_num in ar.start..=ar.end {
                let line_toks: Vec<_> = toks.iter().filter(|t| t.line == line_num).collect();
                let right_tokens = line_toks
                    .iter()
                    .map(|t| RenderedToken {
                        text: t.text.clone(),
                        kind: t.kind,
                        highlight: TokenHighlight::Added,
                    })
                    .collect();
                lines.push(DiffLine {
                    left_number: None,
                    right_number: Some(line_num + 1),
                    left_tokens: vec![],
                    right_tokens,
                    status: LineStatus::Added,
                });
            }
            Some(SectionDiff {
                changed: true,
                lines,
            })
        }
        (Some(br), None) => {
            let toks = tokens_in_range(&before.tokens, br.start, br.end);
            let mut lines = Vec::new();
            for line_num in br.start..=br.end {
                let line_toks: Vec<_> = toks.iter().filter(|t| t.line == line_num).collect();
                let left_tokens = line_toks
                    .iter()
                    .map(|t| RenderedToken {
                        text: t.text.clone(),
                        kind: t.kind,
                        highlight: TokenHighlight::Deleted,
                    })
                    .collect();
                lines.push(DiffLine {
                    left_number: Some(line_num + 1),
                    right_number: None,
                    left_tokens,
                    right_tokens: vec![],
                    status: LineStatus::Removed,
                });
            }
            Some(SectionDiff {
                changed: true,
                lines,
            })
        }
        (None, None) => None,
    }
}

/// Line-based diff for docstring sections.
///
/// Tree-sitter parses docstrings as a single `string_content` token, so
/// token-based diffing marks the entire docstring as changed when only a
/// small part differs. This function diffs docstring source lines directly
/// using Myers diff on the text, producing proper per-line status.
fn diff_docstring_lines(
    before: &ParsedFile,
    before_range: &LineRange,
    after: &ParsedFile,
    after_range: &LineRange,
) -> SectionDiff {
    let before_lines: Vec<&str> = (before_range.start..=before_range.end)
        .filter_map(|i| before.lines.get(i).map(|s| s.as_str()))
        .collect();
    let after_lines: Vec<&str> = (after_range.start..=after_range.end)
        .filter_map(|i| after.lines.get(i).map(|s| s.as_str()))
        .collect();

    let ops = capture_diff_slices(Algorithm::Myers, &before_lines, &after_lines);

    let mut diff_lines = Vec::new();

    for op in &ops {
        match op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..*len {
                    diff_lines.push(DiffLine {
                        left_number: Some(before_range.start + old_index + i + 1),
                        right_number: Some(after_range.start + new_index + i + 1),
                        left_tokens: vec![],
                        right_tokens: vec![],
                        status: LineStatus::Unchanged,
                    });
                }
            }
            similar::DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..*old_len {
                    let line_idx = before_range.start + old_index + i;
                    let text = before.lines.get(line_idx).map(|s| s.as_str()).unwrap_or("");
                    diff_lines.push(DiffLine {
                        left_number: Some(line_idx + 1),
                        right_number: None,
                        left_tokens: vec![RenderedToken {
                            text: text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Deleted,
                        }],
                        right_tokens: vec![],
                        status: LineStatus::Removed,
                    });
                }
            }
            similar::DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..*new_len {
                    let line_idx = after_range.start + new_index + i;
                    let text = after.lines.get(line_idx).map(|s| s.as_str()).unwrap_or("");
                    diff_lines.push(DiffLine {
                        left_number: None,
                        right_number: Some(line_idx + 1),
                        left_tokens: vec![],
                        right_tokens: vec![RenderedToken {
                            text: text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Added,
                        }],
                        status: LineStatus::Added,
                    });
                }
            }
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                // Pair up lines for Modified status, then emit remaining as Added/Removed
                let pairs = (*old_len).min(*new_len);
                for i in 0..pairs {
                    let b_idx = before_range.start + old_index + i;
                    let a_idx = after_range.start + new_index + i;
                    let b_text = before.lines.get(b_idx).map(|s| s.as_str()).unwrap_or("");
                    let a_text = after.lines.get(a_idx).map(|s| s.as_str()).unwrap_or("");
                    diff_lines.push(DiffLine {
                        left_number: Some(b_idx + 1),
                        right_number: Some(a_idx + 1),
                        left_tokens: vec![RenderedToken {
                            text: b_text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Deleted,
                        }],
                        right_tokens: vec![RenderedToken {
                            text: a_text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Added,
                        }],
                        status: LineStatus::Modified,
                    });
                }
                for i in pairs..*old_len {
                    let b_idx = before_range.start + old_index + i;
                    let b_text = before.lines.get(b_idx).map(|s| s.as_str()).unwrap_or("");
                    diff_lines.push(DiffLine {
                        left_number: Some(b_idx + 1),
                        right_number: None,
                        left_tokens: vec![RenderedToken {
                            text: b_text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Deleted,
                        }],
                        right_tokens: vec![],
                        status: LineStatus::Removed,
                    });
                }
                for i in pairs..*new_len {
                    let a_idx = after_range.start + new_index + i;
                    let a_text = after.lines.get(a_idx).map(|s| s.as_str()).unwrap_or("");
                    diff_lines.push(DiffLine {
                        left_number: None,
                        right_number: Some(a_idx + 1),
                        left_tokens: vec![],
                        right_tokens: vec![RenderedToken {
                            text: a_text.to_string(),
                            kind: "string_content",
                            highlight: TokenHighlight::Added,
                        }],
                        status: LineStatus::Added,
                    });
                }
            }
        }
    }

    let changed = diff_lines.iter().any(|l| l.status != LineStatus::Unchanged);
    SectionDiff {
        lines: diff_lines,
        changed,
    }
}

fn diff_optional_docstring(
    before: &ParsedFile,
    before_range: Option<&LineRange>,
    after: &ParsedFile,
    after_range: Option<&LineRange>,
) -> Option<SectionDiff> {
    match (before_range, after_range) {
        (Some(br), Some(ar)) => Some(diff_docstring_lines(before, br, after, ar)),
        // For added/removed, delegate to the existing logic
        (None, Some(_)) | (Some(_), None) | (None, None) => {
            diff_optional_section(before, before_range, after, after_range)
        }
    }
}

fn diff_definition_pair(
    before: &ParsedFile,
    before_def: &Definition,
    after: &ParsedFile,
    after_def: &Definition,
) -> DefinitionDiff {
    let decorator_diff = diff_optional_section(
        before,
        before_def.decorator_lines.as_ref(),
        after,
        after_def.decorator_lines.as_ref(),
    );
    let signature_diff = diff_section(
        before,
        &before_def.signature_lines,
        after,
        &after_def.signature_lines,
    );
    let docstring_diff = diff_optional_docstring(
        before,
        before_def.docstring_lines.as_ref(),
        after,
        after_def.docstring_lines.as_ref(),
    );
    let body_diff = diff_optional_section(
        before,
        before_def.body_lines.as_ref(),
        after,
        after_def.body_lines.as_ref(),
    );

    let any_changed = decorator_diff.as_ref().is_some_and(|d| d.changed)
        || signature_diff.changed
        || docstring_diff.as_ref().is_some_and(|d| d.changed)
        || body_diff.as_ref().is_some_and(|d| d.changed);

    let status = if any_changed {
        DefStatus::Modified
    } else {
        DefStatus::Unchanged
    };

    DefinitionDiff {
        name: after_def.name.clone(),
        kind: after_def.kind.clone(),
        status,
        decorator_diff,
        signature_diff,
        docstring_diff,
        body_diff,
    }
}

fn make_section_added(file: &ParsedFile, range: &LineRange) -> SectionDiff {
    let toks = tokens_in_range(&file.tokens, range.start, range.end);
    let mut lines = Vec::new();
    for line_num in range.start..=range.end {
        let line_toks: Vec<_> = toks.iter().filter(|t| t.line == line_num).collect();
        let right_tokens = line_toks
            .iter()
            .map(|t| RenderedToken {
                text: t.text.clone(),
                kind: t.kind,
                highlight: TokenHighlight::Added,
            })
            .collect();
        lines.push(DiffLine {
            left_number: None,
            right_number: Some(line_num + 1),
            left_tokens: vec![],
            right_tokens,
            status: LineStatus::Added,
        });
    }
    SectionDiff {
        changed: true,
        lines,
    }
}

fn make_section_removed(file: &ParsedFile, range: &LineRange) -> SectionDiff {
    let toks = tokens_in_range(&file.tokens, range.start, range.end);
    let mut lines = Vec::new();
    for line_num in range.start..=range.end {
        let line_toks: Vec<_> = toks.iter().filter(|t| t.line == line_num).collect();
        let left_tokens = line_toks
            .iter()
            .map(|t| RenderedToken {
                text: t.text.clone(),
                kind: t.kind,
                highlight: TokenHighlight::Deleted,
            })
            .collect();
        lines.push(DiffLine {
            left_number: Some(line_num + 1),
            right_number: None,
            left_tokens,
            right_tokens: vec![],
            status: LineStatus::Removed,
        });
    }
    SectionDiff {
        changed: true,
        lines,
    }
}

fn make_added_def(file: &ParsedFile, def: &Definition) -> DefinitionDiff {
    DefinitionDiff {
        name: def.name.clone(),
        kind: def.kind.clone(),
        status: DefStatus::Added,
        decorator_diff: def
            .decorator_lines
            .as_ref()
            .map(|r| make_section_added(file, r)),
        signature_diff: make_section_added(file, &def.signature_lines),
        docstring_diff: def
            .docstring_lines
            .as_ref()
            .map(|r| make_section_added(file, r)),
        body_diff: def
            .body_lines
            .as_ref()
            .map(|r| make_section_added(file, r)),
    }
}

fn make_removed_def(file: &ParsedFile, def: &Definition) -> DefinitionDiff {
    DefinitionDiff {
        name: def.name.clone(),
        kind: def.kind.clone(),
        status: DefStatus::Removed,
        decorator_diff: def
            .decorator_lines
            .as_ref()
            .map(|r| make_section_removed(file, r)),
        signature_diff: make_section_removed(file, &def.signature_lines),
        docstring_diff: def
            .docstring_lines
            .as_ref()
            .map(|r| make_section_removed(file, r)),
        body_diff: def
            .body_lines
            .as_ref()
            .map(|r| make_section_removed(file, r)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_source;

    #[test]
    fn test_identical_files_no_diff() {
        let source = "x = 1\ny = 2\n";
        let before = parse_source(source).unwrap();
        let after = parse_source(source).unwrap();
        let diff = diff_files(&before, &after);
        assert!(
            diff.lines.iter().all(|l| l.status == LineStatus::Unchanged),
            "identical files should have no changes"
        );
        assert_eq!(diff.stats.added, 0);
        assert_eq!(diff.stats.removed, 0);
        assert_eq!(diff.stats.modified, 0);
    }

    #[test]
    fn test_whitespace_only_no_diff() {
        let before = parse_source("x = 1\ny = 2\n").unwrap();
        let after = parse_source("x  =  1\ny  =  2\n").unwrap();
        let diff = diff_files(&before, &after);
        assert!(
            diff.lines.iter().all(|l| l.status == LineStatus::Unchanged),
            "whitespace-only changes should produce no diff"
        );
    }

    #[test]
    fn test_variable_rename() {
        let before = parse_source("x = 1\n").unwrap();
        let after = parse_source("y = 1\n").unwrap();
        let diff = diff_files(&before, &after);
        assert!(
            diff.lines.iter().any(|l| l.status == LineStatus::Modified),
            "variable rename should produce a modified line"
        );
        let modified = diff
            .lines
            .iter()
            .find(|l| l.status == LineStatus::Modified)
            .unwrap();
        assert!(modified
            .left_tokens
            .iter()
            .any(|t| t.highlight == TokenHighlight::Deleted));
        assert!(modified
            .right_tokens
            .iter()
            .any(|t| t.highlight == TokenHighlight::Added));
    }

    #[test]
    fn test_added_function() {
        let before = parse_source("x = 1\n").unwrap();
        let after = parse_source("x = 1\ndef foo():\n    pass\n").unwrap();
        let diff = diff_files(&before, &after);
        assert!(
            diff.lines.iter().any(|l| l.status == LineStatus::Added),
            "added function should produce added lines"
        );
        assert!(diff.stats.added > 0);
    }

    #[test]
    fn test_removed_lines() {
        let before = parse_source("x = 1\ny = 2\nz = 3\n").unwrap();
        let after = parse_source("x = 1\n").unwrap();
        let diff = diff_files(&before, &after);
        assert!(
            diff.lines.iter().any(|l| l.status == LineStatus::Removed),
            "removed lines should be detected"
        );
        assert!(diff.stats.removed > 0);
    }

    #[test]
    fn test_diff_added_file() {
        let file = parse_source("x = 1\ny = 2\n").unwrap();
        let diff = diff_added(&file);
        assert!(diff.lines.iter().all(|l| l.status == LineStatus::Added));
        assert!(diff.lines.iter().all(|l| l.left_number.is_none()));
        assert!(diff.lines.iter().all(|l| l.right_number.is_some()));
    }

    #[test]
    fn test_diff_removed_file() {
        let file = parse_source("x = 1\ny = 2\n").unwrap();
        let diff = diff_removed(&file);
        assert!(diff.lines.iter().all(|l| l.status == LineStatus::Removed));
        assert!(diff.lines.iter().all(|l| l.left_number.is_some()));
        assert!(diff.lines.iter().all(|l| l.right_number.is_none()));
    }

    #[test]
    fn test_moved_function_no_diff() {
        let before = parse_source(
            "def foo():\n    return 1\n\ndef bar():\n    return 2\n",
        )
        .unwrap();
        let after = parse_source(
            "def bar():\n    return 2\n\ndef foo():\n    return 1\n",
        )
        .unwrap();
        let diff = diff_files(&before, &after);
        // Moved but unchanged functions should produce no changes.
        let changed: Vec<_> = diff
            .lines
            .iter()
            .filter(|l| l.status != LineStatus::Unchanged)
            .collect();
        assert!(
            changed.is_empty(),
            "moved but unchanged functions should produce no diff, got {} changed lines",
            changed.len()
        );
    }

    #[test]
    fn test_moved_and_modified_function() {
        let before = parse_source(
            "def foo():\n    return 1\n\ndef bar():\n    return 2\n",
        )
        .unwrap();
        let after = parse_source(
            "def bar():\n    return 2\n\ndef foo():\n    return 99\n",
        )
        .unwrap();
        let diff = diff_files(&before, &after);
        // foo was moved AND modified — should show the change.
        assert!(
            diff.lines.iter().any(|l| l.status == LineStatus::Modified),
            "moved+modified function should show modification"
        );
        // bar was just moved — should show as unchanged.
        // Count: 2 lines of bar unchanged, some lines of foo changed.
        assert!(diff.stats.unchanged > 0);
    }

    #[test]
    fn test_diff_definitions_body_changed() {
        let before = parse_source(
            "def foo():\n    \"\"\"Do something.\"\"\"\n    return 1\n",
        )
        .unwrap();
        let after = parse_source(
            "def foo():\n    \"\"\"Do something.\"\"\"\n    return 2\n",
        )
        .unwrap();
        let diff = diff_definitions(&before, &after);
        assert_eq!(diff.definitions.len(), 1);
        let dd = &diff.definitions[0];
        assert_eq!(dd.name, "foo");
        assert_eq!(dd.status, DefStatus::Modified);
        assert!(!dd.signature_diff.changed, "signature should be unchanged");
        assert!(
            dd.docstring_diff.as_ref().is_some_and(|d| !d.changed),
            "docstring should be unchanged"
        );
        assert!(
            dd.body_diff.as_ref().is_some_and(|d| d.changed),
            "body should be changed"
        );
    }

    #[test]
    fn test_diff_definitions_signature_changed() {
        let before = parse_source("def foo(a: int) -> bool:\n    return True\n").unwrap();
        let after = parse_source("def foo(a: int, b: str) -> bool:\n    return True\n").unwrap();
        let diff = diff_definitions(&before, &after);
        assert_eq!(diff.definitions.len(), 1);
        let dd = &diff.definitions[0];
        assert_eq!(dd.status, DefStatus::Modified);
        assert!(dd.signature_diff.changed, "signature should be changed");
    }

    #[test]
    fn test_diff_definitions_added() {
        let before = parse_source("def foo():\n    pass\n").unwrap();
        let after =
            parse_source("def foo():\n    pass\n\ndef bar():\n    pass\n").unwrap();
        let diff = diff_definitions(&before, &after);
        assert_eq!(diff.definitions.len(), 2);
        assert_eq!(diff.definitions[0].name, "foo");
        assert_eq!(diff.definitions[0].status, DefStatus::Unchanged);
        assert_eq!(diff.definitions[1].name, "bar");
        assert_eq!(diff.definitions[1].status, DefStatus::Added);
    }

    #[test]
    fn test_diff_definitions_removed() {
        let before =
            parse_source("def foo():\n    pass\n\ndef bar():\n    pass\n").unwrap();
        let after = parse_source("def foo():\n    pass\n").unwrap();
        let diff = diff_definitions(&before, &after);
        assert_eq!(diff.definitions.len(), 2);
        let removed: Vec<_> = diff
            .definitions
            .iter()
            .filter(|d| d.status == DefStatus::Removed)
            .collect();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].name, "bar");
    }

    #[test]
    fn test_diff_definitions_preamble() {
        let before = parse_source("import os\n\ndef foo():\n    pass\n").unwrap();
        let after = parse_source("import sys\n\ndef foo():\n    pass\n").unwrap();
        let diff = diff_definitions(&before, &after);
        assert!(
            diff.preamble_diff.as_ref().is_some_and(|p| p.changed),
            "preamble should be changed"
        );
    }

    #[test]
    fn test_docstring_line_level_diff() {
        // Only the fork name in the return type changes — most docstring lines should be unchanged
        let before = parse_source(
            "\
def compute_address(address: Address, salt: Bytes32) -> Address:
    \"\"\"Compute the address.

    Parameters
    ----------
    address :
        The sender address.
    salt :
        The salt value.

    Returns
    -------
    address: `ethereum.forks.osaka.fork_types.Address`
        The computed address.
    \"\"\"
    return address
",
        )
        .unwrap();
        let after = parse_source(
            "\
def compute_address(address: Address, salt: Bytes32) -> Address:
    \"\"\"Compute the address.

    Parameters
    ----------
    address :
        The sender address.
    salt :
        The salt value.

    Returns
    -------
    address: `ethereum.forks.amsterdam.fork_types.Address`
        The computed address.
    \"\"\"
    return address
",
        )
        .unwrap();
        let diff = diff_definitions(&before, &after);
        assert_eq!(diff.definitions.len(), 1);
        let dd = &diff.definitions[0];
        assert!(
            dd.docstring_diff.as_ref().is_some_and(|d| d.changed),
            "docstring should show as changed"
        );

        let doc_diff = dd.docstring_diff.as_ref().unwrap();
        let unchanged_count = doc_diff
            .lines
            .iter()
            .filter(|l| l.status == LineStatus::Unchanged)
            .count();
        let changed_count = doc_diff
            .lines
            .iter()
            .filter(|l| l.status != LineStatus::Unchanged)
            .count();
        assert!(
            unchanged_count > changed_count,
            "most docstring lines should be unchanged, got {} unchanged vs {} changed",
            unchanged_count,
            changed_count,
        );
    }
}
