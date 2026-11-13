//! Loads the NVIDIA encoder at run time instead of linking it.
//!
//! nvidia-video-codec-sdk normally links libnvidia-encode, so the binary would
//! not build or start on machines without the NVIDIA driver. With its
//! `ci-check` feature nothing is linked, and these two definitions provide the
//! only driver functions its encoder API calls, forwarding to the library
//! loaded with dlopen.

use nvidia_video_codec_sdk::sys::nvEncodeAPI::{NV_ENCODE_API_FUNCTION_LIST, NVENCSTATUS};
use std::sync::OnceLock;

#[cfg(windows)]
const ENCODE_LIB: &str = "nvEncodeAPI64.dll";
#[cfg(windows)]
const CUDA_LIB: &str = "nvcuda.dll";
#[cfg(not(windows))]
const ENCODE_LIB: &str = "libnvidia-encode.so.1";
#[cfg(not(windows))]
const CUDA_LIB: &str = "libcuda.so.1";

fn lib() -> Option<&'static libloading::Library> {
    static LIB: OnceLock<Option<libloading::Library>> = OnceLock::new();
    // SAFETY: loading the NVIDIA driver's own library, which has no initializers with preconditions.
    LIB.get_or_init(|| unsafe { libloading::Library::new(ENCODE_LIB).ok() }).as_ref()
}

/// Why NVENC cannot be used on this machine, if it cannot.
pub fn missing() -> Option<String> {
    if lib().is_none() {
        return Some(format!("{ENCODE_LIB} not found (no NVIDIA driver)"));
    }
    // cudarc panics instead of returning an error when libcuda is missing, so check first.
    // SAFETY: as above.
    if unsafe { libloading::Library::new(CUDA_LIB) }.is_err() {
        return Some(format!("{CUDA_LIB} not found (no NVIDIA driver)"));
    }
    None
}

/// # Safety
/// Same contract as the NVIDIA function: `function_list` points to a list with its version set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn NvEncodeAPICreateInstance(function_list: *mut NV_ENCODE_API_FUNCTION_LIST) -> NVENCSTATUS {
    type F = unsafe extern "C" fn(*mut NV_ENCODE_API_FUNCTION_LIST) -> NVENCSTATUS;
    match lib().and_then(|l| unsafe { l.get::<F>(b"NvEncodeAPICreateInstance\0") }.ok()) {
        Some(f) => unsafe { f(function_list) },
        None => NVENCSTATUS::NV_ENC_ERR_NO_ENCODE_DEVICE,
    }
}

/// # Safety
/// Same contract as the NVIDIA function: `version` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn NvEncodeAPIGetMaxSupportedVersion(version: *mut u32) -> NVENCSTATUS {
    type F = unsafe extern "C" fn(*mut u32) -> NVENCSTATUS;
    match lib().and_then(|l| unsafe { l.get::<F>(b"NvEncodeAPIGetMaxSupportedVersion\0") }.ok()) {
        Some(f) => unsafe { f(version) },
        None => NVENCSTATUS::NV_ENC_ERR_NO_ENCODE_DEVICE,
    }
}
