//! Caley's app shell: a server picker (`ui/`) that opens the chosen Caley server in the app's webview.
//!
//! The shell loads the server's own pages: the same origin as in a browser, so the session cookie, the
//! embedded data, swaps, the event stream, OIDC and the CSP work as they do there. The picker is the
//! only page of the app's own; the servers' pages get no IPC (no capability lists a remote origin).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A Caley server found at an address: its origin, which the picker opens.
#[derive(Serialize)]
struct Server {
    origin: String,
}

/// Why an address is not a Caley server the shell can open (`error`, for the picker's message).
#[derive(Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum ProbeError {
    /// Not an http(s) URL with a host.
    InvalidUrl,
    /// No answer (DNS, TLS, connection, timeout).
    Unreachable { detail: String },
    /// An answer, but not Caley's `GET /api/auth/config`.
    NotCaley,
}

/// What Caley's public `GET /api/auth/config` answers (backend/src/oidc.rs).
#[derive(Deserialize)]
struct AuthConfig {
    #[allow(dead_code)]
    oidc_enabled: bool,
}

/// TLS for the probe as the backend does it (ring, the bundled webpki roots): no system store to load, no
/// platform verifier to set up over JNI on Android. A server behind a private CA fails the probe; Android's
/// WebView would not trust a user-installed CA either.
fn tls_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// The origin an address typed in the picker names: `https://` unless it gives a scheme, path and query
/// dropped (Caley is served at its origin's root).
fn origin_of(address: &str) -> Result<url::Url, ProbeError> {
    let address = address.trim();
    let with_scheme = if address.contains("://") {
        address.to_owned()
    } else {
        format!("https://{address}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| ProbeError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err(ProbeError::InvalidUrl);
    }
    let origin = url.origin().ascii_serialization();
    url::Url::parse(&origin).map_err(|_| ProbeError::InvalidUrl)
}

/// Checks that a Caley server answers at `address` and returns its origin: the one the answer came from,
/// so a server that redirects `http://` to `https://` is opened over https. Asked from Rust because the
/// picker's page may not read another origin's response (Caley sends no CORS headers).
#[tauri::command]
async fn probe_server(address: String) -> Result<Server, ProbeError> {
    let origin = origin_of(&address)?;
    let config_url = origin.join("/api/auth/config").map_err(|_| ProbeError::InvalidUrl)?;
    let unreachable = |e: reqwest::Error| ProbeError::Unreachable {
        detail: e.to_string(),
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .use_preconfigured_tls(tls_config())
        .build()
        .map_err(unreachable)?;
    let response = client.get(config_url).send().await.map_err(unreachable)?;
    let final_origin = response.url().origin().ascii_serialization();
    if !response.status().is_success() {
        return Err(ProbeError::NotCaley);
    }
    response
        .json::<AuthConfig>()
        .await
        .map_err(|_| ProbeError::NotCaley)?;
    Ok(Server {
        origin: final_origin,
    })
}

/// Desktop: a menu with "Switch server" (back to the picker) and the edit commands, which macOS's
/// shortcuts (copy, paste) need once the app has a menu of its own. Android goes back to the picker with
/// its Back button (MainActivity.kt).
#[cfg(desktop)]
fn add_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::Manager;
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};

    let switch = MenuItemBuilder::with_id("switch-server", "Switch server…")
        .accelerator("CmdOrCtrl+Shift+S")
        .build(app)?;
    let server = SubmenuBuilder::new(app, "Caley")
        .item(&switch)
        .separator()
        .quit()
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    app.set_menu(MenuBuilder::new(app).items(&[&server, &edit]).build()?)?;

    // the picker's URL, which the window has before it opens a server; `?pick`: to choose, not to open the
    // server used last (ui/picker.js)
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let mut picker = window.url()?;
    picker.set_query(Some("pick"));
    app.on_menu_event(move |app, event| {
        if event.id() == "switch-server" {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.navigate(picker.clone());
            }
        }
    });
    Ok(())
}

