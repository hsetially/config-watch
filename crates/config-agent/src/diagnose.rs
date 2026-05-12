use camino::Utf8PathBuf;
use chrono::Utc;
use serde::Serialize;

use crate::config::AgentConfig;
use crate::watcher::{detect_mount_info, WatchBackend};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CheckResult {
    Pass { message: String },
    Warn { message: String },
    Fail { message: String, #[serde(skip_serializing_if = "Option::is_none")] detail: Option<String> },
}

#[derive(Debug, Serialize)]
pub struct DiagnosticCheck {
    pub name: String,
    pub category: String,
    pub result: CheckResult,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticReport {
    pub agent_id: String,
    pub environment: String,
    pub timestamp: String,
    pub checks: Vec<DiagnosticCheck>,
}

// ---------------------------------------------------------------------------
// Check implementations
// ---------------------------------------------------------------------------

fn check_watch_mode(cfg: &AgentConfig) -> DiagnosticCheck {
    let (backend, reason) = match cfg.watch_mode.as_str() {
        "poll" => (WatchBackend::Poll, "forced by config".to_string()),
        "inotify" => (WatchBackend::Inotify, "forced by config".to_string()),
        _ => {
            let any_nfs = cfg
                .watch_roots
                .iter()
                .any(|r| detect_mount_info(&r.root_path.to_string()).is_nfs);
            if any_nfs {
                (WatchBackend::Poll, "auto-detected NFS mount".to_string())
            } else {
                (WatchBackend::Inotify, "no NFS mount detected".to_string())
            }
        }
    };

    // Warn if forcing inotify on an NFS mount
    let result = if cfg.watch_mode == "inotify" {
        let any_nfs = cfg
            .watch_roots
            .iter()
            .any(|r| detect_mount_info(&r.root_path.to_string()).is_nfs);
        if any_nfs {
            CheckResult::Warn {
                message: format!(
                    "Effective backend: {} ({}), but NFS mount detected — inotify will NOT detect changes",
                    backend, reason
                ),
            }
        } else {
            CheckResult::Pass {
                message: format!("Effective backend: {} ({})", backend, reason),
            }
        }
    } else {
        CheckResult::Pass {
            message: format!("Effective backend: {} ({})", backend, reason),
        }
    };

    DiagnosticCheck {
        name: "watch_mode".into(),
        category: "watcher".into(),
        result,
    }
}

fn check_watch_roots_accessible(cfg: &AgentConfig) -> DiagnosticCheck {
    let mut accessible = 0usize;
    let mut failures = Vec::new();

    for root in &cfg.watch_roots {
        let path = root.root_path.as_std_path();
        if !path.exists() {
            failures.push(format!("{}: does not exist", root.root_path));
        } else if !path.is_dir() {
            failures.push(format!("{}: not a directory", root.root_path));
        } else {
            accessible += 1;
        }
    }

    let total = cfg.watch_roots.len();
    let result = if failures.is_empty() {
        CheckResult::Pass {
            message: format!("{}/{} roots accessible", accessible, total),
        }
    } else {
        CheckResult::Fail {
            message: format!("{}/{} roots accessible", accessible, total),
            detail: Some(failures.join("; ")),
        }
    };

    DiagnosticCheck {
        name: "watch_roots_accessible".into(),
        category: "watcher".into(),
        result,
    }
}

fn check_mount_types(cfg: &AgentConfig) -> DiagnosticCheck {
    let mut details = Vec::new();

    for root in &cfg.watch_roots {
        let info = detect_mount_info(&root.root_path.to_string());
        details.push(format!(
            "{}: mount={} fs={} is_nfs={}",
            info.path,
            info.mount_point.as_deref().unwrap_or("(none)"),
            info.fs_type.as_deref().unwrap_or("(local)"),
            info.is_nfs,
        ));
    }

    DiagnosticCheck {
        name: "mount_types".into(),
        category: "watcher".into(),
        result: CheckResult::Pass {
            message: details.join("; "),
        },
    }
}

fn check_dir_writable(label: &str, dir: &Utf8PathBuf) -> DiagnosticCheck {
    let result = if !dir.exists() {
        CheckResult::Fail {
            message: format!("{} directory does not exist: {}", label, dir),
            detail: None,
        }
    } else {
        let test_file = dir.join(".health_check_write_test");
        match std::fs::write(&test_file, b"test") {
            Ok(_) => {
                let _ = std::fs::remove_file(&test_file);
                CheckResult::Pass {
                    message: format!("{} writable: {}", label, dir),
                }
            }
            Err(e) => CheckResult::Fail {
                message: format!("{} not writable: {}", label, dir),
                detail: Some(e.to_string()),
            },
        }
    };

    DiagnosticCheck {
        name: format!("{}_writable", label.replace(' ', "_")),
        category: "storage".into(),
        result,
    }
}

fn check_control_plane_reachable(cfg: &AgentConfig) -> DiagnosticCheck {
    let url = &cfg.control_plane_base_url;

    // Try to parse the URL and extract host:port for a TCP connect check.
    let result = match url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) {
        Some(host_port) => {
            let (host, port) = if host_port.contains('/') {
                let hp = host_port.split('/').next().unwrap_or(host_port);
                parse_host_port(hp)
            } else {
                parse_host_port(host_port)
            };

            match std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::new(
                    host.parse().unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
                    port,
                ),
                std::time::Duration::from_secs(5),
            ) {
                Ok(_) => CheckResult::Pass {
                    message: format!("Control plane reachable at {}", url),
                },
                Err(e) => CheckResult::Fail {
                    message: format!("Control plane unreachable: {}", url),
                    detail: Some(e.to_string()),
                },
            }
        }
        None => CheckResult::Warn {
            message: format!("Cannot parse control_plane_base_url: {}", url),
        },
    };

    DiagnosticCheck {
        name: "control_plane_reachable".into(),
        category: "control_plane".into(),
        result,
    }
}

