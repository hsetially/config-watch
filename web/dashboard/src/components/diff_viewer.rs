use yew::{function_component, html, UseStateHandle, Html, Properties};

use crate::models::{DiffLine, DiffLineKind, WordSegment};

#[derive(Properties, PartialEq)]
pub struct DiffViewerProps {
    pub diff_text: String,
    #[prop_or_default]
    pub collapsed: bool,
}

/// Detected diff format from the render string prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffFormat {
    Unified,
    Context,
    FullFile,
    SideBySide,
    Raw,
}

/// Parse the `@@format:...@@` prefix and return (format, remaining_text).
fn parse_format_prefix(text: &str) -> (DiffFormat, &str) {
    if let Some(rest) = text.strip_prefix("@@format:unified@@\n") {
        return (DiffFormat::Unified, rest);
    }
    if let Some(rest) = text.strip_prefix("@@format:context@@\n") {
        return (DiffFormat::Context, rest);
    }
    if let Some(rest) = text.strip_prefix("@@format:full_file@@\n") {
        return (DiffFormat::FullFile, rest);
    }
    if let Some(rest) = text.strip_prefix("@@format:side_by_side@@\n") {
        return (DiffFormat::SideBySide, rest);
    }
    if let Some(rest) = text.strip_prefix("@@format:raw@@\n") {
        return (DiffFormat::Raw, rest);
    }
    // Backward compatibility: no prefix → detect from content
    let is_side_by_side = text.lines().any(|l| {
        let trimmed = l.trim();
        trimmed.starts_with("previous") && trimmed.contains('|') && trimmed.contains("current")
    });
    if is_side_by_side {
        (DiffFormat::SideBySide, text)
    } else {
        (DiffFormat::Unified, text)
    }
}

#[function_component(DiffViewer)]
pub fn diff_viewer(props: &DiffViewerProps) -> Html {
    let (format, text) = parse_format_prefix(&props.diff_text);
    let expanded: UseStateHandle<bool> = yew::use_state(|| !props.collapsed);

    let is_expanded = *expanded;

    let toggle = {
        let expanded = expanded.clone();
        move |_| {
            expanded.set(!*expanded);
        }
    };

    let content_html = match format {
        DiffFormat::SideBySide => {
            let rows = parse_side_by_side_rows(text);
            html! {
                <div class="diff-sbs-table">
                    { for rows.iter().enumerate().map(|(i, row)| render_sbs_row(i, row)) }
                </div>
            }
        }
        _ => {
            let lines = parse_unified(text);
            html! {
                <div class="diff-lines">
                    { for lines.iter().enumerate().map(|(i, line)| render_diff_line(i, line)) }
                </div>
            }
        }
    };

    let line_count = match format {
        DiffFormat::SideBySide => parse_side_by_side_rows(text).len(),
        _ => parse_unified(text).len(),
    };

    let format_label = match format {
        DiffFormat::Unified => "unified",
        DiffFormat::Context => "context",
        DiffFormat::FullFile => "full file",
        DiffFormat::SideBySide => "side-by-side",
        DiffFormat::Raw => "raw",
    };

    html! {
        <div class="diff-viewer">
            <div class="diff-header">
                <span class="diff-line-count">{ format!("{} lines ({})", line_count, format_label) }</span>
                <button class="diff-toggle-btn" onclick={toggle}>
                    { if is_expanded { "Collapse" } else { "Expand" } }
                </button>
            </div>
            <div class={format!("diff-content{}", if is_expanded { "" } else { " diff-content-collapsed" })}>
                { content_html }
            </div>
        </div>
    }
}

fn render_diff_line(key: usize, line: &DiffLine) -> Html {
    let class = match line.kind {
        DiffLineKind::Header => "diff-line-header",
        DiffLineKind::HunkMeta => "diff-line-hunk",
        DiffLineKind::Added => "diff-line-added",
        DiffLineKind::Removed => "diff-line-removed",
        DiffLineKind::Context => "diff-line-context",
    };

    let prefix = match line.kind {
        DiffLineKind::Added => "+",
        DiffLineKind::Removed => "-",
        DiffLineKind::Context => " ",
        _ => "",
    };

    let line_num = line.line_num.map(|n| n.to_string()).unwrap_or_default();

    html! {
        <div class={class} key={key.to_string()}>
            <span class="diff-line-num">{ line_num }</span>
            <span class="diff-line-prefix">{ prefix }</span>
            <span class="diff-line-text">
                { for line.words.iter().map(|seg| {
                    if seg.changed {
                        html! { <span class="diff-word-hl">{ &seg.content }</span> }
                    } else {
                        html! { <span>{ &seg.content }</span> }
                    }
                }).collect::<Vec<Html>>() }
            </span>
        </div>
    }
}

