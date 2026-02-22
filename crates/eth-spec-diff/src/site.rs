use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diff::DiffStats;

const CSS: &str = include_str!("../assets/style.css");
const JS: &str = include_str!("../assets/script.js");

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub struct FileEntry {
    pub relative_path: PathBuf,
    pub html_path: String,
    pub stats: DiffStats,
    /// Pre-rendered diff table HTML, used during generation then discarded.
    pub cached_html: Option<String>,
}

pub struct ForkPairSummary {
    pub before_name: String,
    pub after_name: String,
    pub dir_name: String,
    pub files: Vec<FileEntry>,
}

pub struct SpecFileEntry {
    pub relative_path: PathBuf,
    pub html_path: String,
    pub def_count: usize,
    /// Pre-rendered spec HTML, used during generation then discarded.
    pub cached_html: Option<String>,
}

pub struct ForkSummary {
    pub name: String,
    pub short_name: String,
    pub dir_name: String, // "spec/{short_name}"
    pub files: Vec<SpecFileEntry>,
}

// -- Page templates --

/// Redirect index.html to the first fork pair's index page.
pub fn render_index_page(pairs: &[ForkPairSummary]) -> String {
    let target = if pairs.is_empty() {
        "#".to_string()
    } else {
        format!("{}/index.html", pairs[0].dir_name)
    };
    format!(
        "<!DOCTYPE html>\n\
         <html><head>\
         <meta http-equiv=\"refresh\" content=\"0; url={target}\">\
         </head><body></body></html>\n",
        target = escape_html(&target),
    )
}

pub fn render_pair_index(
    pair: &ForkPairSummary,
    all_pairs: &[ForkPairSummary],
    all_forks: &[ForkSummary],
) -> String {
    let sidebar = render_sidebar_with_selector(pair, all_pairs, all_forks, None, "");
    let title = format!("{} → {}", pair.before_name, pair.after_name);

    let total_added: usize = pair.files.iter().map(|f| f.stats.added).sum();
    let total_removed: usize = pair.files.iter().map(|f| f.stats.removed).sum();
    let total_modified: usize = pair.files.iter().map(|f| f.stats.modified).sum();

    let body = format!(
        "<div class=\"header\">\
         <h1>{title}</h1>\
         </div>\
         <div class=\"summary\">\
         <span class=\"stat added\">+{added} added</span>\
         <span class=\"stat removed\">-{removed} removed</span>\
         <span class=\"stat modified\">~{modified} modified</span>\
         <span class=\"stat\">{files} files changed</span>\
         </div>\
         <p>Select a file from the sidebar to view its diff.</p>",
        title = escape_html(&title),
        added = total_added,
        removed = total_removed,
        modified = total_modified,
        files = pair.files.len(),
    );

    wrap_page_with_sidebar(&title, &sidebar, &body)
}

pub fn render_file_page(
    pair: &ForkPairSummary,
    all_pairs: &[ForkPairSummary],
    all_forks: &[ForkSummary],
    current_file: &Path,
    diff_table_html: &str,
) -> String {
    // Number of parent directories in the file path determines how many ../
    // we need to get back to the pair root directory.
    let parent_depth = current_file.components().count().saturating_sub(1);
    let to_pair_root = if parent_depth == 0 {
        String::new()
    } else {
        "../".repeat(parent_depth)
    };

    let sidebar = render_sidebar_with_selector(
        pair,
        all_pairs,
        all_forks,
        Some(current_file),
        &to_pair_root,
    );
    let title = format!(
        "{} — {} → {}",
        current_file.display(),
        pair.before_name,
        pair.after_name
    );

    // Cross-view link: link to the spec page for the "after" fork
    let cross_link = find_spec_for_pair(pair, all_forks, current_file, &to_pair_root);
    let cross_html = if let Some(href) = cross_link {
        format!(
            "<div class=\"cross-view-link\"><a href=\"{}\">View spec</a></div>",
            escape_html(&href),
        )
    } else {
        String::new()
    };

    let body = format!(
        "<div class=\"header\">\
         <h1>{file}</h1>\
         {cross}\
         </div>\
         {table}",
        file = escape_html(&current_file.display().to_string()),
        cross = cross_html,
        table = diff_table_html,
    );

    wrap_page_with_sidebar(&title, &sidebar, &body)
}

