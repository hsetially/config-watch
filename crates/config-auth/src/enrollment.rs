use std::collections::HashSet;

pub struct EnrollmentVerifier {
    valid_tokens: HashSet<String>,
}

impl EnrollmentVerifier {
    pub fn new(tokens: Vec<String>) -> Self {
        Self {
            valid_tokens: tokens.into_iter().collect(),
        }
    }

    pub fn verify(&self, token: &str) -> bool {
        self.valid_tokens.contains(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_token_accepted() {
        let verifier = EnrollmentVerifier::new(vec!["token-abc".into(), "token-xyz".into()]);
        assert!(verifier.verify("token-abc"));
        assert!(verifier.verify("token-xyz"));
    }

    #[test]
    fn invalid_token_rejected() {
        let verifier = EnrollmentVerifier::new(vec!["token-abc".into()]);
        assert!(!verifier.verify("wrong-token"));
    }
}