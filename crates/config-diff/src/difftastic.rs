use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// --- Format prefix constants for dashboard detection ---

const FORMAT_PREFIX_UNIFIED: &str = "@@format:unified@@\n";
const FORMAT_PREFIX_CONTEXT: &str = "@@format:context@@\n";
const FORMAT_PREFIX_FULL_FILE: &str = "@@format:full_file@@\n";
const FORMAT_PREFIX_SIDE_BY_SIDE: &str = "@@format:side_by_side@@\n";
const FORMAT_PREFIX_RAW: &str = "@@format:raw@@\n";

// --- Configuration types ---

/// Diff output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DiffFormat {
    #[default]
    Unified,
    Context,
    FullFile,
    SideBySide,
    Raw,
}

/// Configuration for diff rendering.
#[derive(Debug, Clone, Deserialize)]
pub struct DiffConfig {
    #[serde(default)]
    pub format: DiffFormat,
    #[serde(default = "default_context_lines")]
    pub context_lines: usize,
    #[serde(default = "default_side_by_side_width")]
    pub side_by_side_width: usize,
}

fn default_context_lines() -> usize {
    3
}

fn default_side_by_side_width() -> usize {
    120
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            format: DiffFormat::Unified,
            context_lines: default_context_lines(),
            side_by_side_width: default_side_by_side_width(),
        }
    }
}

// --- Output types ---

#[derive(Debug, Clone)]
pub enum DiffOutput {
    Unchanged,
    Changed {
        render: String,
        added: u64,
        removed: u64,
    },
    Error {
        message: String,
    },
}

// --- DiffEngine ---

#[derive(Debug, Clone)]
pub struct DiffEngine {
    difft_path: PathBuf,
    difft_available: bool,
    config: DiffConfig,
}

impl Default for DiffEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffEngine {
    pub fn new() -> Self {
        let (path, available) = find_difft_binary();
        if !available {
            tracing::warn!("difftastic not found; falling back to line diff. Install with: cargo install difftastic");
        }
        Self {
            difft_path: path,
            difft_available: available,
            config: DiffConfig::default(),
        }
    }

    pub fn with_config(config: DiffConfig) -> Self {
        let (path, available) = find_difft_binary();
        if !available {
            tracing::warn!("difftastic not found; falling back to line diff. Install with: cargo install difftastic");
        }
        Self {
            difft_path: path,
            difft_available: available,
            config,
        }
    }

    pub fn with_path(path: &str) -> Self {
        let available = probe_difft(path);
        Self {
            difft_path: PathBuf::from(path),
            difft_available: available,
            config: DiffConfig::default(),
        }
    }

    pub fn is_difftastic_available(&self) -> bool {
        self.difft_available
    }

    pub async fn compute_diff(
        &self,
        previous: &str,
        current: &str,
        file_path: &Utf8Path,
    ) -> Result<DiffOutput> {
        if !self.difft_available {
            return Ok(self.line_diff(previous, current));
        }

        match self.config.format {
            DiffFormat::Raw => self.run_difftastic_raw(previous, current, file_path).await,
            _ => {
                // All other formats use JSON mode + custom rendering
                let json_output = self
                    .run_difftastic_json(previous, current, file_path)
                    .await?;

                let prev_lines: Vec<&str> = previous.lines().collect();
                let curr_lines: Vec<&str> = current.lines().collect();

                let parsed: Option<DifftasticJsonOutput> = serde_json::from_str(&json_output).ok();

                match parsed {
                    Some(output) => match self.config.format {
                        DiffFormat::Unified => {
                            render_unified(&output, &prev_lines, &curr_lines, file_path)
                        }
                        DiffFormat::Context => render_context(
                            &output,
                            &prev_lines,
                            &curr_lines,
                            file_path,
                            self.config.context_lines,
                        ),
                        DiffFormat::FullFile => {
                            render_full_file(&output, &prev_lines, &curr_lines, file_path)
                        }
                        DiffFormat::SideBySide => render_side_by_side(
                            &output,
                            &prev_lines,
                            &curr_lines,
                            file_path,
                            self.config.side_by_side_width,
                        ),
                        DiffFormat::Raw => unreachable!(),
                    },
                    None => Ok(self.line_diff(previous, current)),
                }
            }
        }
    }

    pub async fn check_ast_equivalent(&self, previous: &str, current: &str) -> Result<bool> {
        if !self.difft_available {
            return Ok(previous.trim() == current.trim());
        }

        let dir = tempfile::tempdir().context("create temp dir for ast check")?;
        let prev_path = dir.path().join("previous.yaml");
        let curr_path = dir.path().join("current.yaml");
        tokio::fs::write(&prev_path, previous).await?;
        tokio::fs::write(&curr_path, current).await?;

        let output = tokio::process::Command::new(&self.difft_path)
            .arg("--check-only")
            .arg(&prev_path)
            .arg(&curr_path)
            .output()
            .await
            .context("failed to run difftastic --check-only")?;

        Ok(output.status.success())
    }

