//! Cursor-style project opener. A project is a folder; the store is `<project>/.shop/`.
//! Recents live in the user-level shop home so they survive per-project stores.
//! Never writes `C:\TextPCB Platform`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShopError};
use crate::hash::unix_now;
use crate::store::{ensure_under, write_json_atomic};

pub const RECENTS_CAP: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    #[serde(default)]
    pub github: Option<String>,
    #[serde(default)]
    pub opened_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recents {
    #[serde(default)]
    pub projects: Vec<RecentProject>,
}

pub fn shop_home() -> PathBuf {
    if let Ok(h) = env::var("SHOP_HOME") {
        let t = h.trim();
        if !t.is_empty() {
            return PathBuf::from(t);
        }
    }
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".shop")
}

pub fn default_recents_path() -> PathBuf {
    shop_home().join("recents.json")
}

pub fn project_root_from_store(store: &Path) -> PathBuf {
    if store.file_name().and_then(|s| s.to_str()) == Some(".shop") {
        store
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| store.to_path_buf())
    } else {
        store.to_path_buf()
    }
}

pub fn project_name(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

/// Exact Platform path, or any path that would write into it.
pub fn is_forbidden_project(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    let norm = raw.replace('/', "\\");
    let trimmed = norm.trim_end_matches('\\');
    trimmed.eq_ignore_ascii_case(r"C:\TextPCB Platform")
        || raw.contains("TextPCB Platform")
        || raw.contains("C:\\TextPCB")
}

pub fn refuse_platform(path: &Path) -> Result<()> {
    if is_forbidden_project(path) {
        return Err(ShopError::ForbiddenWrite(path.to_path_buf()));
    }
    Ok(())
}

/// Resolve the store directory for a project folder (or an existing `.shop` dir).
pub fn store_dir_for(path: &Path) -> Result<PathBuf> {
    refuse_platform(path)?;
    let path = path.to_path_buf();
    if path.is_file() {
        return Err(ShopError::InvalidId(format!(
            "open folder, not a file: {}",
            path.display()
        )));
    }
    if path.join("meta.json").is_file() || path.join("snapshot.json").is_file() {
        return Ok(path);
    }
    if path.file_name().and_then(|s| s.to_str()) == Some(".shop") {
        return Ok(path);
    }
    Ok(path.join(".shop"))
}

pub fn load_recents(path: &Path) -> Recents {
    let Ok(bytes) = fs::read(path) else {
        return Recents::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_recents(path: &Path, recents: &Recents) -> Result<()> {
    if let Some(parent) = path.parent() {
        refuse_platform(parent)?;
        fs::create_dir_all(parent)?;
        let _ = ensure_under(path, &[parent.to_path_buf()]);
    }
    write_json_atomic(path, recents)
}

pub fn record_recent(recents_path: &Path, project: &Path, github: Option<&str>) -> Result<()> {
    refuse_platform(project)?;
    refuse_platform(recents_path)?;
    let path_s = project.display().to_string();
    let mut recents = load_recents(recents_path);
    recents.projects.retain(|p| p.path != path_s);
    recents.projects.insert(
        0,
        RecentProject {
            path: path_s,
            name: project_name(project),
            github: github.map(str::to_string).filter(|s| !s.is_empty()),
            opened_at: unix_now(),
        },
    );
    recents.projects.truncate(RECENTS_CAP);
    save_recents(recents_path, &recents)
}

pub fn touch_recent_github(recents_path: &Path, project: &Path, slug: &str) -> Result<()> {
    let path_s = project.display().to_string();
    let mut recents = load_recents(recents_path);
    if let Some(p) = recents.projects.iter_mut().find(|p| p.path == path_s) {
        p.github = Some(slug.to_string());
        p.opened_at = unix_now();
    } else {
        recents.projects.insert(
            0,
            RecentProject {
                path: path_s,
                name: project_name(project),
                github: Some(slug.to_string()),
                opened_at: unix_now(),
            },
        );
        recents.projects.truncate(RECENTS_CAP);
    }
    save_recents(recents_path, &recents)
}

pub fn recent_path_for_repo(recents_path: &Path, slug: &str) -> Option<PathBuf> {
    load_recents(recents_path)
        .projects
        .into_iter()
        .find(|p| p.github.as_deref() == Some(slug))
        .map(|p| PathBuf::from(p.path))
}

/// Parse `origin` from `<project>/.git/config` when it is a GitHub remote.
pub fn origin_github_slug(project: &Path) -> Option<String> {
    let git = project.join(".git");
    let config = if git.is_file() {
        let text = fs::read_to_string(&git).ok()?;
        let dir = text
            .strip_prefix("gitdir:")
            .map(|s| s.trim())
            .map(PathBuf::from)?;
        let dir = if dir.is_absolute() {
            dir
        } else {
            project.join(dir)
        };
        dir.join("config")
    } else {
        git.join("config")
    };
    let text = fs::read_to_string(config).ok()?;
    let mut in_origin = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.eq_ignore_ascii_case("[remote \"origin\"]");
            continue;
        }
        if !in_origin {
            continue;
        }
        if let Some(url) = line
            .strip_prefix("url =")
            .or_else(|| line.strip_prefix("url="))
        {
            return parse_github_remote(url.trim());
        }
    }
    None
}

pub fn parse_github_remote(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let rest = rest.trim_start_matches('/');
    let (owner, name) = rest.split_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

pub fn slugs_from_github_search(v: &serde_json::Value) -> Vec<String> {
    let items = v
        .get("items")
        .and_then(|x| x.as_array())
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        let slug = it
            .get("full_name")
            .and_then(|x| x.as_str())
            .or_else(|| it.get("name").and_then(|x| x.as_str()))
            .unwrap_or("");
        if slug.contains('/') && !out.iter().any(|s| s == slug) {
            out.push(slug.to_string());
        }
    }
    out
}

pub fn format_recents(recents_path: &Path) -> String {
    let recents = load_recents(recents_path);
    if recents.projects.is_empty() {
        return "no recent projects\n".into();
    }
    let mut out = String::new();
    for p in &recents.projects {
        match &p.github {
            Some(g) => out.push_str(&format!("{}  {}  {g}\n", p.path, p.name)),
            None => out.push_str(&format!("{}  {}\n", p.path, p.name)),
        }
    }
    out
}
