//! Local secret scan. Reports kind + offset only — never dumps secret material.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretHit {
    pub kind: String,
    pub offset: usize,
}

pub fn scan_secrets(content: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    push_prefix_hits(&mut hits, content, "ghp_", "github_pat_classic");
    push_prefix_hits(&mut hits, content, "github_pat_", "github_fine_grained_pat");
    push_prefix_hits(&mut hits, content, "gho_", "github_oauth");
    push_prefix_hits(&mut hits, content, "ghu_", "github_user_token");
    push_prefix_hits(&mut hits, content, "ghs_", "github_server_token");
    push_prefix_hits(&mut hits, content, "ghr_", "github_refresh_token");
    push_regex_like(&mut hits, content, "AKIA", 20, "aws_access_key_id");
    if let Some(offset) = content.find("-----BEGIN") {
        if content[offset..].contains("PRIVATE KEY") {
            hits.push(SecretHit {
                kind: "private_key_block".into(),
                offset,
            });
        }
    }
    hits.sort_by_key(|h| h.offset);
    hits.dedup_by(|a, b| a.kind == b.kind && a.offset == b.offset);
    hits
}

fn push_prefix_hits(hits: &mut Vec<SecretHit>, content: &str, prefix: &str, kind: &str) {
    let mut start = 0;
    while let Some(rel) = content[start..].find(prefix) {
        let offset = start + rel;
        let rest = &content[offset + prefix.len()..];
        let n = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if n >= 8 {
            hits.push(SecretHit {
                kind: kind.to_string(),
                offset,
            });
        }
        start = offset + prefix.len();
    }
}

fn push_regex_like(
    hits: &mut Vec<SecretHit>,
    content: &str,
    prefix: &str,
    body: usize,
    kind: &str,
) {
    let mut start = 0;
    while let Some(rel) = content[start..].find(prefix) {
        let offset = start + rel;
        let after = &content[offset + prefix.len()..];
        if after.len() >= body && after.chars().take(body).all(|c| c.is_ascii_alphanumeric()) {
            hits.push(SecretHit {
                kind: kind.to_string(),
                offset,
            });
        }
        start = offset + prefix.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_github_token_without_echoing_it() {
        let hits = scan_secrets("token=ghp_abcdefghijklmnopqrstuv");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "github_pat_classic");
        let dumped = serde_json::to_string(&hits).unwrap();
        assert!(!dumped.contains("ghp_abcdefghijklmnopqrstuv"));
    }
}