    /// Run difftastic in JSON mode and return the raw JSON stdout string.
    async fn run_difftastic_json(
        &self,
        previous: &str,
        current: &str,
        file_path: &Utf8Path,
    ) -> Result<String> {
        let dir = tempfile::tempdir().context("create temp dir for diff")?;
        let ext = file_path.extension().unwrap_or("yaml");
        let prev_path = dir.path().join(format!("previous.{}", ext));
        let curr_path = dir.path().join(format!("current.{}", ext));
        tokio::fs::write(&prev_path, previous).await?;
        tokio::fs::write(&curr_path, current).await?;

        let output = tokio::process::Command::new(&self.difft_path)
            .arg("--display")
            .arg("json")
            .arg("--color")
            .arg("never")
            .arg(&prev_path)
            .arg(&curr_path)
            .env("DFT_UNSTABLE", "yes")
            .output()
            .await
            .context("failed to run difftastic")?;

        let code = output.status.code().unwrap_or(-1);
        if code > 1 {
            anyhow::bail!(
                "difftastic exited with code {}: {}",
                code,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Run difftastic in raw mode, capturing its own display output.
    async fn run_difftastic_raw(
        &self,
        previous: &str,
        current: &str,
        file_path: &Utf8Path,
    ) -> Result<DiffOutput> {
        if previous == current {
            return Ok(DiffOutput::Unchanged);
        }

        let dir = tempfile::tempdir().context("create temp dir for diff")?;
        let ext = file_path.extension().unwrap_or("yaml");
        let prev_path = dir.path().join(format!("previous.{}", ext));
        let curr_path = dir.path().join(format!("current.{}", ext));
        tokio::fs::write(&prev_path, previous).await?;
        tokio::fs::write(&curr_path, current).await?;

        let output = tokio::process::Command::new(&self.difft_path)
            .arg("--color")
            .arg("never")
            .arg(&prev_path)
            .arg(&curr_path)
            .output()
            .await
            .context("failed to run difftastic")?;

        let code = output.status.code().unwrap_or(-1);
        if code > 1 {
            return Ok(DiffOutput::Error {
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() && code == 0 {
            return Ok(DiffOutput::Unchanged);
        }

        // Count added/removed heuristically from raw output
        let mut added = 0u64;
        let mut removed = 0u64;
        for line in stdout.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('+') && !trimmed.starts_with("++") {
                added += 1;
            } else if trimmed.starts_with('-') && !trimmed.starts_with("--") {
                removed += 1;
            }
        }

        // Replace temp paths with the real file path
        let render = stdout
            .replace(prev_path.to_str().unwrap_or(""), file_path.as_str())
            .replace(curr_path.to_str().unwrap_or(""), file_path.as_str());

        if render.trim().is_empty() {
            Ok(DiffOutput::Unchanged)
        } else {
            let render = format!("{}{}", FORMAT_PREFIX_RAW, render);
            Ok(DiffOutput::Changed {
                render,
                added,
                removed,
            })
        }
    }

    fn line_diff(&self, previous: &str, current: &str) -> DiffOutput {
        if previous == current {
            return DiffOutput::Unchanged;
        }

        let prev_lines: Vec<&str> = previous.lines().collect();
        let curr_lines: Vec<&str> = current.lines().collect();

        let mut added = 0u64;
        let mut removed = 0u64;
        let mut render = String::new();

        render.push_str(FORMAT_PREFIX_UNIFIED);

        for (i, line) in curr_lines.iter().enumerate() {
            if i >= prev_lines.len() || prev_lines[i] != *line {
                render.push_str(&format!("{:>4} | +{}\n", i + 1, line));
                added += 1;
            }
        }
        for (i, line) in prev_lines.iter().enumerate() {
            if i >= curr_lines.len() || curr_lines[i] != *line {
                render.push_str(&format!("{:>4} | -{}\n", i + 1, line));
                removed += 1;
            }
        }

        if render.trim().is_empty() {
            DiffOutput::Unchanged
        } else {
            DiffOutput::Changed {
                render,
                added,
                removed,
            }
        }
    }
}

// --- Difftastic JSON types ---

#[derive(Debug, Deserialize)]
struct DifftasticJsonOutput {
    status: String,
    chunks: Vec<Vec<DifftasticChunk>>,
    #[serde(default)]
    #[allow(dead_code)]
    language: String,
}

#[derive(Debug, Deserialize)]
struct DifftasticChunk {
    lhs: Option<DifftasticSide>,
    rhs: Option<DifftasticSide>,
}

#[derive(Debug, Deserialize)]
struct DifftasticSide {
    line_number: Option<usize>,
    #[allow(dead_code)]
    changes: Vec<DifftasticChange>,
}

#[derive(Debug, Deserialize)]
struct DifftasticChange {
    #[allow(dead_code)]
    content: String,
    #[allow(dead_code)]
    highlight: String,
}

// --- Line change tracking (shared across renderers) ---

/// Tracks which lines are changed and their type (added, removed, modified).
struct LineChange {
    prev_line: Option<usize>,
    curr_line: Option<usize>,
}

fn collect_line_changes(chunks: &[Vec<DifftasticChunk>]) -> Vec<LineChange> {
    let mut changes = Vec::new();
    for group in chunks {
        for chunk in group {
            match (&chunk.lhs, &chunk.rhs) {
                (Some(lhs), Some(rhs)) => {
                    changes.push(LineChange {
                        prev_line: lhs.line_number,
                        curr_line: rhs.line_number,
                    });
                }
                (None, Some(rhs)) => {
                    changes.push(LineChange {
                        prev_line: None,
                        curr_line: rhs.line_number,
                    });
                }
                (Some(lhs), None) => {
                    changes.push(LineChange {
                        prev_line: lhs.line_number,
                        curr_line: None,
                    });
                }
                (None, None) => {}
            }
        }
    }
    changes
}

// --- Render functions ---

/// Unified diff: only changed lines with `+`/`-` markers (current default).
fn render_unified(
    output: &DifftasticJsonOutput,
    prev_lines: &[&str],
    curr_lines: &[&str],
    file_path: &Utf8Path,
) -> Result<DiffOutput> {
    if output.status == "unchanged" {
        return Ok(DiffOutput::Unchanged);
    }
    if output.status != "changed" {
        return Ok(DiffOutput::Error {
            message: format!("unexpected difftastic status: {}", output.status),
        });
    }

    let mut added = 0u64;
    let mut removed = 0u64;
    let mut render = String::new();

    render.push_str(FORMAT_PREFIX_UNIFIED);
    render.push_str(&format!("--- {}\n", file_path));
    render.push_str(&format!("+++ {}\n", file_path));

    for change in collect_line_changes(&output.chunks) {
        if let Some(ln) = change.prev_line {
            if ln < prev_lines.len() {
                render.push_str(&format!("{:>4} | -{}\n", ln + 1, prev_lines[ln]));
                removed += 1;
            }
        }
        if let Some(ln) = change.curr_line {
            if ln < curr_lines.len() {
                render.push_str(&format!("{:>4} | +{}\n", ln + 1, curr_lines[ln]));
                added += 1;
            }
        }
    }

    if render.trim().is_empty() {
        Ok(DiffOutput::Unchanged)
    } else {
        Ok(DiffOutput::Changed {
            render,
            added,
            removed,
        })
    }
}

/// Context diff: changed lines with N surrounding unchanged lines and hunk headers.
fn render_context(
    output: &DifftasticJsonOutput,
    prev_lines: &[&str],
    curr_lines: &[&str],
    file_path: &Utf8Path,
    context_lines: usize,
) -> Result<DiffOutput> {
    if output.status == "unchanged" {
        return Ok(DiffOutput::Unchanged);
    }
    if output.status != "changed" {
        return Ok(DiffOutput::Error {
            message: format!("unexpected difftastic status: {}", output.status),
        });
    }

    let changes = collect_line_changes(&output.chunks);
    if changes.is_empty() {
        return Ok(DiffOutput::Unchanged);
    }

    // Build unified diff entries using two-pointer walk through both files
    enum Entry {
        Context { prev_ln: usize, curr_ln: usize },
        Removed { prev_ln: usize },
        Added { curr_ln: usize },
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut prev_idx = 0usize;
    let mut curr_idx = 0usize;

    for change in &changes {
        let prev_target = change.prev_line.unwrap_or(prev_lines.len());
        let curr_target = change.curr_line.unwrap_or(curr_lines.len());

        // Emit context lines up to this change
        while prev_idx < prev_target && curr_idx < curr_target {
            if prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
                entries.push(Entry::Context {
                    prev_ln: prev_idx,
                    curr_ln: curr_idx,
                });
            }
            prev_idx += 1;
            curr_idx += 1;
        }

        // Emit removal
        if let Some(pl) = change.prev_line {
            if pl < prev_lines.len() {
                entries.push(Entry::Removed { prev_ln: pl });
                if prev_idx <= pl {
                    prev_idx = pl + 1;
                }
            }
        }

        // Emit addition
        if let Some(cl) = change.curr_line {
            if cl < curr_lines.len() {
                entries.push(Entry::Added { curr_ln: cl });
                if curr_idx <= cl {
                    curr_idx = cl + 1;
                }
            }
        }
    }

    // Emit remaining context
    while prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
        entries.push(Entry::Context {
            prev_ln: prev_idx,
            curr_ln: curr_idx,
        });
        prev_idx += 1;
        curr_idx += 1;
    }

    if entries.is_empty() {
        return Ok(DiffOutput::Unchanged);
    }

    // Find positions of change entries (non-context)
    let change_positions: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e, Entry::Context { .. }))
        .map(|(i, _)| i)
        .collect();

    if change_positions.is_empty() {
        return Ok(DiffOutput::Unchanged);
    }

    // Expand each change position by context_lines and merge overlapping ranges
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &pos in &change_positions {
        let start = pos.saturating_sub(context_lines);
        let end = (pos + context_lines + 1).min(entries.len());
        if let Some(last) = ranges.last_mut() {
            if start <= last.1 {
                // Overlapping or adjacent — merge
                last.1 = last.1.max(end);
            } else {
                ranges.push((start, end));
            }
        } else {
            ranges.push((start, end));
        }
    }

    let mut added = 0u64;
    let mut removed = 0u64;
    let mut render = String::new();

    render.push_str(FORMAT_PREFIX_CONTEXT);
    render.push_str(&format!("--- {}\n", file_path));
    render.push_str(&format!("+++ {}\n", file_path));

    for (hunk_start, hunk_end) in &ranges {
        // Compute hunk header: count prev and curr lines
        let mut prev_count = 0usize;
        let mut curr_count = 0usize;
        let mut prev_start: Option<usize> = None;
        let mut curr_start: Option<usize> = None;

        for entry in entries.iter().take(*hunk_end).skip(*hunk_start) {
            match entry {
                Entry::Context { prev_ln, curr_ln } => {
                    if prev_start.is_none() {
                        prev_start = Some(*prev_ln);
                    }
                    if curr_start.is_none() {
                        curr_start = Some(*curr_ln);
                    }
                    prev_count += 1;
                    curr_count += 1;
                }
                Entry::Removed { prev_ln } => {
                    if prev_start.is_none() {
                        prev_start = Some(*prev_ln);
                    }
                    prev_count += 1;
                }
                Entry::Added { curr_ln } => {
                    if curr_start.is_none() {
                        curr_start = Some(*curr_ln);
                    }
                    curr_count += 1;
                }
            }
        }

        let p_start = prev_start.unwrap_or(0) + 1; // 1-based
        let c_start = curr_start.unwrap_or(0) + 1; // 1-based

        render.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            p_start, prev_count, c_start, curr_count
        ));

        for entry in entries.iter().take(*hunk_end).skip(*hunk_start) {
            match entry {
                Entry::Context { curr_ln, .. } => {
                    render.push_str(&format!("{:>4} |  {}\n", curr_ln + 1, curr_lines[*curr_ln]));
                }
                Entry::Removed { prev_ln } => {
                    render.push_str(&format!("{:>4} | -{}\n", prev_ln + 1, prev_lines[*prev_ln]));
                    removed += 1;
                }
                Entry::Added { curr_ln } => {
                    render.push_str(&format!("{:>4} | +{}\n", curr_ln + 1, curr_lines[*curr_ln]));
                    added += 1;
                }
            }
        }
    }

