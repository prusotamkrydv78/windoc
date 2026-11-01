//! Dragging files out of the app: the same drag Explorer starts, so they can be dropped on
//! the desktop, into a folder, a chat or an email. The shell builds the data object from the
//! files, so every drop target understands it.

use std::path::PathBuf;

use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, S_OK};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DoDragDrop, IDropSource, IDropSource_Impl,
    OleInitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, IShellItemArray, SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};
use windows::core::{BOOL, HRESULT, HSTRING, implement};

/// Ends the drag when the left button comes up (a drop) or Escape is pressed (a cancel).
#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Drags `paths` out of the app until the user drops or cancels. Offers copy and link only,
/// so the files stay where they are. `true` when they were dropped somewhere. Call on the UI
/// thread while the left button is down.
pub fn drag_files_out(paths: &[PathBuf]) -> windows::core::Result<bool> {
    // SAFETY: OLE is initialised for this (UI) thread; a second call is harmless.
    let _ = unsafe { OleInitialize(None) };
    let mut ids: Vec<*const ITEMIDLIST> = Vec::new();
    for path in paths {
        let mut id = std::ptr::null_mut();
        // SAFETY: `id` receives a shell-allocated item id, freed below.
        if unsafe { SHParseDisplayName(&HSTRING::from(path.as_os_str()), None, &mut id, 0, None) }
            .is_ok()
            && !id.is_null()
        {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Ok(false);
    }
    // SAFETY: Every id is a valid absolute item id from SHParseDisplayName.
    let items: windows::core::Result<IShellItemArray> =
        unsafe { SHCreateShellItemArrayFromIDLists(&ids) };
    for id in ids {
        // SAFETY: Each id was allocated by the shell and is no longer used.
        unsafe { CoTaskMemFree(Some(id.cast())) };
    }
    // SAFETY: Asking the shell for the items' data object, as Explorer does.
    let data: IDataObject = unsafe { items?.BindToHandler(None, &BHID_DataObject)? };
    let source: IDropSource = Source.into();
    let mut effect = DROPEFFECT(0);
    // SAFETY: Runs the modal drag loop on this thread with valid COM objects.
    let result = unsafe {
        DoDragDrop(
            &data,
            &source,
            DROPEFFECT_COPY | DROPEFFECT_LINK,
            &mut effect,
        )
    };
    Ok(result == DRAGDROP_S_DROP && effect != DROPEFFECT(0))
}
