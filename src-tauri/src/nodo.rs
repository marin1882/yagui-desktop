#![allow(dead_code)]
/// Gestiona el proceso hijo del nodo-server (Node.js).
/// El nodo-server se descarga de GitHub la primera vez que se necesita.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub type NodoHandle = Arc<Mutex<Option<Child>>>;

const NODO_ZIP_URL: &str =
    "https://github.com/marin1882/yagui-nodo/archive/refs/heads/master.zip";

pub fn new_handle() -> NodoHandle {
    Arc::new(Mutex::new(None))
}

// ── Instalación automática ────────────────────────────────────────────────────

/// Descarga e instala el nodo-server en `nodo_dir` si no existe ya.
pub async fn instalar_si_falta(nodo_dir: &Path) -> anyhow::Result<()> {
    let index_js = nodo_dir.join("index.js");
    if index_js.exists() {
        log::info!("[nodo] ya instalado en {:?}", nodo_dir);
        return Ok(());
    }

    log::info!("[nodo] descargando nodo-server desde GitHub...");
    let bytes = reqwest::get(NODO_ZIP_URL)
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    log::info!("[nodo] descargados {} bytes, extrayendo...", bytes.len());
    std::fs::create_dir_all(nodo_dir)?;

    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("zip abierto: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| anyhow::anyhow!("zip entry {}: {}", i, e))?;

        // El zip tiene un directorio raíz "yagui-nodo-master/" que eliminamos
        let raw_name = file.name().to_owned();
        let stripped = raw_name.splitn(2, '/').nth(1).unwrap_or("");
        if stripped.is_empty() {
            continue;
        }

        let out_path = nodo_dir.join(stripped);

        if file.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut dest = std::fs::File::create(&out_path)?;
            std::io::copy(&mut file, &mut dest)?;
        }
    }

    log::info!("[nodo] extracción completa, ejecutando npm install...");
    let npm = which_npm()?;
    let node_bin = which_node()?;
    let node_dir = node_bin
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();

    let status = Command::new(&npm)
        .arg("install")
        .arg("--omit=dev")
        .current_dir(nodo_dir)
        .env("PATH", format!("{}:{}", node_dir.display(), std::env::var("PATH").unwrap_or_default()))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| anyhow::anyhow!("npm install: {}", e))?;

    if !status.success() {
        return Err(anyhow::anyhow!("npm install falló con código {:?}", status.code()));
    }

    // Inicializar repo git para que index.js pueda hacer auto-update en el futuro
    log::info!("[nodo] inicializando repo git para auto-actualizaciones...");
    let git_init_ok = Command::new("git")
        .args(["init"])
        .current_dir(nodo_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if git_init_ok {
        let _ = Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/marin1882/yagui-nodo.git"])
            .current_dir(nodo_dir)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .args(["fetch", "origin", "master", "--depth=1"])
            .current_dir(nodo_dir)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status();
        let _ = Command::new("git")
            .args(["reset", "--hard", "origin/master"])
            .current_dir(nodo_dir)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status();
        log::info!("[nodo] repo git listo");
    } else {
        log::warn!("[nodo] git no disponible — auto-update desactivado hasta próxima instalación");
    }

    log::info!("[nodo] instalación completada en {:?}", nodo_dir);
    Ok(())
}

// ── Arrancar / detener ────────────────────────────────────────────────────────

/// Arranca el nodo-server como proceso hijo.
pub fn arrancar(
    handle: &NodoHandle,
    nodo_dir: &Path,
    api_key: &str,
    tunnel_url: &str,
) -> anyhow::Result<()> {
    let mut guard = handle.lock().unwrap();

    // Matar proceso anterior si lo hubiera
    if let Some(ref mut child) = *guard {
        let _ = child.kill();
    }

    if !nodo_dir.exists() {
        return Err(anyhow::anyhow!(
            "nodo-server no instalado en {:?}",
            nodo_dir
        ));
    }

    let main_js = nodo_dir.join("index.js");
    if !main_js.exists() {
        return Err(anyhow::anyhow!("index.js no encontrado en {:?}", nodo_dir));
    }

    let node_bin = which_node()?;

    let child = Command::new(&node_bin)
        .arg(&main_js)
        .env("API_KEY",           api_key)
        .env("TUNNEL_URL",        tunnel_url)
        .env("SUPABASE_URL",      "https://lbozfbvenchyafyihyso.supabase.co")
        .env("SUPABASE_ANON_KEY", "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imxib3pmYnZlbmNoeWFmeWloeXNvIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NDM3MDgwNjksImV4cCI6MjA1OTI4NDA2OX0.2hSK4EUdmGwUpqH0dGFzD3LN78H3MqJyImHTWHJinE0")
        .current_dir(nodo_dir)
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
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                *guard = None;
                false
            }
        },
    }
}

// ── Búsqueda de binarios ──────────────────────────────────────────────────────

fn which_node() -> anyhow::Result<PathBuf> {
    // PATH primero
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let p = PathBuf::from(dir).join("node");
        if p.exists() {
            return Ok(p);
        }
    }

    // nvm: usar la versión más reciente
    let nvm_root = dirs::home_dir().unwrap_or_default().join(".nvm/versions/node");
    if nvm_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&nvm_root) {
            let mut versions: Vec<_> = entries.flatten().collect();
            versions.sort_by_key(|e| e.file_name());
            if let Some(latest) = versions.last() {
                let node = latest.path().join("bin/node");
                if node.exists() {
                    return Ok(node);
                }
            }
        }
    }

    for dir in ["/usr/local/bin", "/usr/bin"] {
        let p = PathBuf::from(dir).join("node");
        if p.exists() {
            return Ok(p);
        }
    }

    Err(anyhow::anyhow!("node no encontrado. Instala Node.js 18+"))
}

fn which_npm() -> anyhow::Result<PathBuf> {
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let p = PathBuf::from(dir).join("npm");
        if p.exists() {
            return Ok(p);
        }
    }

    let nvm_root = dirs::home_dir().unwrap_or_default().join(".nvm/versions/node");
    if nvm_root.exists() {
        if let Ok(entries) = std::fs::read_dir(&nvm_root) {
            let mut versions: Vec<_> = entries.flatten().collect();
            versions.sort_by_key(|e| e.file_name());
            if let Some(latest) = versions.last() {
                let npm = latest.path().join("bin/npm");
                if npm.exists() {
                    return Ok(npm);
                }
            }
        }
    }

    for dir in ["/usr/local/bin", "/usr/bin"] {
        let p = PathBuf::from(dir).join("npm");
        if p.exists() {
            return Ok(p);
        }
    }

    Err(anyhow::anyhow!("npm no encontrado. Instala Node.js 18+"))
}
