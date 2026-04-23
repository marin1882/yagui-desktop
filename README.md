# Yagüi Desktop

Agente local del comerciante. Conecta tu inventario con las IAs del mundo.

## Qué hace

- Vive en la bandeja del sistema (verde = activo, rojo = detenido)
- Gestiona el tunnel de Cloudflare que conecta tu tienda con la red Yagüi
- Expone un servidor HTTP local en `:7331` para recibir configuración desde la app móvil
- Permite seleccionar la carpeta de inventario que el nodo-server indexará

## Instalación

Descarga el instalador para tu sistema desde [Releases](../../releases).

## Desarrollo

**Requisitos:** Rust 1.70+, Node.js 18+

### Linux

```bash
sudo apt-get install \
  libwebkit2gtk-4.1-dev \
  libayatana-appindicator3-dev \
  libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev \
  libssl-dev

npm install
npm run tauri dev
```

### macOS

```bash
# Xcode Command Line Tools requerido
npm install
npm run tauri dev
```

## Compilar instalador

```bash
npm run tauri build
```

## API HTTP local (puerto 7331)

| Método   | Ruta                 | Descripción                         |
|----------|----------------------|-------------------------------------|
| GET      | `/ping`              | Health check                        |
| GET      | `/estado`            | Estado del nodo                     |
| POST     | `/configurar`        | Configurar y arrancar (desde móvil) |
| POST     | `/configurar-manual` | Configurar manualmente              |
| DELETE   | `/configurar`        | Restablecer / borrar configuración  |
| POST     | `/carpeta`           | Establecer carpeta de inventario    |