    if render.trim().is_empty() {
        Ok(DiffOutput::Unchanged)
    } else {
        Ok(DiffOutput::Changed {
            render,
            added,
            removed,
        })
    }
}

/// Full file diff: entire file with `+`/`-`/` ` prefix on every line.
fn render_full_file(
    output: &DifftasticJsonOutput,
    prev_lines: &[&str],
    curr_lines: &[&str],
    file_path: &Utf8Path,
) -> Result<DiffOutput> {
    if output.status == "unchanged" {
        return Ok(DiffOutput::Unchanged);
    }
    if output.status != "changed" {
        return Ok(DiffOutput::Error {
            message: format!("unexpected difftastic status: {}", output.status),
        });
    }

    let changes = collect_line_changes(&output.chunks);

    let mut changed_curr_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut changed_prev_set: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for change in &changes {
        if let Some(cl) = change.curr_line {
            changed_curr_set.insert(cl);
        }
        if let Some(pl) = change.prev_line {
            changed_prev_set.insert(pl);
        }
    }

    if changed_curr_set.is_empty() && changed_prev_set.is_empty() {
        return Ok(DiffOutput::Unchanged);
    }

    let mut added = 0u64;
    let mut removed = 0u64;
    let mut render = String::new();

    render.push_str(FORMAT_PREFIX_FULL_FILE);
    render.push_str(&format!("--- {}\n", file_path));
    render.push_str(&format!("+++ {}\n", file_path));

    // Use two-pointer walk to interleave removed/added/context lines
    let mut prev_idx = 0usize;
    let mut curr_idx = 0usize;

    for change in &changes {
        let prev_target = change.prev_line.unwrap_or(prev_lines.len());
        let curr_target = change.curr_line.unwrap_or(curr_lines.len());

        // Context lines
        while prev_idx < prev_target && curr_idx < curr_target {
            if prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
                render.push_str(&format!(
                    "{:>4} |  {}\n",
                    curr_idx + 1,
                    curr_lines[curr_idx]
                ));
            }
            prev_idx += 1;
            curr_idx += 1;
        }

        // Removed line from prev
        if let Some(pl) = change.prev_line {
            if pl < prev_lines.len() {
                render.push_str(&format!("{:>4} | -{}\n", pl + 1, prev_lines[pl]));
                removed += 1;
                if prev_idx <= pl {
                    prev_idx = pl + 1;
                }
            }
        }

        // Added line from curr
        if let Some(cl) = change.curr_line {
            if cl < curr_lines.len() {
                render.push_str(&format!("{:>4} | +{}\n", cl + 1, curr_lines[cl]));
                added += 1;
                if curr_idx <= cl {
                    curr_idx = cl + 1;
                }
            }
        }
    }

    // Remaining context
    while prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
        render.push_str(&format!(
            "{:>4} |  {}\n",
            curr_idx + 1,
            curr_lines[curr_idx]
        ));
        prev_idx += 1;
        curr_idx += 1;
    }

    if render.trim().is_empty() {
        Ok(DiffOutput::Unchanged)
    } else {
        Ok(DiffOutput::Changed {
            render,
            added,
            removed,
        })
    }
}

