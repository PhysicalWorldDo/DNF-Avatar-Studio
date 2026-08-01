#[cfg(windows)]
use std::ffi::c_void;

#[cfg(windows)]
#[repr(C)]
struct DataBlob {
    size: u32,
    data: *mut u8,
}

#[cfg(windows)]
#[link(name = "Crypt32")]
unsafe extern "system" {
    fn CryptProtectData(
        data_in: *const DataBlob,
        description: *const u16,
        optional_entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *const c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
    fn CryptUnprotectData(
        data_in: *const DataBlob,
        description: *mut *mut u16,
        optional_entropy: *const DataBlob,
        reserved: *mut c_void,
        prompt: *const c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
}

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

pub fn protect_for_current_user(input: &[u8], purpose: &[u8]) -> Result<Vec<u8>, String> {
    crypt_for_current_user(input, purpose, true)
}

pub fn unprotect_for_current_user(input: &[u8], purpose: &[u8]) -> Result<Vec<u8>, String> {
    crypt_for_current_user(input, purpose, false)
}

#[cfg(windows)]
fn crypt_for_current_user(input: &[u8], purpose: &[u8], protect: bool) -> Result<Vec<u8>, String> {
    let input_size = u32::try_from(input.len()).map_err(|_| protection_error(protect))?;
    let purpose_size = u32::try_from(purpose.len()).map_err(|_| protection_error(protect))?;
    let input_blob = DataBlob {
        size: input_size,
        data: input.as_ptr() as *mut u8,
    };
    let purpose_blob = DataBlob {
        size: purpose_size,
        data: purpose.as_ptr() as *mut u8,
    };
    let mut output_blob = DataBlob {
        size: 0,
        data: std::ptr::null_mut(),
    };
    let purpose_ptr = if purpose.is_empty() {
        std::ptr::null()
    } else {
        &purpose_blob
    };
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input_blob,
                std::ptr::null(),
                purpose_ptr,
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut output_blob,
            )
        } else {
            CryptUnprotectData(
                &input_blob,
                std::ptr::null_mut(),
                purpose_ptr,
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut output_blob,
            )
        }
    };
    if ok == 0 || output_blob.data.is_null() {
        return Err(protection_error(protect));
    }
    let output =
        unsafe { std::slice::from_raw_parts(output_blob.data, output_blob.size as usize).to_vec() };
    unsafe {
        LocalFree(output_blob.data.cast());
    }
    Ok(output)
}

#[cfg(not(windows))]
fn crypt_for_current_user(
    _input: &[u8],
    _purpose: &[u8],
    protect: bool,
) -> Result<Vec<u8>, String> {
    Err(protection_error(protect))
}

fn protection_error(protect: bool) -> String {
    if protect {
        "保护本地数据失败".to_string()
    } else {
        "读取受保护的本地数据失败".to_string()
    }
}
