//! CONTROL plane. Local shop verbs run here. Free text goes to SuperGrokHeavy
//! (`cursor-groksuperheavy`), never to grok-bot. Shop never invents a lane.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::Result;
use crate::hash::{format_rfc3339, unix_now};
use crate::mailbox::{write_steer, SteerRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteerVerb {
    Status,
    Workers,
    Github,
    AddWorker {
        peer: String,
        backend: String,
        model: String,
    },
    OpenParent {
        id: String,
        title: String,
    },
    Split {
        parent: String,
        child: String,
        peer: String,
        paths: String,
    },
    Assign {
        parent: String,
    },
    Join {
        parent: String,
    },
    Reduce {
        parent: String,
        note: String,
    },
    Verify {
        parent: String,
        cmd: Option<String>,
    },
    Bounce {
        parent: String,
        child: String,
        reason: String,
    },
    OpenProject {
        path: String,
    },
    OpenRepo {
        spec: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerLine {
    pub role: String,
    pub text: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl SteerLine {
    pub fn new(role: &str, text: impl Into<String>, wait: Option<String>) -> Self {
        Self {
            role: role.to_string(),
            text: text.into(),
            at: format_rfc3339(unix_now()),
            wait,
            to: None,
        }
    }
}

/// Parse a local steer verb. Incomplete `split` is not a verb — free text, no lane.
pub fn parse_steer(text: &str) -> Option<SteerVerb> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if lower == "status" {
        return Some(SteerVerb::Status);
    }
    if lower == "workers" {
        return Some(SteerVerb::Workers);
    }
    if lower == "github" {
        return Some(SteerVerb::Github);
    }
    if let Some(rest) = strip_prefix_ci(t, "add worker ") {
        let mut parts = rest.split_whitespace();
        let peer = parts.next()?.to_string();
        let backend = parts.next()?.to_string();
        let model = parts.collect::<Vec<_>>().join(" ");
        return Some(SteerVerb::AddWorker {
            peer,
            backend,
            model,
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open local ") {
        let path = rest.trim();
        if path.is_empty() {
            return None;
        }
        return Some(SteerVerb::OpenProject {
            path: path.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open project ") {
        let path = rest.trim();
        if path.is_empty() {
            return None;
        }
        return Some(SteerVerb::OpenProject {
            path: path.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open repo ") {
        let spec = rest.trim();
        if !spec.contains('/') {
            return None;
        }
        return Some(SteerVerb::OpenRepo {
            spec: spec.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "split ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() < 4 {
            return None;
        }
        return Some(SteerVerb::Split {
            parent: parts[0].to_string(),
            child: parts[1].to_string(),
            peer: parts[2].to_string(),
            paths: parts[3].to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "assign ") {
        let parent = rest.split_whitespace().next()?;
        return Some(SteerVerb::Assign {
            parent: parent.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "join ") {
        let parent = rest.split_whitespace().next()?;
        return Some(SteerVerb::Join {
            parent: parent.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "reduce ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let note = parts.collect::<Vec<_>>().join(" ");
        return Some(SteerVerb::Reduce { parent, note });
    }
    if let Some(rest) = strip_prefix_ci(t, "verify ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let cmd = {
            let c = parts.collect::<Vec<_>>().join(" ");
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        };
        return Some(SteerVerb::Verify { parent, cmd });
    }
    if let Some(rest) = strip_prefix_ci(t, "bounce ") {
        let mut parts = rest.split_whitespace();
        let parent = parts.next()?.to_string();
        let child = parts.next()?.to_string();
        let reason = parts.collect::<Vec<_>>().join(" ");
        if reason.is_empty() {
            return None;
        }
        return Some(SteerVerb::Bounce {
            parent,
            child,
            reason,
        });
    }
    if let Some(rest) = strip_prefix_ci(t, "open ") {
        let mut parts = rest.split_whitespace();
        let id = parts.next()?.to_string();
        if id.eq_ignore_ascii_case("project") || id.eq_ignore_ascii_case("repo") {
            return None;
        }
        let title = parts.collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            return None;
        }
        return Some(SteerVerb::OpenParent { id, title });
    }
    None
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    if text.get(..prefix.len())?.eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

pub fn steer_to_supergrok(
    outbox: &std::path::Path,
    mailbox: Option<&std::path::Path>,
    body: &str,
) -> Result<(SteerRecord, Vec<std::path::PathBuf>, Option<String>)> {
    let rec = SteerRecord::new(body);
    let written = write_steer(outbox, mailbox, &rec)?;
    let wait = if mailbox.map(|p| p.is_dir()).unwrap_or(false) {
        None
    } else {
        Some("WAIT: mailbox missing; recorded locally; no SuperGrokHeavy reply faked".into())
    };
    Ok((rec, written, wait))
}

pub fn line_json(line: &SteerLine) -> Value {
    json!(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_text_is_not_split() {
        assert!(parse_steer("please look at the floor").is_none());
        assert!(parse_steer("split the work").is_none());
        assert!(parse_steer("split P").is_none());
    }

    #[test]
    fn split_requires_ids_and_paths() {
        assert_eq!(
            parse_steer("split P c1 alice src/a,src/b"),
            Some(SteerVerb::Split {
                parent: "P".into(),
                child: "c1".into(),
                peer: "alice".into(),
                paths: "src/a,src/b".into(),
            })
        );
    }
}