/// Desktop: the window in Caley's page background (temper's `--tm-bg`, light or dark as the system is)
/// before the webview first paints, which takes WebView2 most of a second; then shown (it is made hidden,
/// tauri.conf.json), so it never shows white.
#[cfg(desktop)]
fn show_window(app: &tauri::App) -> tauri::Result<()> {
    use tauri::Manager;
    use tauri::window::Color;

    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let background = match window.theme().unwrap_or(tauri::Theme::Light) {
        tauri::Theme::Dark => Color(0x09, 0x0a, 0x0e, 0xff),
        _ => Color(0xf8, 0xfa, 0xff, 0xff),
    };
    window.set_background_color(Some(background))?;
    window.show()
}

/// Desktop: notifications allowed for the server's pages. WebView2 denies them at once without a handler
/// (no prompt), so Caley's notification switch could not be turned on; whether Caley notifies stays its
/// own setting, as in a browser that allowed the site. Every other permission is the webview's default.
#[cfg(desktop)]
fn permission(
    webview: tauri::Webview,
    kind: tauri::webview::PermissionKind,
) -> tauri::webview::PermissionResponse {
    use tauri::webview::{PermissionKind, PermissionResponse};
    let from_server = webview
        .url()
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str() != Some("tauri.localhost"));
    match kind {
        PermissionKind::Notifications if from_server => PermissionResponse::Allow,
        _ => PermissionResponse::Default,
    }
}

/// The window, from its config (`create: false` there: made here, with what the config cannot give it). On
/// desktop a window a server's page opens on another site (a link in a mail: `target="_blank"`) is the
/// system's browser instead; one on the page's own origin (the standalone message view) is the webview's.
/// The page's origin is the one the webview last navigated to. Android opens no windows (its links with a
/// target go to the browser from res/raw/shell_bridge.js).
fn main_window(app: &tauri::App) -> tauri::Result<()> {
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else {
        return Ok(());
    };
    let builder = tauri::WebviewWindowBuilder::from_config(app, &config)?;
    #[cfg(desktop)]
    let builder = {
        use std::sync::{Arc, Mutex, PoisonError};
        use tauri::webview::NewWindowResponse;

        let page = Arc::new(Mutex::new(None::<url::Origin>));
        let navigated = Arc::clone(&page);
        builder
            .on_navigation(move |url| {
                *navigated.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.origin());
                true
            })
            .on_new_window(move |url, _| {
                let same_origin = page.lock().unwrap_or_else(PoisonError::into_inner).as_ref() == Some(&url.origin());
                if matches!(url.scheme(), "http" | "https") && !same_origin {
                    open_in_browser(&url);
                    NewWindowResponse::Deny
                } else {
                    NewWindowResponse::Allow
                }
            })
    };
    builder.build()?;
    Ok(())
}

/// Desktop: `url` in the system's browser, its own program per system given the URL as its one argument
/// (no shell reads it).
#[cfg(desktop)]
fn open_in_browser(url: &url::Url) {
    use std::process::Command;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    if let Ok(mut child) = command.arg(url.as_str()).spawn() {
        std::thread::spawn(move || child.wait());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.on_permission_request(permission);
    builder
        .setup(|app| {
            main_window(app)?;
            #[cfg(desktop)]
            {
                add_menu(app)?;
                show_window(app)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![probe_server])
        .run(tauri::generate_context!())
        .expect("error while running Caley");
}

#[cfg(test)]
mod tests {
    use super::origin_of;

    fn origin(address: &str) -> Option<String> {
        origin_of(address).ok().map(|u| u.origin().ascii_serialization())
    }

    #[test]
    fn origins_of_addresses() {
        assert_eq!(origin("mail.example.com").as_deref(), Some("https://mail.example.com"));
        assert_eq!(origin(" https://mail.example.com/en/calendar/?x=1 ").as_deref(), Some("https://mail.example.com"));
        assert_eq!(origin("http://192.168.1.5:3000").as_deref(), Some("http://192.168.1.5:3000"));
        assert_eq!(origin("ftp://example.com"), None);
        assert_eq!(origin("https://"), None);
        assert_eq!(origin(""), None);
    }
}
