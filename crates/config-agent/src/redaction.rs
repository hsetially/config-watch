use regex::Regex;

pub struct RedactionEngine {
    pub key_patterns: Vec<Regex>,
    pub max_preview_bytes: usize,
}

impl RedactionEngine {
    pub fn new(additional_patterns: &[String], max_preview_bytes: usize) -> Self {
        let default_pattern = r"(?i)^(token|secret|password|key|credential|api_key|access_key|secret_key|private_key|auth_token|session_token|bearer).*$";
        let mut patterns = vec![Regex::new(default_pattern).unwrap()];

        for p in additional_patterns {
            if let Ok(re) = Regex::new(p) {
                patterns.push(re);
            }
        }

        Self {
            key_patterns: patterns,
            max_preview_bytes,
        }
    }

    pub fn redact_yaml(&self, content: &str) -> String {
        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let mut in_pem_block = false;

        for line in &mut lines {
            if line.trim().starts_with("-----BEGIN") {
                *line = "  [REDACTED PEM BLOCK]".to_string();
                in_pem_block = true;
                continue;
            }
            if in_pem_block {
                if line.trim().starts_with("-----END") {
                    in_pem_block = false;
                }
                *line = String::new();
                continue;
            }

            if let Some(colon_pos) = line.find(':') {
                let key = &line[..colon_pos].trim();
                if self.key_patterns.iter().any(|p| p.is_match(key)) {
                    let after_colon = &line[colon_pos + 1..];
                    let leading_ws = after_colon.len() - after_colon.trim_start().len();
                    *line = format!("{}:{}[REDACTED]", &line[..colon_pos], &" ".repeat(leading_ws));
                }
            }
        }

        lines.join("\n")
    }

    pub fn truncate(&self, content: &str) -> String {
        if content.len() <= self.max_preview_bytes {
            content.to_string()
        } else {
            let end = content.char_indices()
                .take_while(|(i, _)| *i < self.max_preview_bytes)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(self.max_preview_bytes);
            content[..end].to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_keys() {
        let engine = RedactionEngine::new(&[], 4096);
        let input = "database:\n  password: mysecretpass\n  host: localhost\napi_key: abc123\n";
        let result = engine.redact_yaml(input);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("mysecretpass"));
        assert!(!result.contains("abc123"));
        assert!(result.contains("host: localhost"));
    }

    #[test]
    fn redacts_pem_blocks() {
        let engine = RedactionEngine::new(&[], 4096);
        let input = "cert:\n  data: |\n    -----BEGIN CERTIFICATE-----\n    MIIBxx...\n    -----END CERTIFICATE-----\n  name: my-cert\n";
        let result = engine.redact_yaml(input);
        assert!(result.contains("[REDACTED PEM BLOCK]"));
        assert!(!result.contains("MIIBxx"));
        assert!(result.contains("name: my-cert"));
    }

    #[test]
    fn truncates_long_content() {
        let engine = RedactionEngine::new(&[], 20);
        let input = "012345678901234567890123456789";
        let result = engine.truncate(input);
        assert!(result.len() <= 20);
    }
}