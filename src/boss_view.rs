//! Live floor view for SuperGrokHeavy.
//!
//! ROADMAP 4: the boss sees a status snapshot or image of the floor, not
//! only the memory pack (`profile` / `log` / `floor`) stuffed into a steer
//! JSON. This file is the view. Glue (engine / cli / ui) is in flight on
//! those paths and is not rewritten here.
//!
//! A snapshot is Evidence of what the floor files said now. Incomplete
//! evidence is WAIT — never JOINED without a handoff, never VERIFIED/PASS
//! from a picture. Never write `C:\TextPCB Platform`.
//!
//! Compiled from `tests/boss_view.rs` via `#[path]` until glue adds
//! `pub mod boss_view` and switches these `shop::` imports to `crate::`.
//! No direct `serde` / `serde_json` so the integration test crate can
//! compile this file without extra Cargo.toml deps.

use std::fs;
use std::path::{Path, PathBuf};

use shop::error::{Result, ShopError};
use shop::floor::{load_floor, FloorChild};
use shop::hash::{fingerprint, format_rfc3339, unix_now};
use shop::project::{is_forbidden_project, refuse_platform};
use shop::store::{ensure_under, write_atomic};
use shop::types::{ChildState, EvidenceStatus, ParentState};
use shop::{EvidenceClass, STEER_TO};

/// Distinct from a steer `memory` pack. SuperGrokHeavy reads this kind.
pub const KIND: &str = "live_floor_view";
pub const SCHEMA: &str = "shop/boss-view/v1";
pub const VIEW_REL: &str = "boss/view.json";
pub const SCREEN_REL: &str = "boss/screen.txt";
pub const IMAGE_REL: &str = "boss/view.ppm";

/// Keys of the memory pack `ShopMemory::pack_json` puts on a steer record.
pub const MEMORY_PACK_KEYS: &[&str] = &["profile", "log", "floor"];

pub const IMAGE_WIDTH: u32 = 64;
pub const IMAGE_HEIGHT: u32 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCurrent {
    pub id: String,
    pub state: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewChild {
    pub id: String,
    pub parent_id: String,
    pub peer: String,
    pub paths: Vec<String>,
    pub state: String,
    pub bounce_reason: Option<String>,
    pub handoff_hash: Option<String>,
    /// True when floor said JOINED but no handoff was on disk.
    pub joined_without_handoff: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewClaim {
    pub parent_id: String,
    pub child_id: String,
    pub peer: String,
    pub paths: Vec<String>,
}

/// Live floor view. Not a steer record and not a memory pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BossView {
    pub schema: String,
    pub kind: String,
    pub to: String,
    pub captured_at: String,
    pub status: EvidenceStatus,
    pub class: EvidenceClass,
    pub wait: Option<String>,
    pub current: Option<ViewCurrent>,
    pub children: Vec<ViewChild>,
    pub claims: Vec<ViewClaim>,
    pub screen: String,
    pub image_rel: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    pub view: PathBuf,
    pub screen: PathBuf,
    pub image: PathBuf,
}

impl BossView {
    pub fn is_pass(&self) -> bool {
        self.status == EvidenceStatus::Pass || self.class == EvidenceClass::Pass
    }

    /// JSON object for the live view. Never the steer memory-pack shape.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n");
        field(&mut s, "schema", &self.schema, true);
        field(&mut s, "kind", &self.kind, true);
        field(&mut s, "to", &self.to, true);
        field(&mut s, "captured_at", &self.captured_at, true);
        field(&mut s, "status", &self.status.to_string(), true);
        field(&mut s, "class", self.class.as_str(), true);
        match &self.wait {
            Some(w) => field(&mut s, "wait", w, true),
            None => s.push_str("  \"wait\": null,\n"),
        }
        match &self.current {
            None => s.push_str("  \"current\": null,\n"),
            Some(c) => {
                s.push_str("  \"current\": {\n");
                field_indent(&mut s, "id", &c.id, true, 4);
                field_indent(&mut s, "state", &c.state, true, 4);
                field_indent(&mut s, "title", &c.title, false, 4);
                s.push_str("  },\n");
            }
        }
        s.push_str("  \"children\": [\n");
        for (i, c) in self.children.iter().enumerate() {
            s.push_str("    {\n");
            field_indent(&mut s, "id", &c.id, true, 6);
            field_indent(&mut s, "parent_id", &c.parent_id, true, 6);
            field_indent(&mut s, "peer", &c.peer, true, 6);
            s.push_str("      \"paths\": ");
            s.push_str(&json_str_array(&c.paths));
            s.push_str(",\n");
            field_indent(&mut s, "state", &c.state, true, 6);
            opt_field(&mut s, "bounce_reason", c.bounce_reason.as_deref(), 6);
            opt_field(&mut s, "handoff_hash", c.handoff_hash.as_deref(), 6);
            s.push_str(&format!(
                "      \"joined_without_handoff\": {}\n",
                if c.joined_without_handoff {
                    "true"
                } else {
                    "false"
                }
            ));
            if i + 1 == self.children.len() {
                s.push_str("    }\n");
            } else {
                s.push_str("    },\n");
            }
        }
        s.push_str("  ],\n");
        s.push_str("  \"claims\": [\n");
        for (i, c) in self.claims.iter().enumerate() {
            s.push_str("    {\n");
            field_indent(&mut s, "parent_id", &c.parent_id, true, 6);
            field_indent(&mut s, "child_id", &c.child_id, true, 6);
            field_indent(&mut s, "peer", &c.peer, true, 6);
            s.push_str("      \"paths\": ");
            s.push_str(&json_str_array(&c.paths));
            s.push('\n');
            if i + 1 == self.claims.len() {
                s.push_str("    }\n");
            } else {
                s.push_str("    },\n");
            }
        }
        s.push_str("  ],\n");
        field(&mut s, "screen", &self.screen, true);
        field(&mut s, "image_rel", &self.image_rel, true);
        field(&mut s, "fingerprint", &self.fingerprint, false);
        s.push_str("}\n");
        s
    }
}

