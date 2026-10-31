//! UI Automation provider for the island window.
//!
//! The island is drawn with Skia, so Windows sees no controls. The app publishes a flat
//! list of accessible elements (name, control type, screen bounds) each time the layout
//! changes; this module answers `WM_GETOBJECT` with a fragment root whose children are
//! those elements, so Narrator and other assistive tools can read and navigate them.

use std::sync::LazyLock;

use parking_lot::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::WM_GETOBJECT;
use windows::core::{BSTR, IUnknown, Result, implement};

/// UIA control types the app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibleRole {
    Button,
    Group,
    ListItem,
    Tab,
    TabList,
    Text,
}

impl AccessibleRole {
    fn control_type(self) -> UIA_CONTROLTYPE_ID {
        match self {
            Self::Button => UIA_ButtonControlTypeId,
            Self::Group => UIA_GroupControlTypeId,
            Self::ListItem => UIA_ListItemControlTypeId,
            Self::Tab => UIA_TabItemControlTypeId,
            Self::TabList => UIA_TabControlTypeId,
            Self::Text => UIA_TextControlTypeId,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessibleElement {
    pub name: String,
    pub role: AccessibleRole,
    /// Screen coordinates in physical pixels: left, top, width, height.
    pub bounds: (f64, f64, f64, f64),
    pub selected: bool,
}

struct Tree {
    hwnd: isize,
    name: String,
    elements: Vec<AccessibleElement>,
}

static TREE: LazyLock<Mutex<Tree>> = LazyLock::new(|| {
    Mutex::new(Tree {
        hwnd: 0,
        name: String::new(),
        elements: Vec::new(),
    })
});

const SUBCLASS_ID: usize = 0x5549_4131;

/// Starts answering UI Automation requests for `hwnd`, a window owned by this thread.
pub fn install(hwnd: isize, window_name: &str) {
    {
        let mut tree = TREE.lock();
        tree.hwnd = hwnd;
        tree.name = window_name.to_string();
    }
    // SAFETY: hwnd is a live window created on this thread; the subclass procedure only
    // forwards unhandled messages to DefSubclassProc.
    let _ = unsafe { SetWindowSubclass(HWND(hwnd as *mut _), Some(subclass_proc), SUBCLASS_ID, 0) };
}

/// Replaces the published elements. Cheap to call every frame when nothing changed.
pub fn publish(elements: Vec<AccessibleElement>) {
    let mut tree = TREE.lock();
    if tree.elements != elements {
        tree.elements = elements;
    }
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_GETOBJECT && lparam.0 as i32 == UiaRootObjectId {
        let root: IRawElementProviderSimple = Root.into();
        // SAFETY: Arguments are forwarded unchanged from WM_GETOBJECT.
        return unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) };
    }
    // SAFETY: Every other message continues down the subclass chain unchanged.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn variant_string(text: &str) -> VARIANT {
    VARIANT::from(BSTR::from(text))
}

fn variant_i32(value: i32) -> VARIANT {
    VARIANT::from(value)
}

fn variant_bool(value: bool) -> VARIANT {
    VARIANT::from(value)
}

fn runtime_id(index: i32) -> Result<*mut SAFEARRAY> {
    // SAFETY: A two-element VT_I4 vector is created and filled with in-range indices.
    unsafe {
        let array = SafeArrayCreateVector(VT_I4, 0, 2);
        for (position, value) in [UiaAppendRuntimeId as i32, index].iter().enumerate() {
            let position = position as i32;
            SafeArrayPutElement(array, &position, std::ptr::from_ref(value).cast())?;
        }
        Ok(array)
    }
}

fn host_provider() -> Result<IRawElementProviderSimple> {
    let hwnd = TREE.lock().hwnd;
    // SAFETY: The window handle was registered by install and identifies this window.
    unsafe { UiaHostProviderFromHwnd(HWND(hwnd as *mut _)) }
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct Root;

impl IRawElementProviderSimple_Impl for Root_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(windows::core::Error::empty())
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        Ok(match property {
            UIA_NamePropertyId => variant_string(&TREE.lock().name),
            UIA_ControlTypePropertyId => variant_i32(UIA_PaneControlTypeId.0),
            UIA_IsKeyboardFocusablePropertyId => variant_bool(false),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        host_provider()
    }
}

impl IRawElementProviderFragment_Impl for Root_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let count = TREE.lock().elements.len();
        let index = match direction {
            NavigateDirection_FirstChild if count > 0 => 0,
            NavigateDirection_LastChild if count > 0 => count - 1,
            _ => return Err(windows::core::Error::empty()),
        };
        Ok(Element { index }.into())
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(Root.into())
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let tree = TREE.lock();
        let found = tree.elements.iter().position(|element| {
            let (left, top, width, height) = element.bounds;
            x >= left && x < left + width && y >= top && y < top + height
        });
        drop(tree);
        match found {
            Some(index) => Ok(Element { index }.into()),
            None => Ok(Root.into()),
        }
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        Err(windows::core::Error::empty())
    }
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment)]
struct Element {
    index: usize,
}

impl Element_Impl {
    fn element(&self) -> Option<AccessibleElement> {
        TREE.lock().elements.get(self.index).cloned()
    }
}

impl IRawElementProviderSimple_Impl for Element_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(windows::core::Error::empty())
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let Some(element) = self.element() else {
            return Ok(VARIANT::default());
        };
        Ok(match property {
            UIA_NamePropertyId => variant_string(&element.name),
            UIA_ControlTypePropertyId => variant_i32(element.role.control_type().0),
            UIA_IsKeyboardFocusablePropertyId => variant_bool(false),
            UIA_SelectionItemIsSelectedPropertyId => variant_bool(element.selected),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        Err(windows::core::Error::empty())
    }
}

impl IRawElementProviderFragment_Impl for Element_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let count = TREE.lock().elements.len();
        match direction {
            NavigateDirection_Parent => Ok(Root.into()),
            NavigateDirection_NextSibling if self.index + 1 < count => Ok(Element {
                index: self.index + 1,
            }
            .into()),
            NavigateDirection_PreviousSibling if self.index > 0 => Ok(Element {
                index: self.index - 1,
            }
            .into()),
            _ => Err(windows::core::Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        runtime_id(self.index as i32 + 1)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let Some(element) = self.element() else {
            return Ok(UiaRect::default());
        };
        let (left, top, width, height) = element.bounds;
        Ok(UiaRect {
            left,
            top,
            width,
            height,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(Root.into())
    }
}
