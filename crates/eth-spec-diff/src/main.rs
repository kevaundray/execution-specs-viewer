mod config;
mod diff;
mod discover;
mod parse;
mod render;
mod site;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use crate::config::Config;
use crate::discover::{DiffPair, ForkFiles, SourcePair};
use crate::site::{FileEntry, ForkPairSummary, ForkSummary, SpecFileEntry};

#[derive(Parser)]
#[command(name = "eth-spec-diff", about = "Semantic diff viewer for Ethereum execution specs")]
struct Cli {
    #[arg(short, long)]
    config: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Phase 1: Load config.
    let config = Config::load(&cli.config)?;
    eprintln!("Loaded config with {} forks", config.forks.len());

    // Phase 2: Discover file pairs.
    let pairs = discover::discover_pairs(&config)?;
    eprintln!("Discovered {} fork pairs", pairs.len());

    // Phase 3-5: Process each pair, render HTML, write output.
    let mut pair_summaries = Vec::new();

    for pair in &pairs {
        let summary = process_pair_data(pair, config.source_url.as_deref())?;
        pair_summaries.push(summary);
    }

    // Phase: Generate spec pages
    let fork_files = discover::discover_forks(&config)?;
    eprintln!("Discovered {} forks for spec view", fork_files.len());

    let mut fork_summaries = Vec::new();
    for fork_data in &fork_files {
        let summary = process_fork_spec(fork_data, config.source_url.as_deref())?;
        fork_summaries.push(summary);
    }

    // Write all pages now that we have all pair and fork summaries.
    fs::create_dir_all(&config.output)
        .with_context(|| format!("creating output dir {}", config.output.display()))?;

    // Write redirect index.
    let index_html = site::render_index_page(&pair_summaries);
    let index_path = config.output.join("index.html");
    fs::write(&index_path, index_html)
        .with_context(|| format!("writing {}", index_path.display()))?;

    // Write pair index and file pages.
    for summary in &pair_summaries {
        write_pair_pages(summary, &pair_summaries, &fork_summaries, &config.output)?;
    }

    // Write spec pages.
    for summary in &fork_summaries {
        write_spec_pages(summary, &fork_summaries, &pair_summaries, &config.output)?;
    }

    eprintln!("Output written to {}", config.output.display());
    Ok(())
}

/// Build a file-level GitHub URL (without the #L fragment).
fn build_file_url(source_url: &str, fork_original_path: &std::path::Path, relative_path: &std::path::Path) -> String {
    format!(
        "{}/{}/{}",
        source_url.trim_end_matches('/'),
        fork_original_path.display(),
        relative_path.display(),
    )
}

/// Process a fork pair: parse files, compute diffs, collect data (no writing yet).
fn process_pair_data(pair: &DiffPair, source_url: Option<&str>) -> Result<ForkPairSummary> {
    let dir_name = format!(
        "{}_to_{}",
        pair.before_fork.short_name, pair.after_fork.short_name
    );

    eprintln!(
        "  Processing {} → {} ({} files)",
        pair.before_fork.name,
        pair.after_fork.name,
        pair.sources.len()
    );

    let mut file_entries = Vec::new();
    for source in &pair.sources {
        let before_file_url = source_url.map(|url| {
            build_file_url(url, &pair.before_fork.original_path, &source.relative_path)
        });
        let after_file_url = source_url.map(|url| {
            build_file_url(url, &pair.after_fork.original_path, &source.relative_path)
        });
        let entry = process_source_pair(
            source,
            before_file_url.as_deref(),
            after_file_url.as_deref(),
        )?;
        file_entries.push(entry);
    }

    Ok(ForkPairSummary {
        before_name: pair.before_fork.name.clone(),
        after_name: pair.after_fork.name.clone(),
        dir_name,
        files: file_entries,
    })
}