/// Side-by-side diff: left/right columns for visual comparison.
fn render_side_by_side(
    output: &DifftasticJsonOutput,
    prev_lines: &[&str],
    curr_lines: &[&str],
    file_path: &Utf8Path,
    width: usize,
) -> Result<DiffOutput> {
    if output.status == "unchanged" {
        return Ok(DiffOutput::Unchanged);
    }
    if output.status != "changed" {
        return Ok(DiffOutput::Error {
            message: format!("unexpected difftastic status: {}", output.status),
        });
    }

    let changes = collect_line_changes(&output.chunks);

    let mut changed_curr_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut changed_prev_set: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for change in &changes {
        if let Some(cl) = change.curr_line {
            changed_curr_set.insert(cl);
        }
        if let Some(pl) = change.prev_line {
            changed_prev_set.insert(pl);
        }
    }

    if changed_curr_set.is_empty() && changed_prev_set.is_empty() {
        return Ok(DiffOutput::Unchanged);
    }

    let col_width = (width - 3) / 2; // 3 chars for separator " | "
    let mut added = 0u64;
    let mut removed = 0u64;
    let mut render = String::new();

    render.push_str(FORMAT_PREFIX_SIDE_BY_SIDE);
    render.push_str(&format!("--- {}\n", file_path));
    render.push_str(&format!("+++ {}\n", file_path));
    render.push_str(&format!(
        "{:<col_width$} | {:<col_width$}\n",
        "previous",
        "current",
        col_width = col_width
    ));
    render.push_str(&format!("{}\n", "-".repeat(width)));

    // Walk both files with two-pointer alignment using change data
    let mut prev_idx = 0usize;
    let mut curr_idx = 0usize;

    for change in &changes {
        let prev_target = change.prev_line.unwrap_or(prev_lines.len());
        let curr_target = change.curr_line.unwrap_or(curr_lines.len());

        // Context lines up to this change
        while prev_idx < prev_target && curr_idx < curr_target {
            if prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
                let left = format!(" {}", prev_lines[prev_idx]);
                let right = format!(" {}", curr_lines[curr_idx]);
                render.push_str(&format!(
                    "{:<col_width$} | {:<col_width$}\n",
                    truncate_to_width(&left, col_width),
                    truncate_to_width(&right, col_width),
                ));
            }
            prev_idx += 1;
            curr_idx += 1;
        }

        // Emit the change as a side-by-side row
        match (change.prev_line, change.curr_line) {
            (Some(pl), Some(cl)) => {
                // Modified
                if pl < prev_lines.len() && cl < curr_lines.len() {
                    let left = format!("-{}", prev_lines[pl]);
                    let right = format!("+{}", curr_lines[cl]);
                    render.push_str(&format!(
                        "{:<col_width$} | {:<col_width$}\n",
                        truncate_to_width(&left, col_width),
                        truncate_to_width(&right, col_width),
                    ));
                    removed += 1;
                    added += 1;
                }
                prev_idx = pl + 1;
                curr_idx = cl + 1;
            }
            (Some(pl), None) => {
                // Pure removal
                if pl < prev_lines.len() {
                    let left = format!("-{}", prev_lines[pl]);
                    render.push_str(&format!(
                        "{:<col_width$} | {:<col_width$}\n",
                        truncate_to_width(&left, col_width),
                        "",
                    ));
                    removed += 1;
                }
                prev_idx = pl + 1;
            }
            (None, Some(cl)) => {
                // Pure addition
                if cl < curr_lines.len() {
                    let right = format!("+{}", curr_lines[cl]);
                    render.push_str(&format!(
                        "{:<col_width$} | {:<col_width$}\n",
                        "",
                        truncate_to_width(&right, col_width),
                    ));
                    added += 1;
                }
                curr_idx = cl + 1;
            }
            (None, None) => {}
        }
    }

    // Remaining context
    while prev_idx < prev_lines.len() && curr_idx < curr_lines.len() {
        let left = format!(" {}", prev_lines[prev_idx]);
        let right = format!(" {}", curr_lines[curr_idx]);
        render.push_str(&format!(
            "{:<col_width$} | {:<col_width$}\n",
            truncate_to_width(&left, col_width),
            truncate_to_width(&right, col_width),
        ));
        prev_idx += 1;
        curr_idx += 1;
    }

    if render.trim().is_empty() {
        Ok(DiffOutput::Unchanged)
    } else {
        Ok(DiffOutput::Changed {
            render,
            added,
            removed,
        })
    }
}

