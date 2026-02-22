# eth-spec-diff

A semantic diff viewer for the [Ethereum execution specs](https://github.com/ethereum/execution-specs). Generates a static HTML site showing side-by-side diffs between consecutive fork implementations.

## What it does

Each Ethereum hard fork is implemented as a complete copy of the previous fork's Python code. This tool diffs consecutive forks and renders the changes as a browsable static site.

**Semantic diffing** means whitespace and formatting changes are ignored. The tool parses Python files with [tree-sitter](https://tree-sitter.github.io/tree-sitter/), extracts leaf tokens (skipping comments, whitespace, indent/dedent), and diffs the token sequences with [similar](https://docs.rs/similar). Two files that differ only in formatting produce an empty diff.

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

## Tests

```
cargo test
```

Tests cover token extraction, semantic diffing (whitespace invariance, move detection), and HTML rendering.
