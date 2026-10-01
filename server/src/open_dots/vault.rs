//! The password vault: AES-256-GCM sealing with a master key in the macOS Keychain (a 0600 key
//! file elsewhere), ported from Open Dot's `vault.ts`. Plaintext only ever leaves this module to be
//! typed into a dot's page (`browser::fill_login`) or to call a provider (Composio tokens and keys).
//! Nothing here logs, and no error message carries a secret.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::Aes256Gcm;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use rand::RngCore;

use crate::paths::GxserverPaths;

/// The Keychain item that holds the master key (service, account `master`).
const KEYCHAIN_SERVICE: &str = "ghostex-open-dots-vault";
const KEYCHAIN_ACCOUNT: &str = "master";
const SECURITY_TOOL: &str = "/usr/bin/security";
/// `security` exits with errSecItemNotFound's code when the item does not exist yet.
const SECURITY_ITEM_NOT_FOUND_EXIT: i32 = 44;

#[derive(Clone, Copy)]
struct MasterKey {
    bytes: [u8; 32],
    backend: &'static str,
}

/// The master key per gxserver root, read once per process.
fn key_cache() -> &'static Mutex<HashMap<PathBuf, MasterKey>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, MasterKey>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key_file(paths: &GxserverPaths) -> PathBuf {
    paths.root_dir.join("open-dots").join("vault.key")
}

/// Where the master key lives, for Settings: `"keychain"` or `"file"`.
pub(crate) fn vault_backend(paths: &GxserverPaths) -> &'static str {
    if let Some(key) = key_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&paths.root_dir)
    {
        return key.backend;
    }
    // Not loaded yet: report where `master_key` will look first, without creating anything
    // (Settings polls this, and it must not spawn `security` every few seconds).
    if key_file(paths).is_file() || !cfg!(target_os = "macos") {
        "file"
    } else {
        "keychain"
    }
}

/// Encrypts any secret with the vault key: `<iv b64>.<tag b64>.<data b64>` (Open Dot's format).
pub(crate) fn seal(paths: &GxserverPaths, plain: &str) -> Result<String, String> {
    let key = master_key(paths)?;
    let cipher = Aes256Gcm::new_from_slice(&key.bytes)
        .map_err(|_| "The vault key is not valid.".to_string())?;
    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let nonce = aes_gcm::aead::Nonce::<Aes256Gcm>::try_from(&iv[..])
        .map_err(|_| "Couldn't seal the secret.".to_string())?;
    let mut sealed = cipher
        .encrypt(&nonce, plain.as_bytes())
        .map_err(|_| "Couldn't seal the secret.".to_string())?;
    // aes-gcm appends the 16-byte tag; Open Dot's format stores it as its own part.
    let tag = sealed.split_off(sealed.len() - 16);
    Ok(format!(
        "{}.{}.{}",
        BASE64.encode(iv),
        BASE64.encode(tag),
        BASE64.encode(sealed)
    ))
}

pub(crate) fn unseal(paths: &GxserverPaths, sealed: &str) -> Result<String, String> {
    let unreadable = || "This saved secret can't be read. Save it again.".to_string();
    let mut parts = sealed.trim().split('.');
    let (Some(iv), Some(tag), Some(data), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(unreadable());
    };
    let iv = BASE64.decode(iv).map_err(|_| unreadable())?;
    let tag = BASE64.decode(tag).map_err(|_| unreadable())?;
    let mut data = BASE64.decode(data).map_err(|_| unreadable())?;
    if iv.len() != 12 || tag.len() != 16 {
        return Err(unreadable());
    }
    let key = master_key(paths)?;
    let cipher = Aes256Gcm::new_from_slice(&key.bytes).map_err(|_| unreadable())?;
    let nonce = aes_gcm::aead::Nonce::<Aes256Gcm>::try_from(&iv[..]).map_err(|_| unreadable())?;
    data.extend_from_slice(&tag);
    let plain = cipher.decrypt(&nonce, data.as_slice()).map_err(|_| {
        "This saved secret can't be unlocked with this computer's vault key. Save it again."
            .to_string()
    })?;
    String::from_utf8(plain).map_err(|_| unreadable())
}

