/// Servidor HTTP local en :7331
///
/// GET    /ping              → { "status": "ok" }
/// GET    /estado            → { "estado": "running"|"stopped"|"error", ... }
/// POST   /configurar        → { api_key, tunnel_url, tunnel_token } → guarda y arranca
/// POST   /configurar-manual → igual que POST /configurar (para config sin móvil)
/// DELETE /configurar        → borra config, mata procesos, vuelve a Estado A
/// POST   /carpeta           → { carpeta: "/ruta" } → persiste carpeta de inventario

use axum::{
    extract::State,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};

use crate::{config, nodo};
use crate::installer;

#[derive(Clone)]
pub struct AppState {
    pub nodo_handle:    nodo::NodoHandle,
    pub tunnel_pid:     Arc<Mutex<Option<u32>>>,
    pub error_msg:      Arc<Mutex<Option<String>>>,
    pub inventory_path: Arc<Mutex<Option<String>>>,
    pub nodo_dir:       PathBuf,
}

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/ping",              get(ping))
        .route("/estado",            get(estado))
        .route("/configurar",        post(configurar).delete(reset))
        .route("/configurar-manual", post(configurar_manual))
        .route("/carpeta",           post(set_carpeta))
        .with_state(state)
        .layer(cors)
}

// ── /ping ─────────────────────────────────────────────────────────────────────

async fn ping() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

// ── /estado ───────────────────────────────────────────────────────────────────

async fn estado(State(state): State<AppState>) -> Json<Value> {
    let cfg            = config::load();
    let nodo_activo    = nodo::esta_corriendo(&state.nodo_handle);
    let tunnel_pid     = *state.tunnel_pid.lock().unwrap();
    let error_msg      = state.error_msg.lock().unwrap().clone();
    let inventory_path = state.inventory_path.lock().unwrap().clone();
    let configured     = cfg.is_complete();

    let estado_str = if let Some(ref msg) = error_msg {
        if msg.is_empty() { "stopped" } else { "error" }
    } else if nodo_activo {
        "running"
    } else if configured {
        "starting"
    } else {
        "stopped"
    };

    Json(json!({
        "estado":          estado_str,
        "corriendo":       nodo_activo,    // alias directo para el frontend
        "configured":      configured,
        "nodo_activo":     nodo_activo,
        "tunnel_pid":      tunnel_pid,
        "tunnel_url":      cfg.tunnel_url,
        "api_key":         cfg.api_key,
        "error":           error_msg,
        "inventory_path":  inventory_path,
    }))
}

// ── Lógica compartida de configurar ───────────────────────────────────────────

#[derive(Deserialize)]
struct ConfigurarBody {
    api_key:        String,
    tunnel_url:     String,
    tunnel_token:   String,
    inventory_path: Option<String>,
}

