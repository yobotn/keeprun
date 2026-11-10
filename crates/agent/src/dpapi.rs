//! Encrypt secret env values at rest with Windows DPAPI (current account scope).
//! The prod agent runs as LocalSystem, so only LocalSystem can decrypt them.

use anyhow::{bail, Result};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const PREFIX: &str = "dpapi:";

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
    v
}

pub fn encrypt(plain: &str) -> Result<String> {
    let input = blob(plain.as_bytes());
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )?;
        let bytes = take(out);
        Ok(format!("{PREFIX}{}", hex(&bytes)))
    }
}

pub fn decrypt(stored: &str) -> Result<String> {
    let Some(h) = stored.strip_prefix(PREFIX) else {
        // Plain value written before encryption was enabled.
        return Ok(stored.to_string());
    };
    let Some(bytes) = unhex(h) else {
        bail!("Stored secret is damaged.")
    };
    let input = blob(&bytes);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        if CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut out,
        )
        .is_err()
        {
            bail!("This account cannot decrypt the stored secret. Secrets can only be read by the account that saved them (LocalSystem for the service). Enter the value again.");
        }
        Ok(String::from_utf8_lossy(&take(out)).to_string())
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let (pairs, _) = b.as_chunks::<2>();
    pairs
        .iter()
        .map(|[hi, lo]| Some((digit(*hi)? << 4) | digit(*lo)?))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let e = super::encrypt("s3cret value").unwrap();
        assert!(e.starts_with("dpapi:"));
        assert!(!e.contains("s3cret"));
        assert_eq!(super::decrypt(&e).unwrap(), "s3cret value");
        assert_eq!(super::decrypt("plain").unwrap(), "plain");
        // Corrupt values fail cleanly instead of panicking.
        assert!(super::decrypt("dpapi:a\u{e9}b").is_err());
        assert!(super::decrypt("dpapi:zz").is_err());
    }
}