/// Write all HTML pages for a single fork pair.
fn write_pair_pages(
    summary: &ForkPairSummary,
    all_pairs: &[ForkPairSummary],
    all_forks: &[ForkSummary],
    output_root: &PathBuf,
) -> Result<()> {
    let pair_dir = output_root.join(&summary.dir_name);
    fs::create_dir_all(&pair_dir)
        .with_context(|| format!("creating pair dir {}", pair_dir.display()))?;

    // Write pair index page.
    let pair_index_html = site::render_pair_index(summary, all_pairs, all_forks);
    let pair_index_path = pair_dir.join("index.html");
    fs::write(&pair_index_path, pair_index_html)
        .with_context(|| format!("writing {}", pair_index_path.display()))?;

    // Write individual file diff pages.
    for file_entry in &summary.files {
        let file_html = site::render_file_page(
            summary,
            all_pairs,
            all_forks,
            &file_entry.relative_path,
            file_entry.cached_html.as_deref().unwrap_or(""),
        );
        let file_path = pair_dir.join(&file_entry.html_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        fs::write(&file_path, file_html)
            .with_context(|| format!("writing {}", file_path.display()))?;
    }

    Ok(())
}

fn process_source_pair(
    source: &SourcePair,
    before_file_url: Option<&str>,
    after_file_url: Option<&str>,
) -> Result<FileEntry> {
    let (before_parsed, after_parsed) = match (&source.before_path, &source.after_path) {
        (Some(before_path), Some(after_path)) => {
            let before = parse::parse_file(before_path)?;
            let after = parse::parse_file(after_path)?;
            (Some(before), Some(after))
        }
        (None, Some(after_path)) => {
            let after = parse::parse_file(after_path)?;
            (None, Some(after))
        }
        (Some(before_path), None) => {
            let before = parse::parse_file(before_path)?;
            (Some(before), None)
        }
        (None, None) => {
            anyhow::bail!("source pair has no paths");
        }
    };

    // Choose rendering path: definition cards if both files have definitions,
    // otherwise fall back to raw diff table.
    let (table_html, stats) = match (&before_parsed, &after_parsed) {
        (Some(b), Some(a)) => {
            let has_defs = !b.definitions.is_empty() || !a.definitions.is_empty();
            if has_defs {
                let def_diff = diff::diff_definitions(b, a);
                let html = render::render_definition_cards(
                    &def_diff, b, a, before_file_url, after_file_url,
                );
                let stats = def_diff.stats;
                (html, stats)
            } else {
                let diff_result = diff::diff_files(b, a);
                let html = render::render_diff_table(&diff_result, b, a);
                let stats = diff_result.stats;
                (html, stats)
            }
        }
        (None, Some(a)) => {
            let diff_result = diff::diff_added(a);
            let html = render::render_added_table(&diff_result, a);
            let stats = diff_result.stats;
            (html, stats)
        }
        (Some(b), None) => {
            let diff_result = diff::diff_removed(b);
            let html = render::render_removed_table(&diff_result, b);
            let stats = diff_result.stats;
            (html, stats)
        }
        (None, None) => unreachable!(),
    };

    let html_path = format!("{}.html", source.relative_path.display());

    Ok(FileEntry {
        relative_path: source.relative_path.clone(),
        html_path,
        stats,
        cached_html: Some(table_html),
    })
}

/// Process a single fork for spec view: parse files and render spec HTML.
fn process_fork_spec(fork_data: &ForkFiles, source_url: Option<&str>) -> Result<ForkSummary> {
    let dir_name = format!("spec/{}", fork_data.fork.short_name);

    eprintln!(
        "  Processing spec for {} ({} files)",
        fork_data.fork.name,
        fork_data.files.len()
    );

    let mut file_entries = Vec::new();
    for source in &fork_data.files {
        let file_url = source_url.map(|url| {
            build_file_url(url, &fork_data.fork.original_path, &source.relative_path)
        });
        let parsed = parse::parse_file(&source.absolute_path)?;
        let spec_html = render::render_spec_file(&parsed, file_url.as_deref());
        let html_path = format!("{}.html", source.relative_path.display());

        file_entries.push(SpecFileEntry {
            relative_path: source.relative_path.clone(),
            html_path,
            def_count: parsed.definitions.len(),
            cached_html: Some(spec_html),
        });
    }

    Ok(ForkSummary {
        name: fork_data.fork.name.clone(),
        short_name: fork_data.fork.short_name.clone(),
        dir_name,
        files: file_entries,
    })
}

/// Write all spec pages for a single fork.
fn write_spec_pages(
    summary: &ForkSummary,
    all_forks: &[ForkSummary],
    all_pairs: &[ForkPairSummary],
    output_root: &PathBuf,
) -> Result<()> {
    let fork_dir = output_root.join(&summary.dir_name);
    fs::create_dir_all(&fork_dir)
        .with_context(|| format!("creating spec dir {}", fork_dir.display()))?;

    // Write fork index page.
    let index_html = site::render_spec_index(summary, all_forks, all_pairs);
    let index_path = fork_dir.join("index.html");
    fs::write(&index_path, index_html)
        .with_context(|| format!("writing {}", index_path.display()))?;

    // Write individual spec file pages.
    for file_entry in &summary.files {
        let file_html = site::render_spec_file_page(
            summary,
            all_forks,
            all_pairs,
            &file_entry.relative_path,
            file_entry.cached_html.as_deref().unwrap_or(""),
        );
        let file_path = fork_dir.join(&file_entry.html_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        fs::write(&file_path, file_html)
            .with_context(|| format!("writing {}", file_path.display()))?;
    }

    Ok(())
}
