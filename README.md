# eth-spec-diff

A semantic diff viewer for the [Ethereum execution specs](https://github.com/ethereum/execution-specs). Generates a static HTML site showing side-by-side diffs between consecutive fork implementations.

## What it does

Each Ethereum hard fork is implemented as a complete copy of the previous fork's Python code. This tool diffs consecutive forks and renders the changes as a browsable static site.

**Semantic diffing** means whitespace and formatting changes are ignored. The tool parses Python files with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), extracts leaf tokens (skipping comments, whitespace, indent/dedent), and diffs the token sequences with [similar](https://docs.rs/similar). Two files that differ only in formatting produce an empty diff.

**Move detection** means functions or classes that moved within a file but didn't change won't show up as diffs. Top-level definitions are matched by name before diffing their contents.

## Usage

```
cargo run -- --config config.toml
```

Then open `<output>/index.html` in a browser, or serve it:

```
python3 -m http.server 8000 --directory <output>
```

## Config file

```toml
spec_root = "execution-specs/src/ethereum"
output = "diff-output"

[[forks]]
name = "Frontier"
short_name = "frontier"
path = "forks/frontier"

[[forks]]
name = "Homestead"
short_name = "homestead"
path = "forks/homestead"
```

- **spec_root** -- base directory for resolving paths (relative to the config file)
- **output** -- where to write the generated site (relative to spec_root)
- **forks** -- ordered list of forks; consecutive pairs are diffed

Fork `path` values are relative to `spec_root`. The tool discovers all `*.py` files in each fork directory and pairs them across consecutive forks.

## How it works

The pipeline has five phases:

1. **Config** -- load the TOML file, resolve paths
2. **Discover** -- walk fork directories for `*.py` files, pair consecutive forks, union their file sets
3. **Parse** -- parse each Python file with tree-sitter-python, extract leaf tokens and top-level block structure
4. **Diff** -- match top-level definitions (functions, classes) by name across before/after files, then diff token sequences within each matched block using Myers algorithm
5. **Render** -- generate HTML pages with side-by-side diff tables, a file tree sidebar, and a fork pair selector dropdown

### Token extraction

Tree-sitter produces a concrete syntax tree. The tool walks it depth-first and emits leaf nodes as tokens, skipping:
- comments
- newlines, indentation markers (INDENT/DEDENT)

Tokens compare equal by `(kind, text)`, ignoring position. This is what makes the diff semantic.

### Block matching

Before diffing tokens, the tool extracts top-level blocks from each file:
- **Named blocks**: function and class definitions (including decorated ones), matched by name
- **Unnamed blocks**: consecutive imports, assignments, etc., matched by position

This prevents moved-but-unchanged definitions from appearing as changes.

### Syntax highlighting

Token kinds map to CSS classes for syntax coloring:
- `def`, `class`, `return`, etc. -> `.kw` (keyword)
- `identifier` -> `.ident`
- `string` -> `.str`
- `integer`, `float` -> `.num`
- operators -> `.op`
- `@` -> `.decorator`

Changed tokens within modified lines get `.tok-add` / `.tok-del` classes for inline highlighting.

## Output structure

```
<output>/
  index.html                              # redirects to first fork pair
  frontier_to_homestead/
    index.html                            # file tree + summary + fork pair dropdown
    fork.py.html                          # individual file diff
    vm/gas.py.html
    ...
  homestead_to_dao_fork/
    ...
```

CSS and JS are embedded in each page via `include_str!` so the output is fully self-contained.

## Module overview

| Module | Purpose |
|--------|---------|
| `main.rs` | CLI, orchestrates the 5-phase pipeline |
| `config.rs` | TOML config loading and path resolution |
| `discover.rs` | Walks fork directories, pairs consecutive forks |
| `parse.rs` | tree-sitter parsing, token extraction, block extraction |
| `diff.rs` | Token-sequence diffing with block matching, line-level mapping |
| `render.rs` | HTML diff table rendering with syntax highlighting |
| `site.rs` | Page templates, file tree, fork pair dropdown, CSS/JS embedding |

## Tests

```
cargo test
```

Tests cover token extraction, semantic diffing (whitespace invariance, move detection), and HTML rendering.
