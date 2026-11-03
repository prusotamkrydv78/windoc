//! Notchy phone unlock: a Windows credential provider (sign-in tile).
//!
//! The tile shows on the lock and sign-in screens next to the password and PIN tiles, which
//! keep working. While Windows has the provider loaded it broadcasts
//! `NOTCHY-UNLOCK?<pair id> <nonce> <pc name>` on UDP 47823 every two seconds. The paired
//! phone asks for the user's fingerprint and answers with the nonce sealed under the Notchy
//! Link key (ChaCha20-Poly1305). A valid answer makes the tile sign in automatically with the
//! password saved at install time.
//!
//! Install data lives in `HKLM\SOFTWARE\Notchy\Unlock` (admins and SYSTEM only):
//! `User`, `Domain`, `Password` (DPAPI, machine scope), `Key` (64 hex chars) and `PcName`.
//! `packaging\unlock\Install-NotchyUnlock.ps1` writes it and registers this DLL.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_NOTIMPL, E_POINTER,
    HLOCAL, LocalFree, NTSTATUS, S_FALSE,
};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Security::Authentication::Identity::{
    LSA_STRING, LsaConnectUntrusted, LsaDeregisterLogonProcess, LsaLookupAuthenticationPackage,
};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
use windows::Win32::System::Com::{CoTaskMemAlloc, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegCloseKey,
    RegGetValueW, RegOpenKeyExW,
};
use windows::Win32::UI::Shell::{
    CPFIS_NONE, CPFS_DISPLAY_IN_BOTH, CPFS_DISPLAY_IN_SELECTED_TILE, CPFT_LARGE_TEXT,
    CPFT_SMALL_TEXT, CPFT_SUBMIT_BUTTON, CPGSR_NO_CREDENTIAL_NOT_FINISHED,
    CPGSR_RETURN_CREDENTIAL_FINISHED, CPSI_NONE, CPUS_LOGON, CPUS_UNLOCK_WORKSTATION,
    CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION, CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR,
    CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE, CREDENTIAL_PROVIDER_FIELD_STATE,
    CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE, CREDENTIAL_PROVIDER_NO_DEFAULT,
    CREDENTIAL_PROVIDER_STATUS_ICON, CREDENTIAL_PROVIDER_USAGE_SCENARIO, ICredentialProvider,
    ICredentialProvider_Impl, ICredentialProviderCredential, ICredentialProviderCredential_Impl,
    ICredentialProviderCredentialEvents, ICredentialProviderEvents, SHStrDupW,
};
use windows_core::{GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, implement};

/// {6E3F8C2A-4B1D-4F7A-9C55-2D8E1A7B9F31}
pub const CLSID_NOTCHY_UNLOCK: GUID = GUID::from_u128(0x6e3f8c2a_4b1d_4f7a_9c55_2d8e1a7b9f31);

const PORT: u16 = 47823;
const FIELD_TITLE: u32 = 0;
const FIELD_HINT: u32 = 1;
const FIELD_SUBMIT: u32 = 2;
const FIELD_COUNT: u32 = 3;

// ---- Saved configuration ---------------------------------------------------------------

