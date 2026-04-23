#![allow(dead_code)]
/// Gestiona el proceso hijo del nodo-server (Node.js).

use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::path::PathBuf;

pub type NodoHandle = Arc<Mutex<Option<Child>>>;

pub fn new_handle() -> NodoHandle {
    Arc::new(Mutex::new(None))
}

/// Ruta al directorio del nodo-server.
fn nodo_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join("yagui-nodo")
}

/// Ruta al config.json que usa el nodo-server.
fn config_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        dirs::home_dir().unwrap_or_default().join("yagui").join("config.json")
    } else {
        PathBuf::from("/etc/yagui/config.json")
    }
}

/// Arranca el nodo-server como proceso hijo.
pub fn arrancar(handle: &NodoHandle, api_key: &str, tunnel_url: &str) -> anyhow::Result<()> {
    let mut guard = handle.lock().unwrap();

    // Matar proceso anterior si lo hubiera
    if let Some(ref mut child) = *guard {
        let _ = child.kill();
    }

    let dir = nodo_dir();
    if !dir.exists() {
        return Err(anyhow::anyhow!(
            "nodo-server no instalado en {:?}",
            dir
        ));
    }

    let node_bin = which_node()?;
    let main_js = dir.join("index.js");

    if !main_js.exists() {
        return Err(anyhow::anyhow!("index.js no encontrado en {:?}", dir));
    }

    let config_file = config_path();

    let child = Command::new(&node_bin)
        .arg(&main_js)
        .env("CONFIG_PATH", &config_file)
        .env("API_KEY", api_key)
        .env("TUNNEL_URL", tunnel_url)
        .current_dir(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    log::info!("[nodo] arrancado pid={}", child.id());
    *guard = Some(child);
    Ok(())
}

/// Detiene el nodo-server.
pub fn detener(handle: &NodoHandle) {
    let mut guard = handle.lock().unwrap();
    if let Some(ref mut child) = *guard {
        let _ = child.kill();
        log::info!("[nodo] detenido");
    }
    *guard = None;
}

/// Comprueba si el proceso sigue vivo.
pub fn esta_corriendo(handle: &NodoHandle) -> bool {
    let mut guard = handle.lock().unwrap();
    match *guard {
        None => false,
        Some(ref mut child) => match child.try_wait() {
            Ok(None) => true,           // sigue corriendo
            Ok(Some(_)) | Err(_) => {   // terminó o error
                *guard = None;
                false
            }
        },
    }
}

fn which_node() -> anyhow::Result<PathBuf> {
    // Buscar en rutas habituales, incluyendo nvm
    let candidates = [
        dirs::home_dir()
            .unwrap_or_default()
            .join(".nvm/versions/node")
            .to_string_lossy()
            .into_owned(),
        "/usr/local/bin".to_string(),
        "/usr/bin".to_string(),
    ];

    // Intentar desde PATH primero
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let p = PathBuf::from(dir).join("node");
        if p.exists() {
            return Ok(p);
        }
    }

    // Buscar en nvm
    let nvm_root = dirs::home_dir().unwrap_or_default().join(".nvm/versions/node");
    if nvm_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&nvm_root) {
            let mut versions: Vec<_> = entries.flatten().collect();
            versions.sort_by_key(|e| e.file_name());
            // Usar la más reciente
            if let Some(latest) = versions.last() {
                let node = latest.path().join("bin/node");
                if node.exists() {
                    return Ok(node);
                }
            }
        }
    }

    for candidate in &candidates {
        let p = PathBuf::from(candidate).join("node");
        if p.exists() {
            return Ok(p);
        }
    }

    Err(anyhow::anyhow!("node no encontrado. Instala Node.js 18+"))
}