fn field(out: &mut String, key: &str, value: &str, comma: bool) {
    field_indent(out, key, value, comma, 2);
}

fn field_indent(out: &mut String, key: &str, value: &str, comma: bool, spaces: usize) {
    out.push_str(&" ".repeat(spaces));
    out.push('"');
    out.push_str(key);
    out.push_str("\": \"");
    out.push_str(&json_escape(value));
    out.push('"');
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn opt_field(out: &mut String, key: &str, value: Option<&str>, spaces: usize) {
    out.push_str(&" ".repeat(spaces));
    out.push('"');
    out.push_str(key);
    out.push_str("\": ");
    match value {
        Some(v) => {
            out.push('"');
            out.push_str(&json_escape(v));
            out.push('"');
        }
        None => out.push_str("null"),
    }
    out.push_str(",\n");
}

fn json_str_array(items: &[String]) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|s| format!("\"{}\"", json_escape(s)))
        .collect();
    format!("[{}]", parts.join(", "))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// True when the JSON text is the steer memory pack shape, not a live view.
pub fn json_is_memory_pack(text: &str) -> bool {
    let t = text.trim();
    MEMORY_PACK_KEYS
        .iter()
        .all(|k| t.contains(&format!("\"{k}\"")))
        && !t.contains(&format!("\"kind\": \"{KIND}\""))
}

/// True when the JSON text is this module's live view, not a memory pack.
pub fn json_is_live_view(text: &str) -> bool {
    let t = text.trim();
    t.contains(&format!("\"kind\": \"{KIND}\""))
        && t.contains(&format!("\"schema\": \"{SCHEMA}\""))
        && t.contains(&format!("\"to\": \"{STEER_TO}\""))
        && !json_is_memory_pack(t)
}

pub fn view_paths(store: &Path) -> Written {
    Written {
        view: store.join(VIEW_REL),
        screen: store.join(SCREEN_REL),
        image: store.join(IMAGE_REL),
    }
}

/// Capture what the floor files say right now.
///
/// Missing files → INFORMATION_GAP / WAIT, no invented parent or JOINED.
/// JOINED without a handoff is not shown as JOINED.
/// A complete snapshot is still not PASS.
pub fn capture(store: &Path) -> BossView {
    if is_forbidden_project(store) {
        return wait_view(
            EvidenceClass::Inconclusive,
            "FORBIDDEN: refused C:\\TextPCB Platform; no fake floor view",
        );
    }

    let floor = load_floor(store);
    let mut wait_notes: Vec<String> = Vec::new();
    let floor_present = store.join("floor").is_dir()
        || store.join("floor").join("current.json").is_file()
        || store.join("snapshot.json").is_file();

    if !floor_present
        && floor.current.is_none()
        && floor.children.is_empty()
        && floor.claims.is_empty()
    {
        return wait_view(
            EvidenceClass::InformationGap,
            "WAIT: no floor files; live view not invented",
        );
    }

    let current = floor.current.as_ref().map(|c| ViewCurrent {
        id: c.id.clone(),
        state: c.state.to_string(),
        title: c.title.clone(),
    });

    if current.is_none() {
        wait_notes.push("WAIT: no live parent on the floor".into());
    }

    let mut children = Vec::new();
    let mut fake_joined = 0usize;
    for c in &floor.children {
        let child = view_child(c);
        if child.joined_without_handoff {
            fake_joined += 1;
        }
        children.push(child);
    }
    if fake_joined > 0 {
        wait_notes.push(format!(
            "WAIT: {fake_joined} child(ren) marked JOINED without handoff; not shown as JOINED"
        ));
    }

    if let Some(cur) = &current {
        if cur.state == ParentState::Verified.to_string() {
            wait_notes.push(
                "WAIT: floor current says VERIFIED; a view is not verify authority".into(),
            );
        }
        if cur.state == ParentState::Closed.to_string() {
            wait_notes.push("WAIT: floor current says CLOSED; a view is not close authority".into());
        }
    }

    let claims = floor
        .claims
        .iter()
        .map(|c| ViewClaim {
            parent_id: c.parent_id.clone(),
            child_id: c.child_id.clone(),
            peer: c.peer.clone(),
            paths: c.paths.clone(),
        })
        .collect();

    let class = if !wait_notes.is_empty() {
        EvidenceClass::InformationGap
    } else {
        // Have a picture. Pictures are not PASS.
        EvidenceClass::Unknown
    };
    let wait = if wait_notes.is_empty() {
        None
    } else {
        Some(wait_notes.join("; "))
    };

    finish_view(current, children, claims, class, wait)
}

