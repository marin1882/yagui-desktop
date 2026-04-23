/// Gestión del icono de bandeja del sistema.
/// Estado verde  = nodo corriendo
/// Estado amarillo = iniciando / configurando
/// Estado rojo   = detenido o error

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Manager,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EstadoTray {
    Iniciando,
    Corriendo,
    Detenido,
    Error,
}

/// Icono PNG 32x32 verde (R=34 G=197 B=94 — Tailwind green-500)
fn icono_verde() -> Vec<u8> {
    icono_solido(34, 197, 94)
}

/// Icono PNG 32x32 amarillo (R=234 G=179 B=8 — Tailwind yellow-500)
fn icono_amarillo() -> Vec<u8> {
    icono_solido(234, 179, 8)
}

/// Icono PNG 32x32 rojo (R=239 G=68 B=68 — Tailwind red-500)
fn icono_rojo() -> Vec<u8> {
    icono_solido(239, 68, 68)
}

/// Genera un PNG de 32x32 de color sólido (RGBA) usando PNG crudo mínimo.
fn icono_solido(r: u8, g: u8, b: u8) -> Vec<u8> {
    // PNG con un pixel 1x1 escalable — usamos RGBA raw 32x32
    // Para evitar depender de una librería PNG en tiempo de compilación,
    // codificamos el PNG a mano con la estructura mínima.
    png_rgba_32x32(r, g, b, 255)
}

/// Genera un PNG RGBA 32×32 de color sólido sin librerías externas.
fn png_rgba_32x32(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
    const W: u32 = 32;
    const H: u32 = 32;

    // Scanline: filtro 0x00 + W píxeles RGBA
    let mut scanline = Vec::with_capacity(1 + (W as usize) * 4);
    scanline.push(0u8); // filter None
    for _ in 0..W {
        scanline.extend_from_slice(&[r, g, b, a]);
    }

    // Todos los scanlines iguales
    let mut raw_image: Vec<u8> = Vec::new();
    for _ in 0..H {
        raw_image.extend_from_slice(&scanline);
    }

    // Comprimir con zlib (deflate)
    let compressed = miniz_deflate(&raw_image);

    let mut out = Vec::new();

    // PNG signature
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    // IHDR chunk
    write_chunk(&mut out, b"IHDR", &{
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&W.to_be_bytes());
        ihdr.extend_from_slice(&H.to_be_bytes());
        ihdr.push(8);  // bit depth
        ihdr.push(6);  // RGBA color type
        ihdr.push(0);  // compression method
        ihdr.push(0);  // filter method
        ihdr.push(0);  // interlace
        ihdr
    });

    // IDAT chunk
    write_chunk(&mut out, b"IDAT", &compressed);

    // IEND chunk
    write_chunk(&mut out, b"IEND", &[]);

    out
}

fn write_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let crc = crc32_ieee(chunk_type, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// CRC-32 IEEE simple sin librerías externas.
fn crc32_ieee(prefix: &[u8], data: &[u8]) -> u32 {
    let table = crc32_table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in prefix.iter().chain(data.iter()) {
        crc = (crc >> 8) ^ table[((crc ^ (b as u32)) & 0xFF) as usize];
    }
    crc ^ 0xFFFF_FFFF
}

fn crc32_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for n in 0u32..256 {
        let mut c = n;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        table[n as usize] = c;
    }
    table
}

/// Compresión zlib mínima (nivel 0 — sin compresión).
fn miniz_deflate(data: &[u8]) -> Vec<u8> {
    // zlib header: CMF=0x78 (deflate, window 32KB), FLG calcula checksum
    // Usamos nivel 0 (sin compresión) para simplicidad
    let mut out = Vec::new();

    // zlib header bytes
    let cmf: u8 = 0x78;
    let flg: u8 = 0x9C; // 0x789C = sin compresión máxima pero válido
    // Verificar que (cmf * 256 + flg) % 31 == 0 → 0x789C % 31 = 0 ✓
    out.push(cmf);
    out.push(flg);

    // DEFLATE non-compressed blocks (BTYPE = 00)
    const BLOCK_MAX: usize = 65535;
    let mut pos = 0;
    while pos < data.len() {
        let end = (pos + BLOCK_MAX).min(data.len());
        let block = &data[pos..end];
        let is_last = end == data.len();

        out.push(if is_last { 0x01 } else { 0x00 }); // BFINAL | BTYPE=00
        let len = block.len() as u16;
        let nlen = !len;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(block);

        pos = end;
    }

    // Adler-32 checksum
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

// ── API pública ───────────────────────────────────────────────────────────────

pub fn crear_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let icono = Image::from_bytes(&icono_amarillo())?;

    let menu = crear_menu(app)?;

    TrayIconBuilder::new()
        .icon(icono)
        .menu(&menu)
        .tooltip("Yagüi — iniciando...")
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::TrayIconEvent;
            if let TrayIconEvent::Click { .. } = event {
                let app = tray.app_handle();
                abrir_ventana_setup(app);
            }
        })
        .build(app)
}

pub fn actualizar_tray(tray: &TrayIcon, estado: EstadoTray) {
    let (icono_bytes, tooltip) = match estado {
        EstadoTray::Corriendo   => (icono_verde(),    "Yagüi — activo"),
        EstadoTray::Iniciando   => (icono_amarillo(), "Yagüi — iniciando..."),
        EstadoTray::Detenido    => (icono_rojo(),     "Yagüi — detenido"),
        EstadoTray::Error       => (icono_rojo(),     "Yagüi — error"),
    };

    if let Ok(img) = Image::from_bytes(&icono_bytes) {
        let _ = tray.set_icon(Some(img));
    }
    let _ = tray.set_tooltip(Some(tooltip));
}

fn crear_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let abrir = MenuItem::with_id(app, "abrir",  "Abrir panel", true, None::<&str>)?;
    let sep   = tauri::menu::PredefinedMenuItem::separator(app)?;
    let salir = MenuItem::with_id(app, "salir",  "Salir",       true, None::<&str>)?;
    Menu::with_items(app, &[&abrir, &sep, &salir])
}

pub fn abrir_ventana_setup(app: &AppHandle) {
    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;

    if let Some(w) = app.get_webview_window("setup") {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }

    let _ = WebviewWindowBuilder::new(app, "setup", WebviewUrl::App("index.html".into()))
        .title("Yagüi — configuración")
        .inner_size(480.0, 540.0)
        .resizable(false)
        .center()
        .build();
}