fn parse_host_port(s: &str) -> (String, u16) {
    if let Some(idx) = s.rfind(':') {
        let host = &s[..idx];
        let port: u16 = s[idx + 1..].parse().unwrap_or(8082);
        (host.to_string(), port)
    } else {
        (s.to_string(), 8082)
    }
}

fn check_difftastic_available() -> DiagnosticCheck {
    let (path, available) = config_diff::find_difft_binary();

    let result = if available {
        CheckResult::Pass {
            message: format!("difftastic found: {}", path.display()),
        }
    } else {
        CheckResult::Warn {
            message: "difftastic not found; line-diff fallback active".into(),
        }
    };

    DiagnosticCheck {
        name: "difftastic_available".into(),
        category: "tools".into(),
        result,
    }
}

fn check_config_validity(cfg: &AgentConfig) -> DiagnosticCheck {
    let result = match cfg.validate() {
        Ok(()) => CheckResult::Pass {
            message: "Configuration is valid".into(),
        },
        Err(e) => CheckResult::Fail {
            message: "Configuration validation failed".into(),
            detail: Some(e.to_string()),
        },
    };

    DiagnosticCheck {
        name: "config_validity".into(),
        category: "config".into(),
        result,
    }
}

fn check_tls_config(cfg: &AgentConfig) -> DiagnosticCheck {
    let result = if cfg.tls_required && !cfg.control_plane_base_url.starts_with("https://") {
        CheckResult::Fail {
            message: "tls_required=true but control_plane_base_url is not https://".into(),
            detail: Some(format!("URL: {}", cfg.control_plane_base_url)),
        }
    } else if !cfg.tls_required && cfg.control_plane_base_url.starts_with("https://") {
        CheckResult::Pass {
            message: "HTTPS URL with tls_required=false (acceptable for dev)".into(),
        }
    } else if !cfg.tls_required && !cfg.control_plane_base_url.starts_with("https://") {
        CheckResult::Warn {
            message: "tls_required=false with HTTP URL — not recommended for production".into(),
        }
    } else {
        CheckResult::Pass {
            message: "TLS configuration is consistent".into(),
        }
    };

    DiagnosticCheck {
        name: "tls_config".into(),
        category: "config".into(),
        result,
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

pub fn run(cfg: &AgentConfig, format: &str) -> anyhow::Result<()> {
    let mut checks = Vec::new();

    checks.push(check_watch_mode(cfg));
    checks.push(check_watch_roots_accessible(cfg));
    checks.push(check_mount_types(cfg));
    // yaml_files_found uses a simplified count; skip the glob logic
    // and just check if roots have any yaml at top level
    checks.push(check_yaml_files_found_simple(cfg));
    checks.push(check_dir_writable("snapshot", &cfg.snapshot_dir));
    checks.push(check_dir_writable("spool", &cfg.spool_dir));
    checks.push(check_control_plane_reachable(cfg));
    checks.push(check_difftastic_available());
    checks.push(check_config_validity(cfg));
    checks.push(check_tls_config(cfg));

    let report = DiagnosticReport {
        agent_id: cfg.agent_id.clone(),
        environment: cfg.environment.clone(),
        timestamp: Utc::now().to_rfc3339(),
        checks,
    };

    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => {
            print_text_report(&report);
        }
    }

    Ok(())
}

fn check_yaml_files_found_simple(cfg: &AgentConfig) -> DiagnosticCheck {
    let mut zero_roots = Vec::new();
    let mut total = 0usize;

    for root in &cfg.watch_roots {
        let mut count = 0usize;
        if root.root_path.exists() {
            if let Ok(entries) = std::fs::read_dir(&root.root_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path.to_string_lossy();
                        if name.ends_with(".yaml") || name.ends_with(".yml") {
                            count += 1;
                        }
                    }
                }
            }
        }
        total += count;
        if count == 0 {
            zero_roots.push(root.root_path.to_string());
        }
    }

    let result = if zero_roots.is_empty() {
        CheckResult::Pass {
            message: format!("{} YAML files found (top-level scan)", total),
        }
    } else {
        CheckResult::Warn {
            message: format!(
                "No YAML files in top-level of: {} (recursive watch may find more)",
                zero_roots.join(", ")
            ),
        }
    };

    DiagnosticCheck {
        name: "yaml_files_found".into(),
        category: "files".into(),
        result,
    }
}