// --- Side-by-side types and rendering ---

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SbsKind {
    Context,
    Removed,
    Added,
    Header,
    Separator,
}

#[derive(Debug, Clone)]
pub struct SideBySideRow {
    pub left: Option<String>,
    pub left_kind: SbsKind,
    pub right: Option<String>,
    pub right_kind: SbsKind,
}

fn render_sbs_row(key: usize, row: &SideBySideRow) -> Html {
    let left_class = match row.left_kind {
        SbsKind::Context => "diff-sbs-context",
        SbsKind::Removed => "diff-sbs-removed",
        SbsKind::Added => "diff-sbs-added",
        SbsKind::Header => "diff-sbs-header",
        SbsKind::Separator => "diff-sbs-separator-line",
    };
    let right_class = match row.right_kind {
        SbsKind::Context => "diff-sbs-context",
        SbsKind::Removed => "diff-sbs-removed",
        SbsKind::Added => "diff-sbs-added",
        SbsKind::Header => "diff-sbs-header",
        SbsKind::Separator => "diff-sbs-separator-line",
    };

    let left_text = row.left.as_deref().unwrap_or("");
    let right_text = row.right.as_deref().unwrap_or("");

    html! {
        <div class="diff-sbs-row" key={key.to_string()}>
            <div class={format!("diff-sbs-cell {}", left_class)}>
                { left_text }
            </div>
            <div class="diff-sbs-gutter">{ "│" }</div>
            <div class={format!("diff-sbs-cell {}", right_class)}>
                { right_text }
            </div>
        </div>
    }
}

fn parse_side_by_side_rows(text: &str) -> Vec<SideBySideRow> {
    let mut rows = Vec::new();

    for line in text.lines() {
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            rows.push(SideBySideRow {
                left: Some(line.to_string()),
                left_kind: SbsKind::Header,
                right: None,
                right_kind: SbsKind::Header,
            });
            continue;
        }

        // Skip the column header line and separator
        let trimmed = line.trim();
        if trimmed.starts_with("previous") && trimmed.contains('|') && trimmed.contains("current") {
            rows.push(SideBySideRow {
                left: Some("previous".to_string()),
                left_kind: SbsKind::Header,
                right: Some("current".to_string()),
                right_kind: SbsKind::Header,
            });
            continue;
        }
        if trimmed.starts_with('-') && line.chars().filter(|c| *c == '-').count() > 10 {
            continue;
        }

        let sep_pos = line.find(" | ");
        if let Some(pos) = sep_pos {
            let left = line[..pos].trim_end();
            let right = line[pos + 3..].trim_end();

            let left_stripped = if left.starts_with('-') || left.starts_with(' ') {
                left[1..].trim_end().to_string()
            } else {
                left.to_string()
            };

            let right_stripped = if right.starts_with('+') || right.starts_with(' ') {
                right[1..].trim_end().to_string()
            } else {
                right.to_string()
            };

            let left_is_removed = left.starts_with('-');
            let right_is_added = right.starts_with('+');

            // When left is empty and right has content, it's a pure addition
            // When right is empty and left has content, it's a pure removal
            let left_empty = left_stripped.is_empty();
            let right_empty = right_stripped.is_empty();

            let (left_kind, right_kind) = if left_is_removed && right_is_added {
                (SbsKind::Removed, SbsKind::Added)
            } else if left_is_removed && right_empty {
                (SbsKind::Removed, SbsKind::Context)
            } else if right_is_added && left_empty {
                (SbsKind::Context, SbsKind::Added)
            } else if left_is_removed {
                (SbsKind::Removed, SbsKind::Context)
            } else if right_is_added {
                (SbsKind::Context, SbsKind::Added)
            } else {
                (SbsKind::Context, SbsKind::Context)
            };

            rows.push(SideBySideRow {
                left: if left_empty { None } else { Some(left_stripped) },
                left_kind,
                right: if right_empty { None } else { Some(right_stripped) },
                right_kind,
            });
        }
    }

    rows
}

fn parse_unified(text: &str) -> Vec<DiffLine> {
    let mut raw: Vec<(DiffLineKind, Option<u32>, String)> = Vec::new();

    for line in text.lines() {
        // Try to extract line number prefix: "    N | rest" or "NNNN | rest"
        let (line_num, rest) = if let Some(pipe_pos) = line.find(" | ") {
            let num_part = line[..pipe_pos].trim();
            if let Ok(n) = num_part.parse::<u32>() {
                (Some(n), &line[pipe_pos + 3..])
            } else {
                (None, line)
            }
        } else {
            (None, line)
        };

        if rest.starts_with("diff ")
            || rest.starts_with("index ")
            || rest.starts_with("--- ")
            || rest.starts_with("+++ ")
        {
            raw.push((DiffLineKind::Header, None, rest.to_string()));
        } else if rest.starts_with("@@") {
            raw.push((DiffLineKind::HunkMeta, None, rest.to_string()));
        } else if let Some(stripped) = rest.strip_prefix('+') {
            raw.push((DiffLineKind::Added, line_num, stripped.to_string()));
        } else if let Some(stripped) = rest.strip_prefix('-') {
            raw.push((DiffLineKind::Removed, line_num, stripped.to_string()));
        } else if rest.starts_with(' ') && rest.len() > 1 {
            raw.push((DiffLineKind::Context, line_num, rest[1..].to_string()));
        } else {
            raw.push((DiffLineKind::Context, line_num, rest.to_string()));
        }
    }

    pair_and_build(raw)
}

