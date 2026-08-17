//! Claim-lock path overlap. Component-aware prefixes collide; `src/foo` does not
//! collide with `src/foobar`.

pub fn normalize_path(raw: &str) -> String {
    let mut s = raw.replace('\\', "/");
    s = s.trim().to_string();
    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    s = s.trim_matches('/').to_string();
    s
}

pub fn parse_paths(csv: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in csv.split(',') {
        let n = normalize_path(part);
        if n.is_empty() {
            continue;
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// True when `a` and `b` are the same path or one is a directory prefix of the other.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let a = normalize_path(a);
    let b = normalize_path(b);
    if a.is_empty() || b.is_empty() {
        return true;
    }
    a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_overlaps_and_sibling_does_not() {
        assert!(paths_overlap("src/foo", "src/foo"));
        assert!(paths_overlap("src", "src/foo"));
        assert!(paths_overlap("src/foo/bar", "src/foo"));
        assert!(!paths_overlap("src/foo", "src/foobar"));
        assert!(!paths_overlap("src/foo", "src/bar"));
    }

    #[test]
    fn parse_trims_and_dedups() {
        assert_eq!(parse_paths(" a, b ,a, ./c/ "), vec!["a", "b", "c"]);
    }
}