fn print_text_report(report: &DiagnosticReport) {
    println!("=== config-agent diagnostics ===");
    println!("Agent ID:    {}", report.agent_id);
    println!("Environment: {}", report.environment);
    println!("Timestamp:   {}", report.timestamp);
    println!();

    for check in &report.checks {
        let status_str = match &check.result {
            CheckResult::Pass { .. } => "PASS",
            CheckResult::Warn { .. } => "WARN",
            CheckResult::Fail { .. } => "FAIL",
        };
        let message = match &check.result {
            CheckResult::Pass { message } => message,
            CheckResult::Warn { message } => message,
            CheckResult::Fail { message, .. } => message,
        };
        let detail = match &check.result {
            CheckResult::Fail { detail, .. } => detail.as_deref(),
            _ => None,
        };

        print!("[{}] {}:{} ", status_str, check.category, check.name);
        // Pad to align messages
        println!("{}", message);
        if let Some(d) = detail {
            println!("         {}", d);
        }
    }

    // Summary
    let pass_count = report.checks.iter().filter(|c| matches!(c.result, CheckResult::Pass { .. })).count();
    let warn_count = report.checks.iter().filter(|c| matches!(c.result, CheckResult::Warn { .. })).count();
    let fail_count = report.checks.iter().filter(|c| matches!(c.result, CheckResult::Fail { .. })).count();
    println!();
    println!(
        "Results: {} passed, {} warnings, {} failed",
        pass_count, warn_count, fail_count
    );

    if fail_count > 0 {
        std::process::exit(1);
    }
}