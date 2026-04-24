use config_shared::events::Severity;
use config_shared::snapshots::DiffSummary;

/// Build a DiffSummary from structured counts (from difftastic JSON or line_diff).
pub fn build_diff_summary(
    added: u64,
    removed: u64,
    file_size_before: u64,
    file_size_after: u64,
    diff_render: &str,
) -> DiffSummary {
    let mut comment_changes = 0u64;
    let total_changes = added + removed;

    // Detect comment-only changes from render text (works for both line_diff and difftastic inline)
    for line in diff_render.lines() {
        let trimmed = line.trim_start();
        if (trimmed.starts_with('+') && !trimmed.starts_with("++"))
            || (trimmed.starts_with('-') && !trimmed.starts_with("--"))
        {
            let content = trimmed[1..].trim();
            if content.starts_with('#') {
                comment_changes += 1;
            }
        }
    }

    let comment_only_hint = total_changes > 0 && comment_changes == total_changes;
    let syntax_equivalent_hint = total_changes == 0 && file_size_before > 0 && file_size_after > 0;

    DiffSummary {
        changed_line_estimate: added + removed,
        file_size_before,
        file_size_after,
        comment_only_hint,
        syntax_equivalent_hint,
        yaml_lint_findings: vec![],
    }
}

/// Legacy parser for raw diff render text (line_diff `+`/`-` format).
/// Used when structured counts aren't available.
pub fn parse_diff_summary(diff_render: &str, file_size_before: u64, file_size_after: u64) -> DiffSummary {
    let mut added = 0u64;
    let mut removed = 0u64;
    let mut comment_changes = 0u64;
    let mut total_changes = 0u64;

    for line in diff_render.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('+') && !trimmed.starts_with("++") {
            added += 1;
            total_changes += 1;
            let content = trimmed[1..].trim();
            if content.starts_with('#') {
                comment_changes += 1;
            }
        } else if trimmed.starts_with('-') && !trimmed.starts_with("--") {
            removed += 1;
            total_changes += 1;
            let content = trimmed[1..].trim();
            if content.starts_with('#') {
                comment_changes += 1;
            }
        }
    }

    let comment_only_hint = total_changes > 0 && comment_changes == total_changes;
    let syntax_equivalent_hint = total_changes == 0 && file_size_before > 0 && file_size_after > 0;

    DiffSummary {
        changed_line_estimate: added + removed,
        file_size_before,
        file_size_after,
        comment_only_hint,
        syntax_equivalent_hint,
        yaml_lint_findings: vec![],
    }
}

pub fn classify_severity(summary: &DiffSummary, event_kind: &config_shared::events::ChangeKind) -> Severity {
    if matches!(event_kind, config_shared::events::ChangeKind::Deleted) {
        return Severity::Info;
    }
    if summary.changed_line_estimate > 50 {
        return Severity::Info;
    }
    Severity::Info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_diff_summary_counts_changes() {
        let summary = build_diff_summary(2, 1, 100, 120, "+ key: new\n- key: old\n+ added: line\n");
        assert_eq!(summary.changed_line_estimate, 3);
        assert_eq!(summary.file_size_before, 100);
        assert_eq!(summary.file_size_after, 120);
        assert!(!summary.comment_only_hint);
    }

    #[test]
    fn build_diff_summary_detects_comment_only() {
        let summary = build_diff_summary(1, 1, 50, 55, "+ # just a comment\n- # old comment\n");
        assert!(summary.comment_only_hint);
    }

    #[test]
    fn build_diff_summary_empty() {
        let summary = build_diff_summary(0, 0, 100, 100, "");
        assert_eq!(summary.changed_line_estimate, 0);
        assert!(summary.syntax_equivalent_hint);
    }

    #[test]
    fn parse_diff_summary_counts_changes() {
        let diff = "+ key: new\n- key: old\n+ added: line\n";
        let summary = parse_diff_summary(diff, 100, 120);
        assert_eq!(summary.changed_line_estimate, 3);
        assert_eq!(summary.file_size_before, 100);
        assert_eq!(summary.file_size_after, 120);
        assert!(!summary.comment_only_hint);
    }

    #[test]
    fn parse_diff_summary_detects_comment_only() {
        let diff = "+ # just a comment\n- # old comment\n";
        let summary = parse_diff_summary(diff, 50, 55);
        assert!(summary.comment_only_hint);
    }

    #[test]
    fn parse_diff_summary_empty() {
        let summary = parse_diff_summary("", 100, 100);
        assert_eq!(summary.changed_line_estimate, 0);
        assert!(summary.syntax_equivalent_hint);
    }

    #[test]
    fn classify_severity_deleted_is_info() {
        use config_shared::events::ChangeKind;
        let summary = DiffSummary {
            changed_line_estimate: 1,
            file_size_before: 100,
            file_size_after: 0,
            comment_only_hint: false,
            syntax_equivalent_hint: false,
            yaml_lint_findings: vec![],
        };
        assert_eq!(
            classify_severity(&summary, &ChangeKind::Deleted),
            Severity::Info
        );
    }

    #[test]
    fn classify_severity_large_change_is_info() {
        use config_shared::events::ChangeKind;
        let summary = DiffSummary {
            changed_line_estimate: 100,
            file_size_before: 1000,
            file_size_after: 2000,
            comment_only_hint: false,
            syntax_equivalent_hint: false,
            yaml_lint_findings: vec![],
        };
        assert_eq!(
            classify_severity(&summary, &ChangeKind::Modified),
            Severity::Info
        );
    }

    #[test]
    fn classify_severity_small_change_is_info() {
        use config_shared::events::ChangeKind;
        let summary = DiffSummary {
            changed_line_estimate: 3,
            file_size_before: 100,
            file_size_after: 120,
            comment_only_hint: false,
            syntax_equivalent_hint: false,
            yaml_lint_findings: vec![],
        };
        assert_eq!(
            classify_severity(&summary, &ChangeKind::Modified),
            Severity::Info
        );
    }
}