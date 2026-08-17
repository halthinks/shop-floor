//! Connected-repo record. Not a clone-the-world client.

use serde::{Deserialize, Serialize};

use crate::error::{Result, ShopError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepo {
    pub owner: String,
    pub name: String,
    #[serde(default)]
    pub default_branch: Option<String>,
}

impl GitHubRepo {
    pub fn parse(spec: &str, default_branch: Option<&str>) -> Result<Self> {
        let spec = spec.trim().trim_matches('/');
        let (owner, name) = spec
            .split_once('/')
            .ok_or_else(|| ShopError::InvalidId(format!("expected owner/name, got '{spec}'")))?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return Err(ShopError::InvalidId(spec.to_string()));
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
            default_branch: default_branch
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string),
        })
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

pub fn child_branch_name(parent_id: &str, child_id: &str) -> String {
    format!("shop/{parent_id}/{child_id}")
}