#[derive(Serialize)]
struct OkResp {
    ok:    bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn aplicar_config(
    state: &AppState,
    body: ConfigurarBody,
) -> Result<(), String> {
    if body.api_key.is_empty() || body.tunnel_url.is_empty() || body.tunnel_token.is_empty() {
        return Err("faltan campos".into());
    }

    let cfg = config::Config {
        api_key:      body.api_key.clone(),
        tunnel_url:   body.tunnel_url.clone(),
        tunnel_token: body.tunnel_token.clone(),
    };

    config::save(&cfg).map_err(|e| format!("error guardando config: {}", e))?;
    println!("[configurar] config guardada OK");

    if let Some(ref path) = body.inventory_path {
        *state.inventory_path.lock().unwrap() = Some(path.clone());
    }

    // Arrancar tunnel (await antes de cualquier Mutex para que el futuro sea Send)
    let tunnel_pid = installer::arrancar_tunnel_directo(&body.tunnel_token)
        .await
        .map(|child| {
            let pid = child.id();
            println!("[configurar] tunnel arrancado pid={}", pid);
            std::mem::forget(child);
            pid
        })
        .map_err(|e| {
            let msg = format!("tunnel error: {}", e);
            println!("[configurar] ERROR tunnel: {}", msg);
            msg
        })?;

    *state.tunnel_pid.lock().unwrap() = Some(tunnel_pid);

    // Instalar nodo-server si no existe
    nodo::instalar_si_falta(&state.nodo_dir)
        .await
        .map_err(|e| {
            let msg = format!("nodo install error: {}", e);
            println!("[configurar] ERROR nodo install: {}", msg);
            msg
        })?;

    // Arrancar nodo-server
    nodo::arrancar(
        &state.nodo_handle,
        &state.nodo_dir,
        &body.api_key,
        &body.tunnel_url,
        body.inventory_path.as_deref(),
    )
    .map_err(|e| {
        let msg = format!("nodo error: {}", e);
        println!("[configurar] ERROR nodo: {}", msg);
        msg
    })?;

    *state.error_msg.lock().unwrap() = None;
    println!("[configurar] completado OK");
    Ok(())
}

// ── POST /configurar ──────────────────────────────────────────────────────────

async fn configurar(
    State(state): State<AppState>,
    Json(body): Json<ConfigurarBody>,
) -> Json<OkResp> {
    println!(
        "[configurar] api_key={:?} tunnel_url={:?} token_len={}",
        body.api_key, body.tunnel_url, body.tunnel_token.len()
    );
    match aplicar_config(&state, body).await {
        Ok(())   => Json(OkResp { ok: true,  error: None }),
        Err(msg) => {
            *state.error_msg.lock().unwrap() = Some(msg.clone());
            Json(OkResp { ok: false, error: Some(msg) })
        }
    }
}

// ── POST /configurar-manual ───────────────────────────────────────────────────

async fn configurar_manual(
    State(state): State<AppState>,
    Json(body): Json<ConfigurarBody>,
) -> Json<OkResp> {
    println!(
        "[configurar-manual] api_key={:?} tunnel_url={:?} token_len={}",
        body.api_key, body.tunnel_url, body.tunnel_token.len()
    );
    match aplicar_config(&state, body).await {
        Ok(())   => Json(OkResp { ok: true,  error: None }),
        Err(msg) => {
            *state.error_msg.lock().unwrap() = Some(msg.clone());
            Json(OkResp { ok: false, error: Some(msg) })
        }
    }
}

// ── DELETE /configurar (reset) ────────────────────────────────────────────────

async fn reset(State(state): State<AppState>) -> Json<OkResp> {
    println!("[reset] iniciando restablecimiento de configuración");

    // 1. Matar nodo-server
    nodo::detener(&state.nodo_handle);
    println!("[reset] nodo-server detenido");

    // 2. Matar cloudflared por pid
    if let Some(pid) = state.tunnel_pid.lock().unwrap().take() {
        println!("[reset] matando cloudflared pid={}", pid);
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }

    // 3. Borrar config.json
    if let Some(path) = config::config_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
            println!("[reset] config.json borrado: {:?}", path);
        }
    }

    // 4. Limpiar estado en memoria
    *state.error_msg.lock().unwrap()      = None;
    *state.inventory_path.lock().unwrap() = None;

    println!("[reset] completado — volviendo a Estado A");
    Json(OkResp { ok: true, error: None })
}

// ── POST /carpeta ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CarpetaBody {
    carpeta: String,
}

async fn set_carpeta(
    State(state): State<AppState>,
    Json(body): Json<CarpetaBody>,
) -> Json<OkResp> {
    let carpeta = body.carpeta.clone();
    println!("[carpeta] nueva ruta: {:?}", carpeta);
    *state.inventory_path.lock().unwrap() = Some(carpeta.clone());

    // Relanzar nodo con el nuevo INVENTORY_PATH para que lo use de inmediato
    let cfg = config::load();
    if cfg.is_complete() {
        match nodo::arrancar(
            &state.nodo_handle,
            &state.nodo_dir,
            &cfg.api_key,
            &cfg.tunnel_url,
            Some(carpeta.as_str()),
        ) {
            Ok(())  => println!("[carpeta] nodo relanzado con INVENTORY_PATH={}", carpeta),
            Err(e)  => println!("[carpeta] error relanzando nodo: {}", e),
        }
    }

    Json(OkResp { ok: true, error: None })
}