/// Find the spec page URL for the "after" fork of this diff pair.
fn find_spec_for_pair(
    pair: &ForkPairSummary,
    all_forks: &[ForkSummary],
    current_file: &Path,
    to_pair_root: &str,
) -> Option<String> {
    let after_fork = all_forks.iter().find(|f| f.name == pair.after_name)?;
    let file_html = format!("{}.html", current_file.display());
    let has_file = after_fork.files.iter().any(|f| f.html_path == file_html);
    if has_file {
        // From {pair_dir}/file.py.html → need to get to spec/{short_name}/file.py.html
        let to_site_root = if to_pair_root.is_empty() {
            "../".to_string()
        } else {
            format!("{to_pair_root}../")
        };
        Some(format!("{}{}/{}", to_site_root, after_fork.dir_name, file_html))
    } else {
        None
    }
}

// -- Spec view page templates --

pub fn render_spec_index(
    fork: &ForkSummary,
    all_forks: &[ForkSummary],
    all_pairs: &[ForkPairSummary],
) -> String {
    let sidebar = render_spec_sidebar(fork, all_forks, all_pairs, None, "");
    let title = format!("{} — Spec", fork.name);

    let total_defs: usize = fork.files.iter().map(|f| f.def_count).sum();

    let body = format!(
        "<div class=\"header\">\
         <h1>{name}</h1>\
         </div>\
         <div class=\"summary\">\
         <span class=\"stat\">{defs} definitions</span>\
         <span class=\"stat\">{files} files</span>\
         </div>\
         <p>Select a file from the sidebar to view its documentation.</p>",
        name = escape_html(&fork.name),
        defs = total_defs,
        files = fork.files.len(),
    );

    wrap_page_with_sidebar(&title, &sidebar, &body)
}

pub fn render_spec_file_page(
    fork: &ForkSummary,
    all_forks: &[ForkSummary],
    all_pairs: &[ForkPairSummary],
    current_file: &Path,
    body_html: &str,
) -> String {
    let parent_depth = current_file.components().count().saturating_sub(1);
    let to_fork_root = if parent_depth == 0 {
        String::new()
    } else {
        "../".repeat(parent_depth)
    };

    let sidebar = render_spec_sidebar(
        fork,
        all_forks,
        all_pairs,
        Some(current_file),
        &to_fork_root,
    );
    let title = format!("{} — {}", current_file.display(), fork.name);

    // Cross-view link: find diff pair where this fork is the "after" side
    let cross_link = find_diff_pair_for_fork(fork, all_pairs, current_file, &to_fork_root);
    let cross_html = if let Some(href) = cross_link {
        format!(
            "<div class=\"cross-view-link\"><a href=\"{}\">View diff</a></div>",
            escape_html(&href),
        )
    } else {
        String::new()
    };

    let body = format!(
        "<div class=\"header\">\
         <h1>{file}</h1>\
         {cross}\
         </div>\
         {table}",
        file = escape_html(&current_file.display().to_string()),
        cross = cross_html,
        table = body_html,
    );

    wrap_page_with_sidebar(&title, &sidebar, &body)
}

/// Find the diff pair URL where this fork is the "after" side.
fn find_diff_pair_for_fork(
    fork: &ForkSummary,
    all_pairs: &[ForkPairSummary],
    current_file: &Path,
    to_fork_root: &str,
) -> Option<String> {
    for pair in all_pairs {
        if pair.after_name == fork.name {
            let file_html = format!("{}.html", current_file.display());
            // Check if this file exists in the pair
            let has_file = pair.files.iter().any(|f| f.html_path == file_html);
            if has_file {
                // From spec/{short_name}/file.py.html → need to get to pair_dir/file.py.html
                // We're in spec/{short_name}/ (with to_fork_root getting us to spec/{short_name}/)
                // We need ../../{pair_dir}/{file}.html
                let to_site_root = format!("{to_fork_root}../../");
                return Some(format!(
                    "{}{}/{}",
                    to_site_root, pair.dir_name, file_html,
                ));
            }
        }
    }
    None
}