/// Truncate a string to at most `width` visible characters.
fn truncate_to_width(s: &str, width: usize) -> String {
    s.chars().take(width).collect()
}

/// Locate the difft binary:
/// 1. Next to the current executable (cargo-built via `cargo install difftastic`)
/// 2. DIFFTASTIC_PATH env var (explicit override)
/// 3. Bare "difft" on PATH (system-installed or cargo-installed)
fn find_difft_binary() -> (PathBuf, bool) {
    // 1. Next to the current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in ["difft", "difft.exe"] {
                let path = dir.join(candidate);
                if probe_difft(path.to_str().unwrap_or("")) {
                    return (path, true);
                }
            }
        }
    }

    // 2. Explicit override
    if let Ok(path) = std::env::var("DIFFTASTIC_PATH") {
        if probe_difft(&path) {
            return (PathBuf::from(path), true);
        }
    }

    // 3. System PATH
    if probe_difft("difft") {
        return (PathBuf::from("difft"), true);
    }

    (PathBuf::from("difft"), false)
}

fn probe_difft(path: &str) -> bool {
    std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_json_changed() -> &'static str {
        r#"{"status":"changed","language":"YAML","path":"/tmp/garbage.yaml","chunks":[[{"lhs":{"line_number":0,"changes":[{"content":"old","highlight":"normal"}]},"rhs":{"line_number":0,"changes":[{"content":"new","highlight":"normal"}]}}]]}"#
    }

    fn test_json_multi_line() -> &'static str {
        // line_number 2 = third line (0-based), which is "replicas: ..."
        r#"{"status":"changed","language":"YAML","chunks":[[{"lhs":{"line_number":2,"changes":[{"content":"2","highlight":"normal"}]},"rhs":{"line_number":2,"changes":[{"content":"3","highlight":"normal"}]}}]]}"#
    }

    fn test_json_added_line() -> &'static str {
        r#"{"status":"changed","language":"YAML","chunks":[[{"lhs":null,"rhs":{"line_number":3,"changes":[{"content":"image","highlight":"string"}]}}]]}"#
    }

    fn make_engine(format: DiffFormat) -> DiffEngine {
        DiffEngine {
            difft_path: PathBuf::new(),
            difft_available: false,
            config: DiffConfig {
                format,
                context_lines: 3,
                side_by_side_width: 80,
            },
        }
    }

    // --- Line diff tests (format-independent fallback) ---

    #[test]
    fn line_diff_detects_changes() {
        let engine = make_engine(DiffFormat::Unified);
        let result = engine.line_diff("key: old", "key: new");
        match result {
            DiffOutput::Changed {
                render,
                added,
                removed,
            } => {
                assert!(
                    render.contains(FORMAT_PREFIX_UNIFIED),
                    "should have format prefix"
                );
                assert!(render.contains(" | +key: new"));
                assert_eq!(added, 1);
                assert_eq!(removed, 1);
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    #[test]
    fn line_diff_unchanged() {
        let engine = make_engine(DiffFormat::Unified);
        let result = engine.line_diff("key: value", "key: value");
        assert!(matches!(result, DiffOutput::Unchanged));
    }

    #[tokio::test]
    async fn compute_diff_uses_fallback_when_no_difftastic() {
        let engine = make_engine(DiffFormat::Unified);
        let result = engine
            .compute_diff("old", "new", Utf8Path::new("test.yaml"))
            .await
            .unwrap();
        assert!(matches!(result, DiffOutput::Changed { .. }));
    }

    #[test]
    fn find_difft_binary_never_panics() {
        let (path, _available) = find_difft_binary();
        assert!(!path.as_os_str().is_empty());
    }

    // --- Unified format tests ---

    #[test]
    fn unified_uses_real_path() {
        let prev = "key: old";
        let curr = "key: new";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_changed()).unwrap();

        let result = render_unified(
            &output,
            &prev_lines,
            &curr_lines,
            Utf8Path::new("fixtures/yaml/app.yaml"),
        )
        .unwrap();
        match result {
            DiffOutput::Changed { render, .. } => {
                assert!(
                    render.contains("fixtures/yaml/app.yaml"),
                    "should use real path, got: {}",
                    render
                );
                assert!(
                    !render.contains("/tmp"),
                    "should not contain temp path, got: {}",
                    render
                );
                assert!(
                    render.contains(" | -key: old"),
                    "should contain removed line, got: {}",
                    render
                );
                assert!(
                    render.contains(" | +key: new"),
                    "should contain added line, got: {}",
                    render
                );
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    #[test]
    fn unified_shows_only_changed_lines() {
        let prev = "service:\n  name: demo\n  replicas: 2";
        let curr = "service:\n  name: demo\n  replicas: 3";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_multi_line()).unwrap();

        let result =
            render_unified(&output, &prev_lines, &curr_lines, Utf8Path::new("app.yaml")).unwrap();
        match result {
            DiffOutput::Changed { render, .. } => {
                assert!(
                    render.contains(" | -  replicas: 2"),
                    "should contain removed line, got: {}",
                    render
                );
                assert!(
                    render.contains(" | +  replicas: 3"),
                    "should contain added line, got: {}",
                    render
                );
                assert!(
                    !render.contains("name: demo"),
                    "should not contain unchanged context, got: {}",
                    render
                );
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    #[test]
    fn unified_added_line() {
        let prev = "service:\n  name: demo\n  replicas: 2";
        let curr = "service:\n  name: demo\n  replicas: 2\n  image: latest";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_added_line()).unwrap();

        let result =
            render_unified(&output, &prev_lines, &curr_lines, Utf8Path::new("app.yaml")).unwrap();
        match result {
            DiffOutput::Changed {
                render,
                added,
                removed,
            } => {
                assert!(render.contains(" | +  image: latest"), "got: {}", render);
                assert_eq!(added, 1);
                assert_eq!(removed, 0);
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    // --- Full file format tests ---

    #[test]
    fn full_file_shows_all_lines() {
        let prev = "service:\n  name: demo\n  replicas: 2";
        let curr = "service:\n  name: demo\n  replicas: 3";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_multi_line()).unwrap();

        let result =
            render_full_file(&output, &prev_lines, &curr_lines, Utf8Path::new("app.yaml")).unwrap();
        match result {
            DiffOutput::Changed { render, .. } => {
                assert!(
                    render.contains(FORMAT_PREFIX_FULL_FILE),
                    "should have full_file format prefix, got: {}",
                    render
                );
                // Should show unchanged lines with " " prefix
                assert!(
                    render.contains(" |  service:"),
                    "should show unchanged lines, got: {}",
                    render
                );
                assert!(
                    render.contains(" |    name: demo"),
                    "should show unchanged lines, got: {}",
                    render
                );
                // Should show old line with "-" and new line with "+"
                assert!(
                    render.contains(" | -  replicas: 2"),
                    "should show removed line, got: {}",
                    render
                );
                assert!(
                    render.contains(" | +  replicas: 3"),
                    "should show added line, got: {}",
                    render
                );
                // Removed line should appear before added line (interleaved)
                let rem_pos = render.find("-  replicas: 2").unwrap();
                let add_pos = render.find("+  replicas: 3").unwrap();
                assert!(
                    rem_pos < add_pos,
                    "removed should appear before added, got: {}",
                    render
                );
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    // --- Side-by-side format tests ---

    #[test]
    fn side_by_side_shows_both_columns() {
        let prev = "service:\n  name: demo\n  replicas: 2";
        let curr = "service:\n  name: demo\n  replicas: 3";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_multi_line()).unwrap();

        let result = render_side_by_side(
            &output,
            &prev_lines,
            &curr_lines,
            Utf8Path::new("app.yaml"),
            80,
        )
        .unwrap();
        match result {
            DiffOutput::Changed { render, .. } => {
                assert!(
                    render.contains(FORMAT_PREFIX_SIDE_BY_SIDE),
                    "should have side_by_side format prefix, got: {}",
                    render
                );
                assert!(
                    render.contains("previous"),
                    "should have column header, got: {}",
                    render
                );
                assert!(
                    render.contains("current"),
                    "should have column header, got: {}",
                    render
                );
                assert!(
                    render.contains("|"),
                    "should have separator, got: {}",
                    render
                );
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    // --- Context format test ---

    #[test]
    fn context_format_includes_unchanged_lines() {
        let prev = "service:\n  name: demo\n  replicas: 2";
        let curr = "service:\n  name: demo\n  replicas: 3";
        let prev_lines: Vec<&str> = prev.lines().collect();
        let curr_lines: Vec<&str> = curr.lines().collect();
        let output: DifftasticJsonOutput = serde_json::from_str(test_json_multi_line()).unwrap();

        let result = render_context(
            &output,
            &prev_lines,
            &curr_lines,
            Utf8Path::new("app.yaml"),
            3,
        )
        .unwrap();
        match result {
            DiffOutput::Changed { render, .. } => {
                assert!(
                    render.contains(FORMAT_PREFIX_CONTEXT),
                    "should have context format prefix, got: {}",
                    render
                );
                assert!(
                    render.contains("@@"),
                    "should contain hunk header, got: {}",
                    render
                );
                assert!(
                    render.contains(" | -  replicas: 2"),
                    "should contain removed line within hunk, got: {}",
                    render
                );
                assert!(
                    render.contains(" | +  replicas: 3"),
                    "should contain added line within hunk, got: {}",
                    render
                );
                assert!(
                    render.contains(" |  service:"),
                    "should contain context lines, got: {}",
                    render
                );
            }
            _ => panic!("expected Changed, got {:?}", result),
        }
    }

    // --- Config defaults test ---

    #[test]
    fn diff_config_defaults() {
        let config = DiffConfig::default();
        assert_eq!(config.format, DiffFormat::Unified);
        assert_eq!(config.context_lines, 3);
        assert_eq!(config.side_by_side_width, 120);
    }

    // --- Format prefix tests ---

    #[test]
    fn format_prefixes_are_distinct() {
        let prefixes = [
            FORMAT_PREFIX_UNIFIED,
            FORMAT_PREFIX_CONTEXT,
            FORMAT_PREFIX_FULL_FILE,
            FORMAT_PREFIX_SIDE_BY_SIDE,
            FORMAT_PREFIX_RAW,
        ];
        for (i, a) in prefixes.iter().enumerate() {
            for (j, b) in prefixes.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "format prefixes should be distinct");
                }
            }
        }
    }

    #[test]
    fn format_prefix_starts_with_at_sign() {
        assert!(FORMAT_PREFIX_UNIFIED.starts_with("@@format:"));
        assert!(FORMAT_PREFIX_CONTEXT.starts_with("@@format:"));
        assert!(FORMAT_PREFIX_FULL_FILE.starts_with("@@format:"));
        assert!(FORMAT_PREFIX_SIDE_BY_SIDE.starts_with("@@format:"));
        assert!(FORMAT_PREFIX_RAW.starts_with("@@format:"));
    }

    #[test]
    fn diff_format_serde_roundtrip() {
        for format in [
            DiffFormat::Unified,
            DiffFormat::Context,
            DiffFormat::FullFile,
            DiffFormat::SideBySide,
            DiffFormat::Raw,
        ] {
            let s = serde_json::to_string(&format).unwrap();
            let parsed: DiffFormat = serde_json::from_str(&s).unwrap();
            assert_eq!(format, parsed);
        }
    }

    #[test]
    fn diff_config_toml_parsing() {
        let toml = r#"
format = "context"
context_lines = 5
side_by_side_width = 160
"#;
        let config: DiffConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.format, DiffFormat::Context);
        assert_eq!(config.context_lines, 5);
        assert_eq!(config.side_by_side_width, 160);
    }

    // --- Real difftastic test (skipped if not installed) ---

    #[tokio::test]
    async fn difftastic_produces_changed_not_error() {
        let engine = DiffEngine::new();
        if !engine.difft_available {
            return;
        }
        let result = engine
            .compute_diff("key: old", "key: new", Utf8Path::new("app.yaml"))
            .await
            .unwrap();
        match &result {
            DiffOutput::Changed {
                render,
                added,
                removed,
            } => {
                assert!(!render.trim().is_empty());
                assert!(*added > 0 || *removed > 0);
                assert!(!render.contains("/tmp"), "should not contain temp path");
                assert!(!render.contains("tempdir"), "should not contain temp dir");
                assert!(render.contains("app.yaml"), "should contain real file path");
            }
            DiffOutput::Error { message } => {
                panic!(
                    "differences found should be Changed, not Error: {}",
                    message
                );
            }
            DiffOutput::Unchanged => {
                panic!("different inputs should not be Unchanged");
            }
        }
    }
}