/// Take classified lines, pair consecutive removed/added lines for word-level diff,
/// and produce final `DiffLine` structs with `WordSegment` arrays.
fn pair_and_build(raw: Vec<(DiffLineKind, Option<u32>, String)>) -> Vec<DiffLine> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < raw.len() {
        let (kind, line_num, ref content) = &raw[i];
        match *kind {
            DiffLineKind::Header | DiffLineKind::HunkMeta | DiffLineKind::Context => {
                result.push(DiffLine {
                    line_num: *line_num,
                    kind: *kind,
                    content: content.clone(),
                    words: vec![WordSegment {
                        content: content.clone(),
                        changed: false,
                    }],
                });
                i += 1;
            }
            DiffLineKind::Removed => {
                // Collect consecutive removed lines
                let mut removed_texts = vec![content.clone()];
                let mut removed_nums = vec![*line_num];
                let mut j = i + 1;
                while j < raw.len() && raw[j].0 == DiffLineKind::Removed {
                    removed_texts.push(raw[j].2.clone());
                    removed_nums.push(raw[j].1);
                    j += 1;
                }
                // Collect following added lines
                let mut added_texts: Vec<String> = Vec::new();
                let mut added_nums: Vec<Option<u32>> = Vec::new();
                while j < raw.len() && raw[j].0 == DiffLineKind::Added {
                    added_texts.push(raw[j].2.clone());
                    added_nums.push(raw[j].1);
                    j += 1;
                }

                let paired = removed_texts.len().min(added_texts.len());
                for k in 0..paired {
                    let left_words = word_diff(&removed_texts[k], &added_texts[k], true);
                    let right_words = word_diff(&removed_texts[k], &added_texts[k], false);
                    result.push(DiffLine {
                        line_num: removed_nums[k],
                        kind: DiffLineKind::Removed,
                        content: removed_texts[k].clone(),
                        words: left_words,
                    });
                    result.push(DiffLine {
                        line_num: added_nums[k],
                        kind: DiffLineKind::Added,
                        content: added_texts[k].clone(),
                        words: right_words,
                    });
                }
                for (idx, text) in removed_texts.iter().skip(paired).enumerate() {
                    let c = text.clone();
                    result.push(DiffLine {
                        line_num: removed_nums[paired + idx],
                        kind: DiffLineKind::Removed,
                        content: c.clone(),
                        words: vec![WordSegment {
                            content: c,
                            changed: true,
                        }],
                    });
                }
                for (idx, text) in added_texts.iter().skip(paired).enumerate() {
                    let c = text.clone();
                    result.push(DiffLine {
                        line_num: added_nums[paired + idx],
                        kind: DiffLineKind::Added,
                        content: c.clone(),
                        words: vec![WordSegment {
                            content: c,
                            changed: true,
                        }],
                    });
                }

                i = j;
            }
            DiffLineKind::Added => {
                let c = content.clone();
                result.push(DiffLine {
                    line_num: *line_num,
                    kind: DiffLineKind::Added,
                    content: c.clone(),
                    words: vec![WordSegment {
                        content: c,
                        changed: true,
                    }],
                });
                i += 1;
            }
        }
    }

    result
}

fn word_diff(left_line: &str, right_line: &str, is_left: bool) -> Vec<WordSegment> {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_words(left_line, right_line);
    let mut segments = Vec::new();

    for op in diff.ops() {
        for change in diff.iter_changes(op) {
            let content = change.to_string_lossy().to_string();
            match change.tag() {
                ChangeTag::Equal => {
                    segments.push(WordSegment {
                        content,
                        changed: false,
                    });
                }
                ChangeTag::Delete => {
                    if is_left {
                        segments.push(WordSegment {
                            content,
                            changed: true,
                        });
                    }
                }
                ChangeTag::Insert => {
                    if !is_left {
                        segments.push(WordSegment {
                            content,
                            changed: true,
                        });
                    }
                }
            }
        }
    }

    if segments.is_empty() {
        vec![WordSegment {
            content: if is_left {
                left_line.to_string()
            } else {
                right_line.to_string()
            },
            changed: true,
        }]
    } else {
        segments
    }
}