fn render_spec_sidebar(
    current_fork: &ForkSummary,
    all_forks: &[ForkSummary],
    all_pairs: &[ForkPairSummary],
    active_file: Option<&Path>,
    link_prefix: &str,
) -> String {
    let to_site_root = if link_prefix.is_empty() {
        "../../".to_string()
    } else {
        format!("{link_prefix}../../")
    };

    // View toggle: Diff / Spec
    let first_pair_url = if all_pairs.is_empty() {
        "#".to_string()
    } else {
        format!("{}{}/index.html", to_site_root, all_pairs[0].dir_name)
    };
    let view_toggle = format!(
        "<div class=\"view-toggle\">\
         <a href=\"{diff_url}\">Diff</a>\
         <a class=\"active\" href=\"#\">Spec</a>\
         </div>\n",
        diff_url = escape_html(&first_pair_url),
    );

    // Fork selector
    let mut options = String::new();
    for fork in all_forks {
        let url = format!("{}{}/index.html", to_site_root, fork.dir_name);
        let selected = if fork.short_name == current_fork.short_name {
            " selected"
        } else {
            ""
        };
        options.push_str(&format!(
            "<option value=\"{url}\"{selected}>{label}</option>\n",
            url = escape_html(&url),
            selected = selected,
            label = escape_html(&fork.name),
        ));
    }
    let selector = format!(
        "<div class=\"fork-selector\">\
         <label for=\"fork-select\">Fork</label>\
         <select id=\"fork-select\" onchange=\"window.location.href=this.value\">\
         {options}\
         </select>\
         </div>\n",
    );

    let file_tree = render_spec_file_tree(&current_fork.files, active_file, link_prefix);

    format!("{view_toggle}{selector}{file_tree}")
}

fn render_spec_file_tree(
    files: &[SpecFileEntry],
    active_file: Option<&Path>,
    link_prefix: &str,
) -> String {
    let mut root = DirNode::new();
    for entry in files {
        let components: Vec<&str> = entry
            .relative_path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        let mut node = &mut root;
        for (i, comp) in components.iter().enumerate() {
            if i == components.len() - 1 {
                let stats = DiffStats {
                    added: entry.def_count,
                    removed: 0,
                    modified: 0,
                    unchanged: 0,
                };
                node.files.push((
                    comp.to_string(),
                    entry.html_path.clone(),
                    stats,
                ));
            } else {
                node = node
                    .children_dirs
                    .entry(comp.to_string())
                    .or_insert_with(DirNode::new);
            }
        }
    }

    let active_href = active_file.map(|p| {
        let mut s = p.display().to_string();
        s.push_str(".html");
        s
    });

    // Render with def counts instead of +/- stats
    let inner = render_spec_dir_node(&root, active_href.as_deref(), link_prefix);
    format!("<h2>Files</h2>\n<ul class=\"file-tree\">{inner}</ul>\n")
}

fn render_spec_dir_node(node: &DirNode, active_href: Option<&str>, base_href: &str) -> String {
    let mut html = String::new();

    for (dir_name, child) in &node.children_dirs {
        html.push_str(&format!(
            "<li><span class=\"dir-toggle open\">{name}</span>\
             <ul class=\"dir-children\">{children}</ul></li>\n",
            name = escape_html(dir_name),
            children = render_spec_dir_node(child, active_href, base_href),
        ));
    }

    for (filename, href, stats) in &node.files {
        let is_active = active_href.is_some_and(|a| a == href);
        let active_class = if is_active { " active" } else { "" };
        let count_html = if stats.added > 0 {
            format!("<span class=\"stats\">{} defs</span>", stats.added)
        } else {
            String::new()
        };
        html.push_str(&format!(
            "<li><a class=\"file{active}\" href=\"{base}{href}\">\
             <span>{name}</span>{count}</a></li>\n",
            active = active_class,
            base = base_href,
            href = escape_html(href),
            name = escape_html(filename),
            count = count_html,
        ));
    }

    html
}

// -- Sidebar with fork pair selector (diff view) --

fn render_sidebar_with_selector(
    current_pair: &ForkPairSummary,
    all_pairs: &[ForkPairSummary],
    all_forks: &[ForkSummary],
    active_file: Option<&Path>,
    link_prefix: &str,
) -> String {
    // Build the fork pair <select> dropdown.
    // The value is a relative URL from the current page to the target pair's index.
    let to_site_root = if link_prefix.is_empty() {
        "../".to_string()
    } else {
        format!("{link_prefix}../")
    };

    // View toggle: Diff / Spec
    let first_spec_url = if all_forks.is_empty() {
        "#".to_string()
    } else {
        format!("{}{}/index.html", to_site_root, all_forks[0].dir_name)
    };
    let view_toggle = format!(
        "<div class=\"view-toggle\">\
         <a class=\"active\" href=\"#\">Diff</a>\
         <a href=\"{spec_url}\">Spec</a>\
         </div>\n",
        spec_url = escape_html(&first_spec_url),
    );

    let mut options = String::new();
    for pair in all_pairs {
        let label = format!("{} → {}", pair.before_name, pair.after_name);
        let url = format!("{}{}/index.html", to_site_root, pair.dir_name);
        let selected = if pair.dir_name == current_pair.dir_name {
            " selected"
        } else {
            ""
        };
        options.push_str(&format!(
            "<option value=\"{url}\"{selected}>{label}</option>\n",
            url = escape_html(&url),
            selected = selected,
            label = escape_html(&label),
        ));
    }

    let selector = format!(
        "<div class=\"fork-selector\">\
         <label for=\"fork-pair\">Fork pair</label>\
         <select id=\"fork-pair\" onchange=\"window.location.href=this.value\">\
         {options}\
         </select>\
         </div>\n"
    );

    let file_tree = render_file_tree(&current_pair.files, active_file, link_prefix);

    format!("{view_toggle}{selector}{file_tree}")
}