fn view_child(c: &FloorChild) -> ViewChild {
    let joined_without_handoff = c.state == ChildState::Joined && c.handoff.is_none();
    let state = if joined_without_handoff {
        ChildState::Assigned.to_string()
    } else {
        c.state.to_string()
    };
    ViewChild {
        id: c.id.clone(),
        parent_id: c.parent_id.clone(),
        peer: c.peer.clone(),
        paths: c.paths.clone(),
        state,
        bounce_reason: c.bounce_reason.clone(),
        handoff_hash: c.handoff.as_ref().map(|h| h.hash.clone()),
        joined_without_handoff,
    }
}

fn wait_view(class: EvidenceClass, reason: impl Into<String>) -> BossView {
    finish_view(None, Vec::new(), Vec::new(), class, Some(reason.into()))
}

fn finish_view(
    current: Option<ViewCurrent>,
    children: Vec<ViewChild>,
    claims: Vec<ViewClaim>,
    class: EvidenceClass,
    wait: Option<String>,
) -> BossView {
    let captured_at = format_rfc3339(unix_now());
    let mut view = BossView {
        schema: SCHEMA.into(),
        kind: KIND.into(),
        to: STEER_TO.into(),
        captured_at,
        status: EvidenceStatus::Wait,
        class,
        wait,
        current,
        children,
        claims,
        screen: String::new(),
        image_rel: IMAGE_REL.into(),
        fingerprint: String::new(),
    };
    view.screen = render_screen(&view);
    view.fingerprint = fingerprint(view.screen.as_bytes());
    view
}

/// Text screen SuperGrokHeavy can read without opening a steer JSON.
pub fn render_screen(view: &BossView) -> String {
    let mut out = String::from("SHOP FLOOR — LIVE VIEW\n");
    out.push_str(&format!("to: {}\n", view.to));
    out.push_str(&format!(
        "captured: {}\n",
        if view.captured_at.is_empty() {
            "-"
        } else {
            view.captured_at.as_str()
        }
    ));
    out.push_str(&format!(
        "status: {}  class: {}\n",
        view.status,
        view.class.as_str()
    ));
    match &view.current {
        None => out.push_str("PARENT  (none)\n"),
        Some(c) => out.push_str(&format!("PARENT  {}  {}  {}\n", c.id, c.state, c.title)),
    }
    out.push_str("CHILDREN\n");
    if view.children.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for c in &view.children {
            let flag = if c.joined_without_handoff {
                "  WAIT-no-handoff"
            } else {
                ""
            };
            let bounce = c
                .bounce_reason
                .as_deref()
                .map(|r| format!("  bounce={r}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  {}  {}  peer={}  paths={}{bounce}{flag}\n",
                c.id,
                c.state,
                c.peer,
                c.paths.join(","),
            ));
        }
    }
    out.push_str("CLAIMS\n");
    if view.claims.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for c in &view.claims {
            out.push_str(&format!(
                "  {}/{}  peer={}  paths={}\n",
                c.parent_id,
                c.child_id,
                c.peer,
                c.paths.join(",")
            ));
        }
    }
    if let Some(w) = &view.wait {
        out.push_str(&format!("{w}\n"));
    } else {
        out.push_str("live view is Evidence, not PASS\n");
    }
    out
}