struct Config {
    user: String,
    domain: String,
    password: Vec<u16>,
    key: [u8; 32],
    pc_name: String,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_string(key: HKEY, name: &str) -> Option<String> {
    let name = wide(name);
    let mut size = 0u32;
    // SAFETY: querying the size only.
    unsafe {
        RegGetValueW(key, None, PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, None, Some(&mut size))
    }
    .ok()
    .ok()?;
    let mut buffer = vec![0u16; size as usize / 2 + 1];
    // SAFETY: `buffer` holds `size` bytes.
    unsafe {
        RegGetValueW(
            key,
            None,
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn read_binary(key: HKEY, name: &str) -> Option<Vec<u8>> {
    let name = wide(name);
    let mut size = 0u32;
    // SAFETY: querying the size only.
    unsafe {
        RegGetValueW(key, None, PCWSTR(name.as_ptr()), RRF_RT_REG_BINARY, None, None, Some(&mut size))
    }
    .ok()
    .ok()?;
    let mut buffer = vec![0u8; size as usize];
    // SAFETY: `buffer` holds `size` bytes.
    unsafe {
        RegGetValueW(
            key,
            None,
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_BINARY,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    buffer.truncate(size as usize);
    Some(buffer)
}

/// Decrypts the DPAPI blob the installer made (machine scope).
fn unprotect(blob: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points at `blob`; `output` is freed with LocalFree below.
    unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut output) }.ok()?;
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`.
    let plain = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    // SAFETY: allocated by CryptUnprotectData.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
    }
    Some(plain)
}

fn decode_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

fn load_config() -> Option<Config> {
    let path = wide("SOFTWARE\\Notchy\\Unlock");
    let mut key = HKEY::default();
    // SAFETY: `key` receives an open handle closed below.
    unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) }
        .ok()
        .ok()?;
    let config = (|| {
        let password = unprotect(&read_binary(key, "Password")?)?;
        let password: Vec<u16> = password
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        Some(Config {
            user: read_string(key, "User")?,
            domain: read_string(key, "Domain").unwrap_or_default(),
            password,
            key: decode_key(&read_string(key, "Key")?)?,
            pc_name: read_string(key, "PcName").unwrap_or_else(|| "PC".to_string()),
        })
    })();
    // SAFETY: opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    config
}

fn pair_id(key: &[u8; 32]) -> String {
    Sha256::digest(key)[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---- Waiting for the phone -------------------------------------------------------------

struct SendEvents(ICredentialProviderEvents, usize);
// SAFETY: LogonUI's event sink is safe to call from a worker thread (the Microsoft samples
// call CredentialsChanged from one).
unsafe impl Send for SendEvents {}

struct Watch {
    approved: AtomicBool,
    stop: AtomicBool,
    events: std::sync::Mutex<Option<SendEvents>>,
}

/// Broadcasts the request and waits for a sealed answer; stops when `watch.stop` is set.
fn listen(watch: Arc<Watch>, config: Arc<Config>) {
    let Ok(socket) = UdpSocket::bind(("0.0.0.0", PORT)).or_else(|_| UdpSocket::bind(("0.0.0.0", 0)))
    else {
        return;
    };
    let _ = socket.set_broadcast(true);
    let _ = socket.set_read_timeout(Some(Duration::from_millis(400)));
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&config.key));
    let id = pair_id(&config.key);
    let mut nonce = String::new();
    let mut nonce_at = Instant::now() - Duration::from_secs(3600);
    let mut asked_at = Instant::now() - Duration::from_secs(60);
    let mut buffer = [0u8; 512];
    while !watch.stop.load(Ordering::SeqCst) {
        let now = Instant::now();
        // A fresh nonce every two minutes, so an old answer cannot be replayed.
        if now.duration_since(nonce_at) > Duration::from_secs(120) {
            let mut bytes = [0u8; 16];
            if getrandom::fill(&mut bytes).is_err() {
                return;
            }
            nonce = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            nonce_at = now;
        }
        if now.duration_since(asked_at) > Duration::from_secs(2) {
            let ask = format!("NOTCHY-UNLOCK?{id} {nonce} {}", config.pc_name);
            let _ = socket.send_to(ask.as_bytes(), ("255.255.255.255", PORT));
            asked_at = now;
        }
        let Ok((count, _)) = socket.recv_from(&mut buffer) else {
            continue;
        };
        let Some(sealed) = std::str::from_utf8(&buffer[..count])
            .ok()
            .and_then(|text| text.strip_prefix("NOTCHY-UNLOCK!"))
        else {
            continue;
        };
        use base64::Engine;
        let Ok(frame) = base64::engine::general_purpose::STANDARD.decode(sealed.trim()) else {
            continue;
        };
        if frame.len() < 12 + 16 {
            continue;
        }
        let Ok(plain) = cipher.decrypt(Nonce::from_slice(&frame[..12]), &frame[12..]) else {
            continue;
        };
        if plain == format!("ok:{nonce}").as_bytes() {
            watch.approved.store(true, Ordering::SeqCst);
            if let Some(SendEvents(events, context)) = watch.events.lock().ok().and_then(|guard| {
                guard.as_ref().map(|events| SendEvents(events.0.clone(), events.1))
            }) {
                // SAFETY: LogonUI's sink; it asks for the credentials again.
                let _ = unsafe { events.CredentialsChanged(context) };
            }
            // One approval is enough; wait for Windows to use it.
            nonce_at = Instant::now() - Duration::from_secs(3600);
        }
    }
}

// ---- Packing the logon -----------------------------------------------------------------

/// `KERB_INTERACTIVE_UNLOCK_LOGON`, packed with its strings after the struct and string
/// pointers stored as offsets from the start, as LogonUI expects.
fn pack_logon(config: &Config, unlock: bool) -> Vec<u8> {
    let computer;
    let domain = if config.domain.is_empty() {
        computer = std::env::var("COMPUTERNAME").unwrap_or_default();
        &computer
    } else {
        &config.domain
    };
    let domain: Vec<u16> = domain.encode_utf16().collect();
    let user: Vec<u16> = config.user.encode_utf16().collect();
    let password = &config.password;
    // MessageType (4) + padding (4) + three UNICODE_STRINGs (16 each) + LUID (8).
    const HEADER: usize = 8 + 16 * 3 + 8;
    let mut out = vec![0u8; HEADER];
    let message_type: u32 = if unlock { 7 } else { 2 }; // KerbWorkstationUnlockLogon / KerbInteractiveLogon
    out[0..4].copy_from_slice(&message_type.to_le_bytes());
    let mut write_string = |slot: usize, text: &[u16], out: &mut Vec<u8>| {
        let offset = out.len() as u64;
        let bytes = (text.len() * 2) as u16;
        out[slot..slot + 2].copy_from_slice(&bytes.to_le_bytes());
        out[slot + 2..slot + 4].copy_from_slice(&bytes.to_le_bytes());
        out[slot + 8..slot + 16].copy_from_slice(&offset.to_le_bytes());
        for unit in text {
            out.extend_from_slice(&unit.to_le_bytes());
        }
    };
    write_string(8, &domain, &mut out);
    write_string(24, &user, &mut out);
    write_string(40, password, &mut out);
    out
}

fn negotiate_package() -> Option<u32> {
    let mut handle = windows::Win32::Foundation::HANDLE::default();
    // SAFETY: `handle` is closed below.
    let status = unsafe { LsaConnectUntrusted(&mut handle) };
    if status.is_err() {
        return None;
    }
    let name = b"Negotiate";
    let lsa_name = LSA_STRING {
        Length: name.len() as u16,
        MaximumLength: name.len() as u16,
        Buffer: windows_core::PSTR(name.as_ptr() as *mut u8),
    };
    let mut package = 0u32;
    // SAFETY: valid handle and name.
    let status = unsafe { LsaLookupAuthenticationPackage(handle, &lsa_name, &mut package) };
    // SAFETY: opened above.
    let _ = unsafe { LsaDeregisterLogonProcess(handle) };
    status.is_ok().then_some(package)
}

fn co_bytes(bytes: &[u8]) -> *mut u8 {
    // SAFETY: allocating; LogonUI frees it with CoTaskMemFree.
    let memory = unsafe { CoTaskMemAlloc(bytes.len()) } as *mut u8;
    if !memory.is_null() {
        // SAFETY: `memory` holds `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), memory, bytes.len()) };
    }
    memory
}

