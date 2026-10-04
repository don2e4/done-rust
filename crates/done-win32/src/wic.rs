//! Image decoding through the Windows Imaging Component.
//!
//! `windows-sys` declares no COM interfaces, so the few vtable slots used here
//! are written out in the order `wincodec.h` declares them.

use done_core::Result;
use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::core::{GUID, HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::GENERIC_READ;
use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

// From wincodec.h.
const CLSID_WIC_IMAGING_FACTORY: GUID = GUID::from_u128(0xcacaf262_9370_4615_a13b_9f5539da4c0a);
const IID_IWIC_IMAGING_FACTORY: GUID = GUID::from_u128(0xec5ec8a9_c395_4314_9c77_54d7a935ff70);
const GUID_WIC_PIXEL_FORMAT_32BPP_PBGRA: GUID =
    GUID::from_u128(0x6fddc324_4e03_4bfe_b185_3d77768dc910);
const WIC_DECODE_METADATA_CACHE_ON_DEMAND: i32 = 0;
const WIC_BITMAP_DITHER_TYPE_NONE: i32 = 0;
const WIC_BITMAP_PALETTE_TYPE_CUSTOM: i32 = 0;

type Slot = usize;

#[repr(C)]
struct UnknownVtbl {
    query_interface: Slot,
    add_ref: Slot,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct FactoryVtbl {
    base: UnknownVtbl,
    create_decoder_from_filename: unsafe extern "system" fn(
        *mut c_void,
        PCWSTR,
        *const GUID,
        u32,
        i32,
        *mut *mut c_void,
    ) -> HRESULT,
    create_decoder_from_stream: Slot,
    create_decoder_from_file_handle: Slot,
    create_component_info: Slot,
    create_decoder: Slot,
    create_encoder: Slot,
    create_palette: Slot,
    create_format_converter: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DecoderVtbl {
    base: UnknownVtbl,
    query_capability: Slot,
    initialize: Slot,
    get_container_format: Slot,
    get_decoder_info: Slot,
    copy_palette: Slot,
    get_metadata_query_reader: Slot,
    get_preview: Slot,
    get_color_contexts: Slot,
    get_thumbnail: Slot,
    get_frame_count: Slot,
    get_frame: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct SourceVtbl {
    base: UnknownVtbl,
    get_size: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> HRESULT,
    get_pixel_format: Slot,
    get_resolution: Slot,
    copy_palette: Slot,
    copy_pixels:
        unsafe extern "system" fn(*mut c_void, *const c_void, u32, u32, *mut u8) -> HRESULT,
}

#[repr(C)]
struct ConverterVtbl {
    source: SourceVtbl,
    initialize: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        i32,
        *mut c_void,
        f64,
        i32,
    ) -> HRESULT,
    can_convert: Slot,
}

/// An owned COM interface pointer, released on drop.
struct Com(*mut c_void);

impl Com {
    /// # Safety
    /// The object must implement an interface whose vtable begins with `V`.
    unsafe fn vtable<V>(&self) -> &V {
        // SAFETY: a COM object's first field points to its vtable; the caller
        // guarantees the layout, and the object outlives the borrow of `self`.
        unsafe { &**(self.0 as *const *const V) }
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: every COM vtable begins with IUnknown, and this wrapper
            // owns exactly one reference.
            unsafe { (self.vtable::<UnknownVtbl>().release)(self.0) };
        }
    }
}

fn check(path: &Path, step: &str, result: HRESULT) -> Result<()> {
    if result < 0 {
        Err(format!(
            "{}: {step} failed (HRESULT 0x{:08x})",
            path.display(),
            result as u32
        )
        .into())
    } else {
        Ok(())
    }
}

/// Decoded first frame: width, height, and premultiplied BGRA rows, top down.
pub(crate) struct Pixels {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Decode an application-provided local file. COM must be initialized on
/// this thread.
pub(crate) fn decode(path: &Path) -> Result<Pixels> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut object = ptr::null_mut();
    // SAFETY: valid CLSID/IID pointers and an out pointer for the interface.
    check(path, "creating the WIC factory", unsafe {
        CoCreateInstance(
            &CLSID_WIC_IMAGING_FACTORY,
            ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IWIC_IMAGING_FACTORY,
            &mut object,
        )
    })?;
    let factory = Com(object);
    let mut object = ptr::null_mut();
    // SAFETY: `factory` is an IWICImagingFactory; the filename is a
    // NUL-terminated UTF-16 string that outlives the call.
    check(path, "opening the image", unsafe {
        (factory.vtable::<FactoryVtbl>().create_decoder_from_filename)(
            factory.0,
            wide.as_ptr(),
            ptr::null(),
            GENERIC_READ,
            WIC_DECODE_METADATA_CACHE_ON_DEMAND,
            &mut object,
        )
    })?;
    let decoder = Com(object);
    let mut object = ptr::null_mut();
    // SAFETY: `decoder` is an IWICBitmapDecoder; frame 0 exists for any
    // decodable file and the out pointer is valid.
    check(path, "reading the first frame", unsafe {
        (decoder.vtable::<DecoderVtbl>().get_frame)(decoder.0, 0, &mut object)
    })?;
    let frame = Com(object);
    let mut object = ptr::null_mut();
    // SAFETY: `factory` is an IWICImagingFactory and the out pointer is valid.
    check(path, "creating a converter", unsafe {
        (factory.vtable::<FactoryVtbl>().create_format_converter)(factory.0, &mut object)
    })?;
    let converter = Com(object);
    // SAFETY: `converter` is an IWICFormatConverter and `frame` an
    // IWICBitmapSource; both stay alive for the call.
    check(path, "converting to 32-bit BGRA", unsafe {
        (converter.vtable::<ConverterVtbl>().initialize)(
            converter.0,
            frame.0,
            &GUID_WIC_PIXEL_FORMAT_32BPP_PBGRA,
            WIC_BITMAP_DITHER_TYPE_NONE,
            ptr::null_mut(),
            0.0,
            WIC_BITMAP_PALETTE_TYPE_CUSTOM,
        )
    })?;
    let (mut width, mut height) = (0, 0);
    // SAFETY: IWICFormatConverter begins with the IWICBitmapSource vtable;
    // both out pointers are valid.
    check(path, "reading the size", unsafe {
        (converter.vtable::<SourceVtbl>().get_size)(converter.0, &mut width, &mut height)
    })?;
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| format!("{}: image too wide", path.display()))?;
    let size = stride
        .checked_mul(height)
        .ok_or_else(|| format!("{}: image too large", path.display()))?;
    let mut bgra = vec![0; size as usize];
    // SAFETY: the buffer holds exactly `stride * height` bytes and a null
    // rectangle copies the whole image.
    check(path, "copying pixels", unsafe {
        (converter.vtable::<SourceVtbl>().copy_pixels)(
            converter.0,
            ptr::null(),
            stride,
            size,
            bgra.as_mut_ptr(),
        )
    })?;
    Ok(Pixels {
        width,
        height,
        bgra,
    })
}