// -- File tree --

struct DirNode {
    children_dirs: BTreeMap<String, DirNode>,
    files: Vec<(String, String, DiffStats)>, // (filename, href, stats)
}

impl DirNode {
    fn new() -> Self {
        Self {
            children_dirs: BTreeMap::new(),
            files: Vec::new(),
        }
    }
}

fn build_dir_tree(files: &[FileEntry], active_file: Option<&Path>) -> DirNode {
    let _ = active_file; // used below in render
    let mut root = DirNode::new();
    for entry in files {
        let components: Vec<&str> = entry
            .relative_path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        let mut node = &mut root;
        for (i, comp) in components.iter().enumerate() {
            if i == components.len() - 1 {
                node.files.push((
                    comp.to_string(),
                    entry.html_path.clone(),
                    entry.stats.clone(),
                ));
            } else {
                node = node
                    .children_dirs
                    .entry(comp.to_string())
                    .or_insert_with(DirNode::new);
            }
        }
    }
    root
}

fn render_dir_node(node: &DirNode, active_href: Option<&str>, base_href: &str) -> String {
    let mut html = String::new();

    for (dir_name, child) in &node.children_dirs {
        html.push_str(&format!(
            "<li><span class=\"dir-toggle open\">{name}</span>\
             <ul class=\"dir-children\">{children}</ul></li>\n",
            name = escape_html(dir_name),
            children = render_dir_node(child, active_href, base_href),
        ));
    }

    for (filename, href, stats) in &node.files {
        let is_active = active_href.is_some_and(|a| a == href);
        let active_class = if is_active { " active" } else { "" };

        let stats_html = format_stats(stats);
        html.push_str(&format!(
            "<li><a class=\"file{active}\" href=\"{base}{href}\">\
             <span>{name}</span><span class=\"stats\">{stats}</span></a></li>\n",
            active = active_class,
            base = base_href,
            href = escape_html(href),
            name = escape_html(filename),
            stats = stats_html,
        ));
    }

    html
}

fn format_stats(stats: &DiffStats) -> String {
    let mut parts = Vec::new();
    let total_add = stats.added + stats.modified;
    let total_del = stats.removed + stats.modified;
    if total_add > 0 {
        parts.push(format!("<span class=\"add-stat\">+{total_add}</span>"));
    }
    if total_del > 0 {
        parts.push(format!("<span class=\"del-stat\">-{total_del}</span>"));
    }
    parts.join(" ")
}

fn render_file_tree(files: &[FileEntry], active_file: Option<&Path>, link_prefix: &str) -> String {
    let tree = build_dir_tree(files, active_file);
    let active_href = active_file.map(|p| {
        let mut s = p.display().to_string();
        s.push_str(".html");
        s
    });
    let inner = render_dir_node(&tree, active_href.as_deref(), link_prefix);
    format!(
        "<h2>Files</h2>\n<ul class=\"file-tree\">{inner}</ul>\n"
    )
}

// -- HTML wrappers --

fn wrap_page(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n\
         <style>{css}</style>\n\
         </head>\n\
         <body>\n\
         {body}\n\
         <script>{js}</script>\n\
         </body>\n\
         </html>\n",
        title = escape_html(title),
        css = CSS,
        body = body,
        js = JS,
    )
}

fn wrap_page_with_sidebar(title: &str, sidebar_html: &str, main_html: &str) -> String {
    let body = format!(
        "<div class=\"sidebar\">{sidebar}</div>\n\
         <div class=\"main\">{main}</div>",
        sidebar = sidebar_html,
        main = main_html,
    );
    wrap_page(title, &body)
}
