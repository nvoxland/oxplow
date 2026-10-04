//! External-URL webview commands.

use crate::error::IpcError;

/// Open an external URL in a sandboxed `WebviewWindow`.
///
/// The new window inherits
/// the `external-url` capability defined in
/// `apps/desktop/src-tauri/capabilities/external-url.json`, which
/// grants zero oxplow commands and zero plugin permissions.
#[tauri::command]
#[specta::specta]
pub async fn open_external_url(app: tauri::AppHandle, url: String) -> Result<String, IpcError> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(IpcError::invalid(format!(
            "external URL must be http or https: {url}"
        )));
    }

    let parsed = tauri::Url::parse(&url).map_err(|e| IpcError::invalid(format!("bad URL: {e}")))?;

    // Label format must match the `ext-url-*` glob in
    // capabilities/external-url.json.
    let label = format!("ext-url-{}", uuid::Uuid::new_v4().simple());

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed))
        .title(format!("{} — Oxplow", url))
        .inner_size(1100.0, 800.0)
        .build()
        .map_err(|e| IpcError::internal(format!("create webview window: {e}")))?;

    Ok(label)
}

/// Read clipboard text via the OS. Routed through Rust so we don't
/// have to grant the renderer the broader clipboard plugin permission.
#[tauri::command]
#[specta::specta]
pub async fn clipboard_read_text(app: tauri::AppHandle) -> Result<String, IpcError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .read_text()
        .map_err(|e| IpcError::internal(format!("clipboard read: {e}")))
}
