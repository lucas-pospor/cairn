//! App-level settings that are not part of any vault (recent vaults, plugin
//! approvals).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Most recent first.
    pub recent_vaults: Vec<String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl AppConfig {
    pub fn load(app: &AppHandle) -> AppConfig {
        let path = app.path().app_config_dir().ok().map(|d| d.join("config.json"));
        let mut cfg: AppConfig = path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        cfg.path = path;
        cfg
    }

    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(self) {
            Ok(b) => {
                if let Err(e) = std::fs::write(path, b) {
                    log::warn!("cannot save config: {e}");
                }
            }
            Err(e) => log::warn!("cannot serialize config: {e}"),
        }
    }

    pub fn touch_recent(&mut self, root: &str) {
        self.recent_vaults.retain(|p| p != root);
        self.recent_vaults.insert(0, root.to_string());
        self.recent_vaults.truncate(10);
        self.save();
    }

    pub fn forget(&mut self, root: &str) {
        self.recent_vaults.retain(|p| p != root);
        self.save();
    }
}

/// A plugin the user turned on in Settings > Plugins on this device: a hash of
/// the plugin file and the permissions the confirm dialog showed. A plugin a
/// vault's settings.json lists starts only with a matching approval, so a vault
/// from someone else cannot turn its plugins on. Approvals never live in the
/// vault: they are kept per device, by vault root, in `plugin-approvals.json`
/// in the app config folder (read on every use, so it is never stale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginApproval {
    pub hash: String,
    pub permissions: Vec<String>,
}

/// Vault root -> plugin file -> approval.
type Approvals = BTreeMap<String, BTreeMap<String, PluginApproval>>;

/// Serializes the read-modify-write of the approvals file.
static APPROVALS_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

fn approvals_path(app: &AppHandle) -> std::io::Result<PathBuf> {
    let dir = app.path().app_config_dir().map_err(std::io::Error::other)?;
    Ok(dir.join("plugin-approvals.json"))
}

/// A missing or unreadable file approves nothing.
fn read_approvals(path: &Path) -> Approvals {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            log::warn!("plugin approvals unreadable, all plugins are off until turned on again: {e}");
            Approvals::new()
        }),
        Err(_) => Approvals::new(),
    }
}

/// The plugins approved on this device for the vault at `root`, by file name.
pub fn plugin_approvals(app: &AppHandle, root: &str) -> std::io::Result<BTreeMap<String, PluginApproval>> {
    let path = approvals_path(app)?;
    let _lock = APPROVALS_LOCK.lock();
    Ok(read_approvals(&path).remove(root).unwrap_or_default())
}

/// Record (or with `None` remove) the approval of one plugin of the vault at `root`.
pub fn set_plugin_approval(app: &AppHandle, root: &str, file: &str, approval: Option<PluginApproval>) -> std::io::Result<()> {
    let path = approvals_path(app)?;
    let _lock = APPROVALS_LOCK.lock();
    let mut all = read_approvals(&path);
    let vault = all.entry(root.to_string()).or_default();
    match approval {
        Some(a) => {
            vault.insert(file.to_string(), a);
        }
        None => {
            vault.remove(file);
        }
    }
    if vault.is_empty() {
        all.remove(root);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write a temp file and rename it, so a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all)?)?;
    std::fs::rename(&tmp, &path)
}
