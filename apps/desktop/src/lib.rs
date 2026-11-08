mod downloads;
mod secret_store;

use cirra_device::DeviceRuntime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tauri::{Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::Mutex;

struct ClientState(Mutex<DeviceRuntime>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientRequest {
    method: String,
    path: String,
    body: Option<Value>,
}

#[derive(Serialize)]
struct ClientResponse {
    status: u16,
    body: Value,
}

// This only checks the transport envelope. The device engine applies its exact route
// allowlist and authorization before touching state or making relay requests.
fn validate_request(request: &ClientRequest) -> Result<(), String> {
    if !matches!(request.method.as_str(), "GET" | "POST") {
        return Err("Unsupported client method".into());
    }
    let path = &request.path;
    if path.len() > 2048
        || !["/local-api/", "/local-auth/", "/desktop/"]
            .iter()
            .any(|prefix| path.starts_with(prefix))
        || path.contains("..")
        || path.contains('%')
        || path.contains('\\')
        || path.contains('#')
        || path.bytes().any(|c| c.is_ascii_control())
        || path.starts_with("/desktop/auth/start")
    {
        return Err("Unsupported client path".into());
    }
    if request.body.as_ref().is_some_and(|body| {
        serde_json::to_vec(body).map_or(true, |bytes| bytes.len() > 40 * 1024 * 1024)
    }) {
        return Err("Client request is too large".into());
    }
    Ok(())
}

fn require_main(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main" || !window.url().is_ok_and(|url| bundled_navigation(&url)) {
        return Err("Client commands are restricted to the main window".into());
    }
    Ok(())
}

#[tauri::command]
async fn client_request(
    window: WebviewWindow,
    state: State<'_, ClientState>,
    request: ClientRequest,
) -> Result<ClientResponse, String> {
    require_main(&window)?;
    validate_request(&request)?;
    let response = state
        .0
        .lock()
        .await
        .request(&request.method, &request.path, request.body)
        .await;
    Ok(ClientResponse {
        status: response.status,
        body: response.body,
    })
}

#[tauri::command]
fn desktop_info(window: WebviewWindow) -> Result<Value, String> {
    require_main(&window)?;
    Ok(
        serde_json::json!({"platform": std::env::consts::OS, "version": env!("CARGO_PKG_VERSION"), "secureStorage": "macOS Keychain"}),
    )
}

/// No URL supplied by the renderer is ever passed to an OS opener.
#[tauri::command]
async fn open_login(window: WebviewWindow, state: State<'_, ClientState>) -> Result<(), String> {
    require_main(&window)?;
    let mut runtime = state.0.lock().await;
    let response = runtime.request("POST", "/desktop/auth/start", None).await;
    let url = response
        .body
        .get("authorize_url")
        .and_then(Value::as_str)
        .filter(|url| response.status == 200 && runtime.is_authorization_url(url))
        .ok_or_else(|| "Unable to start account login".to_string())?;
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("--")
            .arg(url)
            .status()
            .map_err(|_| "Unable to open the login browser".to_string())?;
        if !status.success() {
            return Err("Unable to open the login browser".into());
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        Err("Browser login requires the supported macOS desktop build".into())
    }
}

/// Only an explicit OS save panel can select a destination; no path argument exists.
#[tauri::command]
async fn save_download(
    window: WebviewWindow,
    filename: String,
    data: String,
) -> Result<bool, String> {
    require_main(&window)?;
    downloads::validate_input(&filename, data.len())?;
    tauri::async_runtime::spawn_blocking(move || downloads::save(filename, data))
        .await
        .map_err(|_| "Unable to complete file export".to_string())?
}

fn bundled_navigation(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "tauri" | "http" | "https")
        && matches!(url.host_str(), Some("localhost") | Some("tauri.localhost"))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"))
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let runtime =
                DeviceRuntime::open(data_dir, Arc::new(secret_store::OsSecretStore::default()))?;
            app.manage(ClientState(Mutex::new(runtime)));
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Cirra")
                .inner_size(1180.0, 820.0)
                .min_inner_size(760.0, 560.0)
                .on_navigation(bundled_navigation)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            client_request,
            desktop_info,
            open_login,
            save_download
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Cirra desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_arbitrary_transport_and_login_url_bypass() {
        for path in [
            "https://relay.example/",
            "file:///etc/passwd",
            "/local-api/../secret",
            "/local-api/%2e%2e/secret",
            "/local-api/x\\y",
            "/desktop/auth/start",
            "/desktop/auth/start#x",
            "/unknown",
        ] {
            assert!(
                validate_request(&ClientRequest {
                    method: "GET".into(),
                    path: path.into(),
                    body: None
                })
                .is_err(),
                "{path}"
            );
        }
        assert!(
            validate_request(&ClientRequest {
                method: "DELETE".into(),
                path: "/local-api/state".into(),
                body: None
            })
            .is_err()
        );
        assert!(
            validate_request(&ClientRequest {
                method: "GET".into(),
                path: "/local-auth/me".into(),
                body: None
            })
            .is_ok()
        );
    }

    #[test]
    fn only_bundled_origin_can_navigate() {
        for url in [
            "https://cirra.example",
            "http://localhost:3000",
            "file:///tmp/index.html",
            "https://tauri.localhost.evil.example",
            "https://user@tauri.localhost",
        ] {
            assert!(!bundled_navigation(&url.parse().unwrap()), "{url}");
        }
        assert!(bundled_navigation(
            &"tauri://localhost/index.html".parse().unwrap()
        ));
        assert!(bundled_navigation(
            &"http://tauri.localhost/index.html".parse().unwrap()
        ));
    }
}
