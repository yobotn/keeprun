//! Add or remove the install folder on the machine PATH, so `keeprun` works in any terminal.
//! Done here rather than in the NSIS script, where string limits can truncate a long PATH.

use anyhow::{Context, Result};
use std::path::Path;
use winreg::enums::{RegType, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};
use winreg::{RegKey, RegValue};

const ENV_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

fn read_path(key: &RegKey) -> Result<String> {
    // Raw read keeps %SystemRoot% style entries unexpanded.
    let raw = key
        .get_raw_value("Path")
        .context("Cannot read the machine PATH")?;
    let (pairs, _) = raw.bytes.as_chunks::<2>();
    let words: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Ok(String::from_utf16_lossy(&words)
        .trim_end_matches('\0')
        .to_string())
}

fn write_path(key: &RegKey, value: &str) -> Result<()> {
    let mut bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|w| w.to_le_bytes())
        .collect();
    bytes.shrink_to_fit();
    key.set_raw_value(
        "Path",
        &RegValue {
            bytes: bytes.into(),
            vtype: RegType::REG_EXPAND_SZ,
        },
    )
    .context("Cannot write the machine PATH")?;
    broadcast_change();
    Ok(())
}

fn same(a: &str, b: &Path) -> bool {
    a.trim()
        .trim_end_matches('\\')
        .eq_ignore_ascii_case(b.display().to_string().trim_end_matches('\\'))
}

/// Returns true when PATH was changed.
pub fn add(dir: &Path) -> Result<bool> {
    let key =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(ENV_KEY, KEY_READ | KEY_WRITE)?;
    let path = read_path(&key)?;
    if path.split(';').any(|p| same(p, dir)) {
        return Ok(false);
    }
    let sep = if path.is_empty() || path.ends_with(';') {
        ""
    } else {
        ";"
    };
    write_path(&key, &format!("{path}{sep}{}", dir.display()))?;
    Ok(true)
}

pub fn remove(dir: &Path) -> Result<bool> {
    let key =
        RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(ENV_KEY, KEY_READ | KEY_WRITE)?;
    let path = read_path(&key)?;
    let kept: Vec<&str> = path.split(';').filter(|p| !same(p, dir)).collect();
    let next = kept.join(";");
    if next == path {
        return Ok(false);
    }
    write_path(&key, &next)?;
    Ok(true)
}

/// Tell running programs (Explorer, new terminals) that the environment changed.
fn broadcast_change() {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            2000,
            None,
        );
    }
}