/// The cached master key, loading (and on first use creating) it.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// An existing key file wins over the Keychain, and the Keychain key is only created when `security` reports the item missing (exit 44), never after another failure such as a locked keychain. Open Dot created a fresh key whenever the lookup failed, which silently orphans every saved login sealed with the old key; mixing a file key and a Keychain key would do the same.
fn master_key(paths: &GxserverPaths) -> Result<MasterKey, String> {
    let mut cache = key_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(key) = cache.get(&paths.root_dir) {
        return Ok(*key);
    }
    let file = key_file(paths);
    let key = if file.is_file() {
        MasterKey {
            bytes: read_key_file(&file)?,
            backend: "file",
        }
    } else if cfg!(target_os = "macos") {
        match keychain_master_key()? {
            Some(bytes) => MasterKey {
                bytes,
                backend: "keychain",
            },
            None => MasterKey {
                bytes: create_key_file(&file)?,
                backend: "file",
            },
        }
    } else {
        MasterKey {
            bytes: create_key_file(&file)?,
            backend: "file",
        }
    };
    cache.insert(paths.root_dir.clone(), key);
    Ok(key)
}

/// The Keychain key: read it, or create it when it does not exist. `Ok(None)` means the Keychain
/// cannot store it here, so the caller uses a key file instead.
fn keychain_master_key() -> Result<Option<[u8; 32]>, String> {
    match keychain_find()? {
        KeychainLookup::Found(hex) => {
            return parse_hex_key(&hex)
                .map(Some)
                .ok_or_else(|| "The vault key in the Keychain is damaged.".to_string());
        }
        KeychainLookup::Missing => {}
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    if !keychain_add(&hex_encode(&bytes)) {
        return Ok(None);
    }
    // Read it back so the key in use is the one the Keychain really holds.
    match keychain_find()? {
        KeychainLookup::Found(hex) => Ok(parse_hex_key(&hex)),
        KeychainLookup::Missing => Ok(None),
    }
}

enum KeychainLookup {
    Found(String),
    Missing,
}

fn keychain_find() -> Result<KeychainLookup, String> {
    let output = std::process::Command::new(SECURITY_TOOL)
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
        ])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("Couldn't reach the Keychain: {error}"))?;
    if output.status.success() {
        return Ok(KeychainLookup::Found(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ));
    }
    if output.status.code() == Some(SECURITY_ITEM_NOT_FOUND_EXIT) {
        return Ok(KeychainLookup::Missing);
    }
    Err(format!(
        "Couldn't read the OpenDots vault key from the Keychain ({}). Unlock the Keychain and try again.",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

/// Adds the key through `security -i` so the hex key goes over stdin instead of the command line,
/// where any process listing could read it.
fn keychain_add(hex: &str) -> bool {
    let Ok(mut child) = std::process::Command::new(SECURITY_TOOL)
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };
    let command =
        format!("add-generic-password -U -s {KEYCHAIN_SERVICE} -a {KEYCHAIN_ACCOUNT} -w {hex}\n");
    let wrote = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(command.as_bytes()).is_ok())
        .unwrap_or(false);
    let exited = child.wait().map(|status| status.success()).unwrap_or(false);
    wrote && exited
}

fn read_key_file(file: &Path) -> Result<[u8; 32], String> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("Couldn't read the vault key file: {error}"))?;
    parse_hex_key(text.trim()).ok_or_else(|| "The vault key file is damaged.".to_string())
}

fn create_key_file(file: &Path) -> Result<[u8; 32], String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Couldn't create the vault folder: {error}"))?;
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(file) {
        Ok(mut handle) => {
            handle
                .write_all(hex_encode(&bytes).as_bytes())
                .map_err(|error| format!("Couldn't write the vault key file: {error}"))?;
            Ok(bytes)
        }
        // Another thread or process created it first: use theirs.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_key_file(file),
        Err(error) => Err(format!("Couldn't create the vault key file: {error}")),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_hex_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}