fn co_string(text: &str) -> windows_core::Result<PWSTR> {
    let text = wide(text);
    // SAFETY: `text` is NUL-terminated.
    unsafe { SHStrDupW(PCWSTR(text.as_ptr())) }
}

// ---- The credential (the tile) ---------------------------------------------------------

#[implement(ICredentialProviderCredential)]
struct Credential {
    config: Arc<Config>,
    watch: Arc<Watch>,
    unlock: bool,
}

impl ICredentialProviderCredential_Impl for Credential_Impl {
    fn Advise(&self, _events: Ref<'_, ICredentialProviderCredentialEvents>) -> windows_core::Result<()> {
        Ok(())
    }

    fn UnAdvise(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn SetSelected(&self) -> windows_core::Result<BOOL> {
        // Sign in straight away when the phone already approved.
        Ok(BOOL::from(self.watch.approved.load(Ordering::SeqCst)))
    }

    fn SetDeselected(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn GetFieldState(
        &self,
        field: u32,
        state: *mut CREDENTIAL_PROVIDER_FIELD_STATE,
        interactive: *mut CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE,
    ) -> windows_core::Result<()> {
        if field >= FIELD_COUNT || state.is_null() || interactive.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: checked for null above.
        unsafe {
            *state = if field == FIELD_TITLE {
                CPFS_DISPLAY_IN_BOTH
            } else {
                CPFS_DISPLAY_IN_SELECTED_TILE
            };
            *interactive = CPFIS_NONE;
        }
        Ok(())
    }

    fn GetStringValue(&self, field: u32) -> windows_core::Result<PWSTR> {
        match field {
            FIELD_TITLE => co_string("Unlock with phone"),
            FIELD_HINT => co_string(if self.watch.approved.load(Ordering::SeqCst) {
                "Approved, signing in…"
            } else {
                "Open the notification on your phone and touch the fingerprint sensor"
            }),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetBitmapValue(&self, _field: u32) -> windows_core::Result<HBITMAP> {
        Err(E_NOTIMPL.into())
    }

    fn GetCheckboxValue(&self, _field: u32, _checked: *mut BOOL, _label: *mut PWSTR) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSubmitButtonValue(&self, field: u32) -> windows_core::Result<u32> {
        if field == FIELD_SUBMIT {
            Ok(FIELD_HINT)
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetComboBoxValueCount(&self, _field: u32, _items: *mut u32, _selected: *mut u32) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueAt(&self, _field: u32, _item: u32) -> windows_core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn SetStringValue(&self, _field: u32, _text: &PCWSTR) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetCheckboxValue(&self, _field: u32, _checked: BOOL) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetComboBoxSelectedValue(&self, _field: u32, _selected: u32) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn CommandLinkClicked(&self, _field: u32) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSerialization(
        &self,
        response: *mut CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE,
        serialization: *mut CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
        status_text: *mut PWSTR,
        status_icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows_core::Result<()> {
        if response.is_null() || serialization.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: LogonUI passes valid out pointers.
        unsafe {
            if !status_icon.is_null() {
                *status_icon = CPSI_NONE;
            }
            if !self.watch.approved.swap(false, Ordering::SeqCst) {
                *response = CPGSR_NO_CREDENTIAL_NOT_FINISHED;
                if !status_text.is_null() {
                    *status_text = co_string("Waiting for your phone… open Notchy's notification")?;
                }
                return Ok(());
            }
            let package = negotiate_package().ok_or(windows_core::Error::from(E_NOTIMPL))?;
            let packed = pack_logon(&self.config, self.unlock);
            let memory = co_bytes(&packed);
            if memory.is_null() {
                return Err(E_POINTER.into());
            }
            *serialization = CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION {
                ulAuthenticationPackage: package,
                clsidCredentialProvider: CLSID_NOTCHY_UNLOCK,
                cbSerialization: packed.len() as u32,
                rgbSerialization: memory,
            };
            *response = CPGSR_RETURN_CREDENTIAL_FINISHED;
        }
        Ok(())
    }

    fn ReportResult(
        &self,
        _status: NTSTATUS,
        _substatus: NTSTATUS,
        status_text: *mut PWSTR,
        status_icon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows_core::Result<()> {
        // SAFETY: optional out pointers.
        unsafe {
            if !status_text.is_null() {
                *status_text = PWSTR::null();
            }
            if !status_icon.is_null() {
                *status_icon = CPSI_NONE;
            }
        }
        Ok(())
    }
}

// ---- The provider ----------------------------------------------------------------------

#[implement(ICredentialProvider)]
struct Provider {
    state: std::sync::Mutex<ProviderState>,
}

#[derive(Default)]
struct ProviderState {
    config: Option<Arc<Config>>,
    watch: Option<Arc<Watch>>,
    unlock: bool,
    credential: Option<ICredentialProviderCredential>,
}

impl ICredentialProvider_Impl for Provider_Impl {
    fn SetUsageScenario(&self, scenario: CREDENTIAL_PROVIDER_USAGE_SCENARIO, _flags: u32) -> windows_core::Result<()> {
        if scenario != CPUS_LOGON && scenario != CPUS_UNLOCK_WORKSTATION {
            return Err(E_NOTIMPL.into());
        }
        let config = load_config().ok_or(windows_core::Error::from(E_NOTIMPL))?;
        let mut state = self.state.lock().map_err(|_| windows_core::Error::from(E_POINTER))?;
        state.config = Some(Arc::new(config));
        state.unlock = scenario == CPUS_UNLOCK_WORKSTATION;
        Ok(())
    }

    fn SetSerialization(&self, _serialization: *const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Advise(&self, events: Ref<'_, ICredentialProviderEvents>, context: usize) -> windows_core::Result<()> {
        let mut state = self.state.lock().map_err(|_| windows_core::Error::from(E_POINTER))?;
        let Some(config) = state.config.clone() else {
            return Ok(());
        };
        let watch = Arc::new(Watch {
            approved: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            events: std::sync::Mutex::new(events.as_ref().map(|events| SendEvents(events.clone(), context))),
        });
        let worker = watch.clone();
        std::thread::spawn(move || listen(worker, config));
        state.watch = Some(watch);
        Ok(())
    }

    fn UnAdvise(&self) -> windows_core::Result<()> {
        if let Ok(mut state) = self.state.lock() {
            if let Some(watch) = state.watch.take() {
                watch.stop.store(true, Ordering::SeqCst);
                if let Ok(mut events) = watch.events.lock() {
                    *events = None;
                }
            }
        }
        Ok(())
    }

    fn GetFieldDescriptorCount(&self) -> windows_core::Result<u32> {
        Ok(FIELD_COUNT)
    }

    fn GetFieldDescriptorAt(&self, index: u32) -> windows_core::Result<*mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR> {
        let (kind, label) = match index {
            FIELD_TITLE => (CPFT_LARGE_TEXT, "Unlock with phone"),
            FIELD_HINT => (CPFT_SMALL_TEXT, "Status"),
            FIELD_SUBMIT => (CPFT_SUBMIT_BUTTON, "Submit"),
            _ => return Err(E_INVALIDARG.into()),
        };
        // SAFETY: LogonUI frees the descriptor and its label with CoTaskMemFree.
        unsafe {
            let descriptor = CoTaskMemAlloc(std::mem::size_of::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>())
                as *mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR;
            if descriptor.is_null() {
                return Err(E_POINTER.into());
            }
            descriptor.write(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR {
                dwFieldID: index,
                cpft: kind,
                pszLabel: co_string(label)?,
                guidFieldType: GUID::zeroed(),
            });
            Ok(descriptor)
        }
    }

    fn GetCredentialCount(&self, count: *mut u32, default: *mut u32, auto_logon: *mut BOOL) -> windows_core::Result<()> {
        if count.is_null() || default.is_null() || auto_logon.is_null() {
            return Err(E_POINTER.into());
        }
        let state = self.state.lock().map_err(|_| windows_core::Error::from(E_POINTER))?;
        let approved = state
            .watch
            .as_ref()
            .is_some_and(|watch| watch.approved.load(Ordering::SeqCst));
        // SAFETY: checked for null above.
        unsafe {
            *count = u32::from(state.config.is_some());
            *default = if approved { 0 } else { CREDENTIAL_PROVIDER_NO_DEFAULT };
            *auto_logon = BOOL::from(approved);
        }
        Ok(())
    }

    fn GetCredentialAt(&self, index: u32) -> windows_core::Result<ICredentialProviderCredential> {
        let mut state = self.state.lock().map_err(|_| windows_core::Error::from(E_POINTER))?;
        if index != 0 {
            return Err(E_INVALIDARG.into());
        }
        if let Some(credential) = &state.credential {
            return Ok(credential.clone());
        }
        let (Some(config), Some(watch)) = (state.config.clone(), state.watch.clone()) else {
            return Err(E_INVALIDARG.into());
        };
        let credential: ICredentialProviderCredential = Credential {
            config,
            watch,
            unlock: state.unlock,
        }
        .into();
        state.credential = Some(credential.clone());
        Ok(credential)
    }
}

// ---- COM plumbing ----------------------------------------------------------------------

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: checked for null.
        unsafe { *object = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let provider: IUnknown = Provider {
            state: std::sync::Mutex::new(ProviderState::default()),
        }
        .into();
        // SAFETY: valid iid and out pointer.
        unsafe { provider.query(iid, object).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

/// COM entry point.
///
/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    // SAFETY: checked for null.
    unsafe {
        *object = std::ptr::null_mut();
        if *clsid != CLSID_NOTCHY_UNLOCK {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(iid, object)
    }
}

/// LogonUI keeps the DLL loaded while it runs.
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_offsets_after_the_header() {
        let config = Config {
            user: "ab".into(),
            domain: "PC".into(),
            password: "pw".encode_utf16().collect(),
            key: [0; 32],
            pc_name: "PC".into(),
        };
        let packed = pack_logon(&config, true);
        assert_eq!(u32::from_le_bytes(packed[0..4].try_into().unwrap()), 7);
        assert_eq!(u64::from_le_bytes(packed[16..24].try_into().unwrap()), 64);
        assert_eq!(packed.len(), 64 + 4 + 4 + 4);
    }
}
