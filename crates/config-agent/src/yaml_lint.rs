use config_shared::snapshots::{YamlLintFinding, YamlLintSeverity};

/// Lint YAML content for structural issues that may cause the file to not work
/// as a configuration file. Returns a list of findings sorted by severity
/// (Critical first, then Warning).
pub fn lint_yaml(content: &[u8]) -> Vec<YamlLintFinding> {
    let mut findings = Vec::new();

    // Convert to string for line-by-line analysis.
    // If not valid UTF-8, that's a critical issue on its own.
    let text = match std::str::from_utf8(content) {
        Ok(s) => s,
        Err(_) => {
            findings.push(YamlLintFinding {
                severity: YamlLintSeverity::Critical,
                check: "invalid_utf8".to_string(),
                message: "file contains invalid UTF-8 bytes".to_string(),
                line: None,
            });
            return findings;
        }
    };

    // Check for tab indentation (Critical — YAML spec violation).
    // Run before parsing since tab indentation is a structural issue
    // that's actionable even if the file also has parse errors.
    check_tab_indentation(text, &mut findings);

    // Try to parse the YAML. If parsing fails, that's a critical finding
    // and we skip the remaining (semantic) checks to avoid noise on broken input.
    let parse_result = serde_yaml::from_slice::<serde_yaml::Value>(content);
    if let Err(err) = parse_result {
        let line = extract_error_line(&err.to_string());
        findings.push(YamlLintFinding {
            severity: YamlLintSeverity::Critical,
            check: "parse_failure".to_string(),
            message: format!("YAML parse error: {}", err),
            line,
        });
        return findings;
    }

    // Check for implicit type coercion (Warning — may not work as expected).
    // Only run on successfully-parsed YAML to avoid noise.
    check_implicit_booleans(text, &mut findings);
    check_implicit_nulls(text, &mut findings);

    findings
}

