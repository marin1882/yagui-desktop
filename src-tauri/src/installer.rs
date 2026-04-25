#![allow(dead_code)]
/// Instala cloudflared si no está disponible y gestiona el tunnel.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
const CF_URL_AMD64: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz";
#[cfg(target_os = "macos")]
const CF_URL_ARM64: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz";

#[cfg(target_os = "linux")]
const CF_URL_AMD64: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64";
#[cfg(target_os = "linux")]
const CF_URL_ARM64: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64";

/// Ruta local donde guardamos cloudflared (~/.yagui/cloudflared).
fn local_bin() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".yagui")
        .join("cloudflared")
}

/// Busca cloudflared en PATH del sistema.
fn which_cloudflared() -> Option<PathBuf> {
    std::env::var("PATH").ok()?.split(':').find_map(|dir| {
        let p = PathBuf::from(dir).join("cloudflared");
        p.exists().then_some(p)
    })
}

/// Devuelve la ruta al binario cloudflared disponible.
/// Prioridad: local (~/.yagui/) → PATH del sistema.
pub fn cloudflared_bin() -> PathBuf {
    let local = local_bin();
    if local.exists() {
        return local;
    }
    which_cloudflared().unwrap_or(local)
}

pub fn is_installed() -> bool {
    local_bin().exists() || which_cloudflared().is_some()
}

/// Descarga cloudflared si no existe. Bloquea hasta que el archivo
/// está escrito en disco y con permisos de ejecución.
/// Devuelve la ruta al binario listo para usar.
pub async fn instalar_si_falta() -> anyhow::Result<PathBuf> {
    let dest = local_bin();

    // Log 1: estado antes de decidir
    println!(
        "[installer] buscando cloudflared en: {:?}",
        dest
    );
    println!("[installer] existe local: {}", dest.exists());

    if let Some(sys) = which_cloudflared() {
        println!("[installer] encontrado en PATH: {:?}", sys);
    }

    if dest.exists() {
        println!("[installer] usando binario existente: {:?}", dest);
        return Ok(dest);
    }

    // Crear directorio destino
    let dest_dir = dest.parent().unwrap();
    std::fs::create_dir_all(dest_dir)?;

    let arch = std::env::consts::ARCH;
    let url = if arch.contains("arm") || arch.contains("aarch64") {
        CF_URL_ARM64
    } else {
        CF_URL_AMD64
    };

    println!("[installer] descargando cloudflared desde {}", url);

    let resp = reqwest::get(url).await?;
    let status = resp.status();
    println!("[installer] respuesta HTTP: {}", status);
    if !status.is_success() {
        return Err(anyhow::anyhow!("descarga fallida: HTTP {}", status));
    }

    let bytes = resp.bytes().await?;

    // Log 4: tamaño descargado
    println!("[installer] descarga completada, tamaño: {} bytes", bytes.len());

    #[cfg(target_os = "linux")]
    {
        std::fs::write(&dest, &bytes)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
        println!("[installer] binario escrito y ejecutable: {:?}", dest);
    }

    #[cfg(target_os = "macos")]
    {
        use std::io::Cursor;
        let gz = flate2::read::GzDecoder::new(Cursor::new(&bytes));
        let mut archive = tar::Archive::new(gz);
        archive.unpack(dest_dir)?;
        if !dest.exists() {
            return Err(anyhow::anyhow!("cloudflared no encontrado tras descomprimir en {:?}", dest_dir));
        }
        println!("[installer] descomprimido en: {:?}", dest);
    }

    // Verificación final
    if !dest.exists() {
        return Err(anyhow::anyhow!("binario no existe tras instalación: {:?}", dest));
    }
    println!("[installer] cloudflared listo en {:?}", dest);
    Ok(dest)
}

// ── Gestión del tunnel ────────────────────────────────────────────────────────

/// Instala el tunnel como servicio del sistema (requiere privilegios).
pub fn instalar_servicio_tunnel(token: &str) -> std::io::Result<std::process::ExitStatus> {
    let bin = cloudflared_bin();
    println!("[installer] instalando servicio con bin: {:?}", bin);
    std::process::Command::new(bin)
        .args(["service", "install", "--token", token])
        .status()
}

/// Arranca el tunnel directamente como proceso hijo (sin privilegios de admin).
/// Llama a `instalar_si_falta()` internamente para garantizar que el binario existe.
pub async fn arrancar_tunnel_directo(token: &str) -> anyhow::Result<std::process::Child> {
    // Matar procesos cloudflared anteriores para evitar acumulación
    let _ = std::process::Command::new("pkill")
        .args(["-f", "cloudflared"])
        .status();
    std::thread::sleep(std::time::Duration::from_millis(500));

    // a) Garantizar que cloudflared existe ANTES de intentar ejecutarlo
    let bin = instalar_si_falta().await?;

    println!("[installer] arrancando tunnel con: {:?}", bin);
    println!("[installer] binario existe: {}", bin.exists());

    let child = std::process::Command::new(&bin)
        .args(["tunnel", "run", "--token", token])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("spawn cloudflared {:?}: {}", bin, e))?;

    println!("[installer] tunnel arrancado, pid={}", child.id());
    Ok(child)
}