/// Tiny PPM image of the same status. No extra crates.
pub fn render_ppm(view: &BossView) -> Vec<u8> {
    let mut pixels: Vec<(u8, u8, u8)> =
        vec![(20, 20, 28); (IMAGE_WIDTH * IMAGE_HEIGHT) as usize];
    let parent_color = match view.current.as_ref().map(|c| c.state.as_str()) {
        Some("HELD") => (200, 180, 40),
        Some("IN_FLIGHT") => (50, 110, 200),
        Some("REDUCE_READY") | Some("REDUCED") => (80, 80, 180),
        Some("VERIFY_WAIT") => (180, 120, 40),
        Some("VERIFIED") => (40, 160, 80),
        Some("CLOSED") => (70, 70, 78),
        _ => (80, 80, 88),
    };
    fill_rect(&mut pixels, 0, 0, IMAGE_WIDTH, 8, parent_color);

    let wait_strip = if view.wait.is_some() {
        (180, 80, 20)
    } else {
        (40, 90, 70)
    };
    fill_rect(&mut pixels, 0, 0, 4, IMAGE_HEIGHT, wait_strip);

    let usable = IMAGE_HEIGHT.saturating_sub(10);
    let n = view.children.len().max(1) as u32;
    let band = (usable / n).max(3);
    for (i, child) in view.children.iter().enumerate() {
        let y = 10 + i as u32 * band;
        if y >= IMAGE_HEIGHT {
            break;
        }
        let color = if child.joined_without_handoff {
            (180, 80, 20)
        } else {
            match child.state.as_str() {
                "ASSIGNED" => (210, 130, 40),
                "JOINED" => (40, 170, 90),
                "BOUNCED" => (190, 50, 50),
                _ => (90, 90, 100),
            }
        };
        fill_rect(
            &mut pixels,
            6,
            y,
            IMAGE_WIDTH.saturating_sub(8),
            band.saturating_sub(1).max(1),
            color,
        );
    }

    let mut out = format!("P3\n{IMAGE_WIDTH} {IMAGE_HEIGHT}\n255\n").into_bytes();
    for (i, (r, g, b)) in pixels.iter().enumerate() {
        out.extend(format!("{r} {g} {b}").into_bytes());
        if (i + 1) % IMAGE_WIDTH as usize == 0 {
            out.push(b'\n');
        } else {
            out.push(b' ');
        }
    }
    out
}

fn fill_rect(
    pixels: &mut [(u8, u8, u8)],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: (u8, u8, u8),
) {
    for yy in y..(y + h).min(IMAGE_HEIGHT) {
        for xx in x..(x + w).min(IMAGE_WIDTH) {
            let i = (yy * IMAGE_WIDTH + xx) as usize;
            if i < pixels.len() {
                pixels[i] = color;
            }
        }
    }
}

/// Write the snapshot + text screen + PPM under `<store>/boss/`.
/// Refuses Platform. Does not write a steer JSON or a memory pack.
pub fn write_view(store: &Path, view: &BossView) -> Result<Written> {
    refuse_platform(store)?;
    if is_forbidden_project(store) {
        return Err(ShopError::ForbiddenWrite(store.to_path_buf()));
    }
    let written = view_paths(store);
    if let Some(dir) = written.view.parent() {
        fs::create_dir_all(dir)?;
    }
    ensure_under(&written.view, &[store.to_path_buf()])?;
    ensure_under(&written.screen, &[store.to_path_buf()])?;
    ensure_under(&written.image, &[store.to_path_buf()])?;

    write_atomic(&written.view, view.to_json().as_bytes())?;
    write_atomic(&written.screen, view.screen.as_bytes())?;
    write_atomic(&written.image, &render_ppm(view))?;
    Ok(written)
}

pub fn capture_and_write(store: &Path) -> Result<(BossView, Written)> {
    let view = capture(store);
    if is_forbidden_project(store) {
        return Err(ShopError::ForbiddenWrite(store.to_path_buf()));
    }
    let written = write_view(store, &view)?;
    Ok((view, written))
}

/// Load a previously written view. Missing file is WAIT, not a fake view.
pub fn load_view(store: &Path) -> Result<BossView> {
    refuse_platform(store)?;
    let path = store.join(VIEW_REL);
    if !path.is_file() {
        return Err(ShopError::IncompleteEvidence(
            "WAIT: no live boss view on disk; not invented".into(),
        ));
    }
    let text = fs::read_to_string(&path)?;
    if !json_is_live_view(&text) {
        return Err(ShopError::IncompleteEvidence(
            "WAIT: on-disk file is not a live floor view".into(),
        ));
    }
    if text.contains("\"status\": \"PASS\"") || text.contains("\"class\": \"PASS\"") {
        return Err(ShopError::IncompleteEvidence(
            "WAIT: on-disk view claimed PASS; rejected".into(),
        ));
    }
    Ok(capture(store))
}
