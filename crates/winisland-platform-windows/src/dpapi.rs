//! Windows DPAPI encryption bound to the current user account.
//!
//! Used for clipboard bodies, private notes and other local secrets (workflow Phase 3).
//! Data protected here can be decrypted only by the same Windows user on any of their
//! sessions, and never prompts.

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows::core::PCWSTR;
use winisland_platform::PlatformError;

fn blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, PlatformError> {
    let length = u32::try_from(data.len()).map_err(|_| PlatformError::backend("data too large"))?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: length,
        // DPAPI only reads input blobs; the mutable pointer type is an API artefact.
        pbData: data.as_ptr().cast_mut(),
    })
}

/// Copies an output blob into a Vec and releases the DPAPI allocation.
fn take_output(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if output.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI returned `cbData` readable bytes at `pbData`.
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: The buffer was allocated by DPAPI with LocalAlloc and is freed exactly once.
    let _ = unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
    bytes
}

/// Encrypts `data` for the current user. `entropy` adds an application-specific secret
/// that must be supplied again to decrypt.
pub fn protect(data: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>, PlatformError> {
    let input = blob(data)?;
    let entropy = entropy.map(blob).transpose()?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: Input blobs point at live slices for the duration of the call and `output`
    // is a valid out-parameter released by take_output.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            entropy
                .as_ref()
                .map(|blob| blob as *const CRYPT_INTEGER_BLOB),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(PlatformError::backend)?;
    Ok(take_output(output))
}

/// Decrypts data produced by [`protect`] with the same `entropy`.
pub fn unprotect(data: &[u8], entropy: Option<&[u8]>) -> Result<Vec<u8>, PlatformError> {
    let input = blob(data)?;
    let entropy = entropy.map(blob).transpose()?;
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: As in protect; no description is requested.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            entropy
                .as_ref()
                .map(|blob| blob as *const CRYPT_INTEGER_BLOB),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(PlatformError::backend)?;
    Ok(take_output(output))
}
