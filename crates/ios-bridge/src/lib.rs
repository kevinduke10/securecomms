//! Narrow C ABI for the signed iOS shell. Each handle is confined to one serial
//! native queue. No JavaScript command accepts a filesystem path or OAuth token.
use anyhow::{Context, Result, ensure};
use cirra_device::{DeviceRuntime, SecretStore, oauth::PlatformLogin};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    path::PathBuf,
    sync::Arc,
};
use zeroize::Zeroizing;
const MAX_REQUEST: usize = 40 * 1024 * 1024;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeychainCallbacks {
    pub get: extern "C" fn(*const c_char, *mut u8, usize, *mut usize) -> i32,
    pub set: extern "C" fn(*const c_char, *const u8, usize) -> i32,
    pub delete: extern "C" fn(*const c_char) -> i32,
}
struct Keychain(KeychainCallbacks);
impl SecretStore for Keychain {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let name = CString::new(name)?;
        let mut bytes = Zeroizing::new(vec![0; 32]);
        let mut length = 0;
        match (self.0.get)(name.as_ptr(), bytes.as_mut_ptr(), bytes.len(), &mut length) {
            1 => Ok(None),
            0 => {
                ensure!(length == 32, "Invalid Keychain vault key");
                Ok(Some(bytes))
            }
            _ => anyhow::bail!("Unable to access iOS Keychain. Existing data was preserved."),
        }
    }
    fn set(&self, name: &str, secret: &[u8]) -> Result<()> {
        ensure!(secret.len() == 32, "Invalid vault key size");
        let name = CString::new(name)?;
        ensure!(
            (self.0.set)(name.as_ptr(), secret.as_ptr(), secret.len()) == 0,
            "Unable to save iOS Keychain credential"
        );
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        let name = CString::new(name)?;
        ensure!(
            (self.0.delete)(name.as_ptr()) == 0,
            "Unable to delete iOS Keychain credential"
        );
        Ok(())
    }
}
pub struct IosEngine {
    runtime: tokio::runtime::Runtime,
    device: DeviceRuntime,
    login: Option<PlatformLogin>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    method: String,
    path: String,
    body: Option<Value>,
}
fn validate_request(input: &str) -> Result<Request> {
    ensure!(input.len() <= MAX_REQUEST, "Client request is too large");
    let request: Request = serde_json::from_str(input)?;
    ensure!(
        matches!(request.method.as_str(), "GET" | "POST")
            && request.path.len() <= 2048
            && ["/local-api/", "/local-auth/", "/desktop/"]
                .iter()
                .any(|prefix| request.path.starts_with(prefix))
            && !request.path.starts_with("/desktop/auth/start")
            && !request.path.contains(['%', '\\', '#'])
            && !request.path.contains("..")
            && !request.path.bytes().any(|b| b.is_ascii_control()),
        "Unsupported native client route"
    );
    Ok(request)
}
fn output(result: Result<Value>) -> *mut c_char {
    let value = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error.to_string()}),
    };
    CString::new(value.to_string())
        .expect("JSON cannot contain NUL bytes")
        .into_raw()
}
unsafe fn input<'a>(value: *const c_char) -> Result<&'a str> {
    ensure!(!value.is_null(), "Missing native request");
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .context("Invalid UTF-8 request")
}
/// ABI version checked by the native shell before opening a handle.
#[unsafe(no_mangle)]
pub extern "C" fn cirra_ios_abi_version() -> u32 {
    1
}
/// # Safety
/// `directory` must be a valid NUL-terminated string. The callbacks remain valid
/// for the lifetime of the handle. Use this handle only on its owning serial queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_open(
    directory: *const c_char,
    callbacks: KeychainCallbacks,
    error: *mut *mut c_char,
) -> *mut IosEngine {
    let result = (|| -> Result<IosEngine> {
        let directory = PathBuf::from(unsafe { input(directory) }?);
        let runtime = tokio::runtime::Runtime::new()?;
        let device = DeviceRuntime::open(directory, Arc::new(Keychain(callbacks)))?;
        Ok(IosEngine {
            runtime,
            device,
            login: None,
        })
    })();
    match result {
        Ok(engine) => Box::into_raw(Box::new(engine)),
        Err(failure) => {
            if !error.is_null() {
                unsafe {
                    *error = output(Err(failure));
                }
            }
            std::ptr::null_mut()
        }
    }
}
/// # Safety
/// `engine` must be an open handle on its owning queue; `request` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_request(
    engine: *mut IosEngine,
    request: *const c_char,
) -> *mut c_char {
    output((|| {
        let engine = unsafe { engine.as_mut() }.context("Device engine is closed")?;
        let request = validate_request(unsafe { input(request) }?)?;
        if request.path == "/local-auth/logout" {
            engine.login = None;
        }
        let result = engine.runtime.block_on(engine.device.request(
            &request.method,
            &request.path,
            request.body,
        ));
        Ok(serde_json::to_value(result)?)
    })())
}
/// # Safety
/// The returned URL is for the native ASWebAuthenticationSession only, never a
/// renderer-supplied opener. `engine` must be valid and confined to its queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_begin_login(engine: *mut IosEngine) -> *mut c_char {
    output((|| {
        let engine = unsafe { engine.as_mut() }.context("Device engine is closed")?;
        engine.login = None;
        let login = engine
            .runtime
            .block_on(engine.device.begin_platform_login())?;
        let value = json!({"authorize_url":login.authorize_url});
        engine.login = Some(login);
        Ok(value)
    })())
}
/// # Safety
/// Only pass the callback received by the active native authentication session.
/// The handle must be valid and confined to its owning queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_complete_login(
    engine: *mut IosEngine,
    callback: *const c_char,
) -> *mut c_char {
    output((|| {
        let engine = unsafe { engine.as_mut() }.context("Device engine is closed")?;
        let callback = Zeroizing::new(unsafe { input(callback) }?.to_owned());
        let mut flow = engine.login.take().context("No pending login")?;
        let tokens = engine.runtime.block_on(flow.complete(&callback))?;
        engine
            .runtime
            .block_on(engine.device.complete_platform_login(tokens))?;
        Ok(json!({}))
    })())
}
/// # Safety
/// `engine` is either null or a valid handle on its owning queue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_cancel_login(engine: *mut IosEngine) {
    if let Some(engine) = unsafe { engine.as_mut() } {
        engine.login = None;
    }
}
/// # Safety
/// Close once, after all queued native requests finish; never reuse this handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_close(engine: *mut IosEngine) {
    if !engine.is_null() {
        drop(unsafe { Box::from_raw(engine) });
    }
}
/// # Safety
/// Free each C string returned by this library exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cirra_ios_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_start_oauth_or_request_arbitrary_resources() {
        for path in [
            "/desktop/auth/start",
            "https://relay.example/v1/account",
            "/local-api/../private",
            "/local-api/%2e%2e",
            "/desktop/status#fragment",
        ] {
            assert!(
                validate_request(&json!({"method":"GET","path":path,"body":null}).to_string())
                    .is_err()
            );
        }
        assert!(
            validate_request(r#"{"method":"GET","path":"/local-auth/me","body":null}"#).is_ok()
        );
        assert!(
            validate_request(r#"{"method":"DELETE","path":"/desktop/status","body":null}"#)
                .is_err()
        );
        assert!(
            validate_request(
                r#"{"method":"GET","path":"/desktop/status","body":null,"token":"bad"}"#
            )
            .is_err()
        );
    }
}
