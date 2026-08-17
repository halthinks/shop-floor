//! Event log, Atom, and RSS. Only real store events — never invented.

use serde_json::Value;

use crate::hash::format_rfc3339;

pub fn event_title(op: &str) -> &'static str {
    match op {
        "add_worker" => "worker added",
        "set_intelligence" => "intelligence assigned",
        "open" => "parent open",
        "split" => "split",
        "assign" => "assign written",
        "accept" => "accept",
        "bounce" => "bounce",
        "join" => "join",
        "reduce" => "reduce",
        "verify" => "verify",
        "github_connect" => "github connect",
        "mailbox_wait" => "mailbox WAIT",
        "merge_blocked" => "merge blocked",
        "merge" => "merge allowed",
        "close" => "close",
        _ => "event",
    }
}

pub fn title_of(event: &Value) -> String {
    if let Some(t) = event.get("title").and_then(|v| v.as_str()) {
        if !t.is_empty() {
            return t.to_string();
        }
    }
    event
        .get("op")
        .and_then(|v| v.as_str())
        .map(event_title)
        .unwrap_or("event")
        .to_string()
}

pub fn detail_of(event: &Value) -> String {
    let mut parts = Vec::new();
    for key in [
        "peer", "parent", "child", "repo", "backend", "status", "reason",
    ] {
        if let Some(v) = event.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                parts.push(format!("{key}={v}"));
            }
        }
    }
    if let Some(n) = event.get("count").and_then(|v| v.as_u64()) {
        parts.push(format!("count={n}"));
    }
    if let Some(note) = event.get("note").and_then(|v| v.as_str()) {
        if parts.is_empty() && !note.is_empty() {
            parts.push(note.to_string());
        }
    }
    parts.join("  ")
}

pub fn at_of(event: &Value) -> String {
    if let Some(at) = event.get("at").and_then(|v| v.as_str()) {
        return at.to_string();
    }
    if let Some(ts) = event.get("ts").and_then(|v| v.as_u64()) {
        return format_rfc3339(ts);
    }
    String::new()
}

/// Chronological tail (oldest of the window first). Same events as the feed.
pub fn format_log(events: &[Value], n: usize) -> String {
    let start = events.len().saturating_sub(n);
    let mut out = String::new();
    for e in &events[start..] {
        let at = at_of(e);
        let title = title_of(e);
        let detail = detail_of(e);
        if detail.is_empty() {
            out.push_str(&format!("{at}  {title}\n"));
        } else {
            out.push_str(&format!("{at}  {title}  {detail}\n"));
        }
    }
    out
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Atom 1.0. Entries are newest first. No invented items.
pub fn atom(events: &[Value]) -> String {
    let mut newest = events.to_vec();
    newest.reverse();
    let updated = newest
        .first()
        .map(at_of)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>shop</title>
  <subtitle>shop-floor activity</subtitle>
  <id>tag:shop,local:events</id>
  <link rel="self" href="/feed.xml"/>
"#,
    );
    out.push_str(&format!("  <updated>{}</updated>\n", xml_esc(&updated)));
    for (i, e) in newest.iter().enumerate() {
        let title = title_of(e);
        let at = at_of(e);
        let at = if at.is_empty() {
            "1970-01-01T00:00:00Z".into()
        } else {
            at
        };
        let detail = detail_of(e);
        let summary = if detail.is_empty() {
            title.clone()
        } else {
            format!("{title}  {detail}")
        };
        let op = e.get("op").and_then(|v| v.as_str()).unwrap_or("event");
        let ts = e.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
        out.push_str(&format!(
            "  <entry>\n    <title>{}</title>\n    <id>tag:shop,local:{ts}:{i}:{op}</id>\n    <updated>{}</updated>\n    <summary>{}</summary>\n  </entry>\n",
            xml_esc(&title),
            xml_esc(&at),
            xml_esc(&summary)
        ));
    }
    out.push_str("</feed>\n");
    out
}

/// RSS 2.0. Same events as Atom.
pub fn rss(events: &[Value]) -> String {
    let mut newest = events.to_vec();
    newest.reverse();
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
  <title>shop</title>
  <link>/feed.xml</link>
  <description>shop-floor activity</description>
"#,
    );
    for e in &newest {
        let title = title_of(e);
        let detail = detail_of(e);
        let desc = if detail.is_empty() {
            title.clone()
        } else {
            format!("{title}  {detail}")
        };
        out.push_str(&format!(
            "  <item>\n    <title>{}</title>\n    <description>{}</description>\n  </item>\n",
            xml_esc(&title),
            xml_esc(&desc)
        ));
    }
    out.push_str("</channel>\n</rss>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn atom_has_feed_tag() {
        let xml =
            atom(&[json!({"op":"add_worker","peer":"cursor","ts":1,"at":"1970-01-01T00:00:01Z"})]);
        assert!(xml.contains("<feed"));
        assert!(xml.contains("worker added"));
        assert!(xml.contains("cursor"));
    }
}
