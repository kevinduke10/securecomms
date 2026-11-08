fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "client_request",
            "desktop_info",
            "open_login",
            "save_download",
        ]),
    ))
    .expect("Tauri application configuration failed");
}