/// Extract a line number from a serde_yaml error message.
/// Error messages contain patterns like "at line X column Y" or "on line X".
fn extract_error_line(msg: &str) -> Option<u32> {
    // Try "at line X" pattern
    if let Some(pos) = msg.find("at line ") {
        let rest = &msg[pos + 8..];
        if let Some(num_end) = rest.find(|c: char| !c.is_ascii_digit()) {
            if let Ok(n) = rest[..num_end].parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

/// Check for tab characters in YAML indentation.
/// Tabs are forbidden by the YAML spec for indentation.
fn check_tab_indentation(text: &str, findings: &mut Vec<YamlLintFinding>) {
    let mut tab_lines: Vec<u32> = Vec::new();

    for (i, line) in text.lines().enumerate() {
        // Check if any tab appears in the leading whitespace of this line
        let leading_whitespace: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();

        if leading_whitespace.contains('\t') {
            tab_lines.push((i + 1) as u32); // 1-based line numbers
        }
    }

    if tab_lines.is_empty() {
        return;
    }

    if tab_lines.len() <= 10 {
        for line_num in tab_lines {
            findings.push(YamlLintFinding {
                severity: YamlLintSeverity::Critical,
                check: "tab_indentation".to_string(),
                message: "tab character used for indentation (YAML forbids tabs in indentation)"
                    .to_string(),
                line: Some(line_num),
            });
        }
    } else {
        findings.push(YamlLintFinding {
            severity: YamlLintSeverity::Critical,
            check: "tab_indentation".to_string(),
            message: format!(
                "tab characters used for indentation on {} lines (YAML forbids tabs in indentation)",
                tab_lines.len()
            ),
            line: Some(tab_lines[0]),
        });
    }
}

/// Check for unquoted values that YAML will implicitly coerce to booleans.
/// Values like yes, no, on, off (case-insensitive) become true/false in YAML 1.1
/// and are ambiguous in YAML 1.2. These are common sources of config bugs.
fn check_implicit_booleans(text: &str, findings: &mut Vec<YamlLintFinding>) {
    let bool_words = [
        "yes", "no", "on", "off", "y", "n", "true", "false", "Yes", "No", "ON", "OFF", "Y", "N",
        "True", "False", "TRUE", "FALSE", "YES", "NO",
    ];

    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();

        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Skip block scalar indicators
        if trimmed.starts_with('|') || trimmed.starts_with('>') {
            continue;
        }

        // Look for mapping key: value pattern
        // Find the first colon that's followed by a space (key: value)
        if let Some(colon_pos) = find_mapping_colon(line) {
            let value_start = colon_pos + 1;
            let value_part = line[value_start..].trim_start();

            // Skip if value starts with a comment or is empty in a different way
            if value_part.starts_with('#') || value_part.is_empty() {
                continue;
            }

            // Skip quoted values
            if value_part.starts_with('"') || value_part.starts_with('\'') {
                continue;
            }

            // Skip block scalar values
            if value_part.starts_with('|') || value_part.starts_with('>') {
                continue;
            }

            // Skip list/flow collection values
            if value_part.starts_with('[') || value_part.starts_with('{') {
                continue;
            }

            // Get the first word of the value
            let first_word = value_part.split([' ', '\t', '#', ',']).next().unwrap_or("");

            // Check if it's an implicit boolean word
            if bool_words.contains(&first_word) {
                findings.push(YamlLintFinding {
                    severity: YamlLintSeverity::Warning,
                    check: "implicit_boolean".to_string(),
                    message: format!(
                        "value '{}' is implicitly coerced to a boolean; quote it to keep as string (e.g., \"{}\")",
                        first_word, first_word
                    ),
                    line: Some((i + 1) as u32),
                });
            }
        }

        // Also check sequence entries: "- yes"
        if let Some(seq_value) = trimmed.strip_prefix("- ") {
            let seq_value = seq_value.trim_start();
            if !seq_value.starts_with('"')
                && !seq_value.starts_with('\'')
                && !seq_value.starts_with('|')
                && !seq_value.starts_with('>')
                && !seq_value.starts_with('[')
                && !seq_value.starts_with('{')
            {
                let first_word = seq_value.split([' ', '\t', '#', ',']).next().unwrap_or("");
                if bool_words.contains(&first_word) {
                    findings.push(YamlLintFinding {
                        severity: YamlLintSeverity::Warning,
                        check: "implicit_boolean".to_string(),
                        message: format!(
                            "sequence value '{}' is implicitly coerced to a boolean; quote it to keep as string",
                            first_word
                        ),
                        line: Some((i + 1) as u32),
                    });
                }
            }
        }
    }
}

/// Find the position of the first colon in a line that is followed by a space
/// and is part of a mapping (not inside a quoted string). This is a simplified
/// heuristic that works for the vast majority of YAML config files.
fn find_mapping_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b':' if !in_single_quote
                && !in_double_quote
                && (i + 1 >= bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') =>
            {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Check for unquoted values that YAML interprets as null:
/// - bare `null`, `Null`, `NULL`
/// - bare `~`
/// - empty value after colon (key: with nothing after)
fn check_implicit_nulls(text: &str, findings: &mut Vec<YamlLintFinding>) {
    let null_words = ["null", "Null", "NULL"];
    let lines: Vec<&str> = text.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();

        // Skip comments and empty lines
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }

        // Check for mapping key: value pattern
        if let Some(colon_pos) = find_mapping_colon(line) {
            let value_start = colon_pos + 1;
            let value_part = line[value_start..].trim_start();

            // Empty value after colon (key: with nothing or just comments)
            let value_before_comment = value_part.split('#').next().unwrap_or("").trim();
            if value_before_comment.is_empty() {
                // If the next line is more indented, this is a block mapping/sequence parent,
                // not an implicit null.
                let current_indent = line.chars().take_while(|c| *c == ' ').count();
                let next_indented = lines.get(i + 1).is_some_and(|next| {
                    let next_indent = next.chars().take_while(|c| *c == ' ').count();
                    !next.trim_start().is_empty() && next_indent > current_indent
                });
                if next_indented {
                    continue;
                }

                findings.push(YamlLintFinding {
                    severity: YamlLintSeverity::Warning,
                    check: "implicit_null".to_string(),
                    message: "empty value is implicitly null; use an explicit value or quote null if intended".to_string(),
                    line: Some((i + 1) as u32),
                });
                continue;
            }

            // Skip quoted values
            if value_before_comment.starts_with('"') || value_before_comment.starts_with('\'') {
                continue;
            }

            // Get the first word
            let first_word = value_before_comment.split([' ', '\t']).next().unwrap_or("");

            // Check for bare ~
            if first_word == "~" {
                findings.push(YamlLintFinding {
                    severity: YamlLintSeverity::Warning,
                    check: "implicit_null".to_string(),
                    message: "bare '~' is implicitly null; quote it if a string is intended"
                        .to_string(),
                    line: Some((i + 1) as u32),
                });
                continue;
            }

            // Check for null/Null/NULL
            if null_words.contains(&first_word) {
                findings.push(YamlLintFinding {
                    severity: YamlLintSeverity::Warning,
                    check: "implicit_null".to_string(),
                    message: format!(
                        "value '{}' is implicitly null; quote it if a string is intended",
                        first_word
                    ),
                    line: Some((i + 1) as u32),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_valid_yaml_no_findings() {
        let yaml = b"key: value\nlist:\n  - item1\n  - item2\n";
        let findings = lint_yaml(yaml);
        assert!(
            findings.is_empty(),
            "expected no findings, got: {:?}",
            findings
        );
    }

    #[test]
    fn lint_standalone_empty_value_is_null() {
        // key: with nothing after and no indented next line is implicit null
        let yaml = b"key:\n";
        let findings = lint_yaml(yaml);
        let null_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_null")
            .collect();
        assert_eq!(null_findings.len(), 1);
    }

    #[test]
    fn lint_parse_failure_critical() {
        let yaml = b"key: [unclosed\n";
        let findings = lint_yaml(yaml);
        assert!(!findings.is_empty());
        let critical = findings.iter().find(|f| f.check == "parse_failure");
        assert!(critical.is_some(), "expected parse_failure finding");
        assert_eq!(critical.unwrap().severity, YamlLintSeverity::Critical);
    }

    #[test]
    fn lint_parse_failure_skips_semantic_checks() {
        let yaml = b"\tkey: [unclosed\n"; // tab + parse error
        let findings = lint_yaml(yaml);
        // Tab findings are reported even on parse failure (structural issue)
        let tab_findings = findings
            .iter()
            .filter(|f| f.check == "tab_indentation")
            .count();
        assert!(
            tab_findings > 0,
            "tab findings should be reported even on parse failure"
        );
        // But semantic checks (implicit boolean/null) are skipped
        let bool_findings = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .count();
        assert_eq!(
            bool_findings, 0,
            "boolean findings should be skipped on parse failure"
        );
    }

    #[test]
    fn lint_tab_indentation_single_line() {
        let yaml = b"\tkey: value\nother: fine\n";
        let findings = lint_yaml(yaml);
        let tab = findings
            .iter()
            .find(|f| f.check == "tab_indentation")
            .unwrap();
        assert_eq!(tab.severity, YamlLintSeverity::Critical);
        assert_eq!(tab.line, Some(1));
    }

    #[test]
    fn lint_tab_indentation_many_lines() {
        let mut yaml = String::new();
        for i in 0..15 {
            yaml.push_str(&format!("\tkey{}: value{}\n", i, i));
        }
        let findings = lint_yaml(yaml.as_bytes());
        let tab_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "tab_indentation")
            .collect();
        assert_eq!(
            tab_findings.len(),
            1,
            "should have one summary finding for many tabs"
        );
        assert!(tab_findings[0].message.contains("15 lines"));
    }

    #[test]
    fn lint_implicit_boolean_yes_no() {
        let yaml = b"feature: yes\ndatabase: no\n";
        let findings = lint_yaml(yaml);
        let bool_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .collect();
        assert_eq!(bool_findings.len(), 2);
        assert_eq!(bool_findings[0].line, Some(1)); // yes on line 1
        assert_eq!(bool_findings[1].line, Some(2)); // no on line 2
    }

    #[test]
    fn lint_implicit_boolean_on_off() {
        let yaml = b"debug: on\nverbose: off\n";
        let findings = lint_yaml(yaml);
        let bool_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .collect();
        assert_eq!(bool_findings.len(), 2);
    }

    #[test]
    fn lint_implicit_boolean_quoted_exempt() {
        let yaml = b"feature: \"yes\"\ndatabase: 'no'\n";
        let findings = lint_yaml(yaml);
        let bool_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .collect();
        assert!(
            bool_findings.is_empty(),
            "quoted values should not be flagged"
        );
    }

    #[test]
    fn lint_implicit_boolean_true_false_not_flagged() {
        // true and false are explicit booleans, not implicit coercion
        let yaml = b"enabled: true\ndisabled: false\n";
        let findings = lint_yaml(yaml);
        // true and false ARE in our check list because they're still implicit
        // coercion from YAML's perspective - the user might intend a string
        let bool_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .collect();
        assert_eq!(
            bool_findings.len(),
            2,
            "true/false should be flagged as implicit booleans"
        );
    }

    #[test]
    fn lint_implicit_null_tilde() {
        let yaml = b"timeout: ~\n";
        let findings = lint_yaml(yaml);
        let null_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_null")
            .collect();
        assert_eq!(null_findings.len(), 1);
        assert_eq!(null_findings[0].line, Some(1));
    }

    #[test]
    fn lint_implicit_null_empty_value() {
        let yaml = b"timeout:\n";
        let findings = lint_yaml(yaml);
        let null_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_null")
            .collect();
        assert_eq!(null_findings.len(), 1);
    }

    #[test]
    fn lint_implicit_null_word() {
        let yaml = b"config: null\n";
        let findings = lint_yaml(yaml);
        let null_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_null")
            .collect();
        assert_eq!(null_findings.len(), 1);
    }

    #[test]
    fn lint_implicit_null_quoted_exempt() {
        let yaml = b"config: \"null\"\ntimeout: '~'\n";
        let findings = lint_yaml(yaml);
        let null_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_null")
            .collect();
        assert!(
            null_findings.is_empty(),
            "quoted null values should not be flagged"
        );
    }

    #[test]
    fn lint_sequence_entry_implicit_boolean() {
        let yaml = b"features:\n  - yes\n  - no\n";
        let findings = lint_yaml(yaml);
        let bool_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "implicit_boolean")
            .collect();
        assert_eq!(bool_findings.len(), 2);
    }

    #[test]
    fn lint_invalid_utf8_critical() {
        let invalid = vec![0xFF, 0xFE, 0x00, 0x01];
        let findings = lint_yaml(&invalid);
        let utf8 = findings.iter().find(|f| f.check == "invalid_utf8");
        assert!(utf8.is_some());
        assert_eq!(utf8.unwrap().severity, YamlLintSeverity::Critical);
    }

    #[test]
    fn lint_mixed_findings() {
        // Tab indentation (critical) + implicit boolean (warning) in same file
        // Tabs cause parse failure, so we get both tab_indentation and parse_failure
        let yaml = b"feature: yes\n\tindented: value\n";
        let findings = lint_yaml(yaml);
        let criticals: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == YamlLintSeverity::Critical)
            .collect();
        // Should have tab_indentation finding (structural, runs before parse)
        let tab_findings = findings
            .iter()
            .filter(|f| f.check == "tab_indentation")
            .count();
        assert!(tab_findings > 0, "should have critical tab finding");
        // Should have parse_failure since tab-indented YAML doesn't parse
        let parse_findings = findings
            .iter()
            .filter(|f| f.check == "parse_failure")
            .count();
        assert!(parse_findings > 0, "should have parse failure finding");
        assert!(!criticals.is_empty(), "should have critical findings");
    }

    #[test]
    fn lint_complex_valid_yaml() {
        let yaml = br#"
server:
  host: "0.0.0.0"
  port: 8080
database:
  url: "postgres://localhost/mydb"
  pool_size: 10
logging:
  level: info
"#;
        let findings = lint_yaml(yaml);
        let warnings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == YamlLintSeverity::Warning)
            .collect();
        // "info" is not a boolean/null coercion word, so no warnings
        assert!(
            warnings.is_empty(),
            "no warnings expected for valid config, got: {:?}",
            warnings
        );
    }

    #[test]
    fn lint_flow_collection_exemption() {
        let yaml = b"config: {key: yes, flag: on}\n";
        let findings = lint_yaml(yaml);
        // Flow collection values starting with { or [ are not checked
        // (the first word after "config: " is "{key:" which is not a bool word)
        let _ = findings; // just ensure it doesn't panic
    }

    #[test]
    fn lint_comment_only_yaml() {
        let yaml = b"# just a comment\n";
        let findings = lint_yaml(yaml);
        assert!(
            findings.is_empty(),
            "comment-only yaml should have no findings"
        );
    }
}
