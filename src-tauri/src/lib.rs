mod config;
mod installer;
mod nodo;
mod server;
mod tray;

use std::sync::{Arc, Mutex};
use tauri::RunEvent;
use tauri_plugin_autostart::MacosLauncher;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let nodo_handle     = nodo::new_handle();
    let tunnel_pid      = Arc::new(Mutex::new(None::<u32>));
    let error_msg       = Arc::new(Mutex::new(None::<String>));
    let inventory_path  = Arc::new(Mutex::new(None::<String>));

    // Directorio donde se instala el nodo-server:
    //   Linux:   ~/.local/share/yagui/nodo-server
    //   macOS:   ~/Library/Application Support/yagui/nodo-server
    //   Windows: %LOCALAPPDATA%\yagui\nodo-server
    let nodo_dir = dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local").join("share"))
        .join("yagui")
        .join("nodo-server");

    let app_state = server::AppState {
        nodo_handle:    nodo_handle.clone(),
        tunnel_pid:     tunnel_pid.clone(),
        error_msg:      error_msg.clone(),
        inventory_path: inventory_path.clone(),
        nodo_dir:       nodo_dir.clone(),
    };

    // Clonar para mover al hilo del servidor HTTP
    let server_state = app_state.clone();

    // Clonar para el handler de "Salir" (necesita matar procesos antes de exit)
    let exit_tunnel_pid    = tunnel_pid.clone();
    let exit_nodo_handle   = nodo_handle.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // ── Ocultar de la barra de dock/taskbar ──────────────────────
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // ── Crear icono de bandeja ────────────────────────────────────
            let tray = tray::crear_tray(app.handle())?;
            let tray_handle = tray.clone();

            // ── Arrancar servidor HTTP en background ──────────────────────
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");

            let router = server::build_router(server_state.clone());

            std::thread::spawn(move || {
                rt.block_on(async move {
                    match tokio::net::TcpListener::bind("0.0.0.0:7331").await {
                        Ok(listener) => {
                            log::info!("[server] escuchando en 0.0.0.0:7331");
                            if let Err(e) = axum::serve(listener, router).await {
                                log::error!("[server] error: {}", e);
                            }
                        }
                        Err(e) => {
                            log::error!("[server] no se pudo bind :7331 — {}", e);
                        }
                    }
                });
            });

            // ── Cargar config y arrancar servicios si está configurado ────
            let cfg = config::load();
            if cfg.is_complete() {
                tray::actualizar_tray(&tray_handle, tray::EstadoTray::Iniciando);

                let nh    = server_state.nodo_handle.clone();
                let tray2 = tray_handle.clone();
                let err2  = server_state.error_msg.clone();
                let tpid  = server_state.tunnel_pid.clone();
                let cfg2  = cfg.clone();
                let ndir  = server_state.nodo_dir.clone();
                let inv   = server_state.inventory_path.clone();

                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio rt setup");

                    // Arrancar tunnel (instala cloudflared si falta)
                    match rt.block_on(installer::arrancar_tunnel_directo(&cfg2.tunnel_token)) {
                        Ok(child) => {
                            *tpid.lock().unwrap() = Some(child.id());
                            std::mem::forget(child);
                        }
                        Err(e) => {
                            log::error!("[setup] tunnel: {}", e);
                            *err2.lock().unwrap() = Some(e.to_string());
                            tray::actualizar_tray(&tray2, tray::EstadoTray::Error);
                            return;
                        }
                    }

                    // Dar tiempo al tunnel para establecerse
                    std::thread::sleep(std::time::Duration::from_secs(2));

                    // Instalar nodo-server si falta
                    if let Err(e) = rt.block_on(nodo::instalar_si_falta(&ndir)) {
                        log::error!("[setup] nodo install: {}", e);
                        *err2.lock().unwrap() = Some(e.to_string());
                        tray::actualizar_tray(&tray2, tray::EstadoTray::Error);
                        return;
                    }

                    // Arrancar nodo-server
                    let inv_inicial = inv.lock().unwrap().clone();
                    match nodo::arrancar(&nh, &ndir, &cfg2.api_key, &cfg2.tunnel_url, inv_inicial.as_deref()) {
                        Ok(()) => {
                            tray::actualizar_tray(&tray2, tray::EstadoTray::Corriendo);
                        }
                        Err(e) => {
                            log::error!("[setup] nodo: {}", e);
                            *err2.lock().unwrap() = Some(e.to_string());
                            tray::actualizar_tray(&tray2, tray::EstadoTray::Error);
                            return;
                        }
                    }

                    // ── Watcher: relanza el nodo si muere (p.ej. tras auto-update) ──
                    let watch_nh   = nh.clone();
                    let watch_ndir = ndir.clone();
                    let watch_cfg  = cfg2.clone();
                    let watch_inv  = inv.clone();
                    std::thread::spawn(move || {
                        loop {
                            std::thread::sleep(std::time::Duration::from_secs(10));
                            if !nodo::esta_corriendo(&watch_nh) {
                                log::info!("[nodo] proceso terminado, relanzando en 3 s...");
                                std::thread::sleep(std::time::Duration::from_secs(3));
                                let inv_path = watch_inv.lock().unwrap().clone();
                                match nodo::arrancar(
                                    &watch_nh,
                                    &watch_ndir,
                                    &watch_cfg.api_key,
                                    &watch_cfg.tunnel_url,
                                    inv_path.as_deref(),
                                ) {
                                    Ok(())  => log::info!("[nodo] relanzado OK"),
                                    Err(e)  => log::error!("[nodo] error al relanzar: {}", e),
                                }
                            }
                        }
                    });
                });
            } else {
                // No configurado → abrir ventana de setup
                tray::actualizar_tray(&tray_handle, tray::EstadoTray::Detenido);
                tray::abrir_ventana_setup(app.handle());
            }

            Ok(())
        })
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "abrir" => tray::abrir_ventana_setup(app),
            "salir" => {
                // Matar nodo-server
                nodo::detener(&exit_nodo_handle);

                // Matar cloudflared por PID
                if let Some(pid) = exit_tunnel_pid.lock().unwrap().take() {
                    let _ = std::process::Command::new("kill")
                        .args(["-TERM", &pid.to_string()])
                        .status();
                }

                // Salir directamente — app.exit(0) queda bloqueado por prevent_exit()
                std::process::exit(0);
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error construyendo la app")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                // Evitar que la app salga al cerrar la última ventana
                api.prevent_exit();
            }
            let _ = app; // evitar warning unused
        });
}
