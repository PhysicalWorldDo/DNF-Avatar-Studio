#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};

pub const DNF_OK: c_int = 0;

#[repr(C)]
pub struct NpkEntry {
    pub name: [c_char; 256],
    pub offset: i32,
    pub size: i32,
}

#[repr(C)]
pub struct NpkFile {
    pub entries: *mut NpkEntry,
    pub count: c_int,
    pub private_data: *mut c_void,
}

#[repr(C)]
pub struct ImageFrame {
    pub format: i32,
    pub compress: i32,
    pub w: i32,
    pub h: i32,
    pub x: i32,
    pub y: i32,
    pub mw: i32,
    pub mh: i32,
    pub data: *mut u8,
    pub data_size: i32,
    pub link_index: i32,
}

#[repr(C)]
pub struct ImgFile {
    pub version: c_int,
    pub frames: *mut ImageFrame,
    pub frame_count: c_int,
    pub private_data: *mut c_void,
}

extern "C" {
    pub fn dnf_npk_open_mem(buf: *const u8, len: usize, out: *mut *mut NpkFile) -> c_int;
    pub fn dnf_npk_read_img(npk: *mut NpkFile, index: c_int, out: *mut *mut ImgFile) -> c_int;
    pub fn dnf_npk_save_file(path: *const c_char, npk: *mut NpkFile, group_by_md5: c_int) -> c_int;
    pub fn dnf_npk_create(out: *mut *mut NpkFile) -> c_int;
    pub fn dnf_npk_add_entry(
        npk: *mut NpkFile,
        name: *const c_char,
        data: *const u8,
        size: c_int,
    ) -> c_int;
    pub fn dnf_npk_free(npk: *mut NpkFile);
    pub fn dnf_img_palette_count(img: *mut ImgFile) -> c_int;
    #[allow(dead_code)]
    pub fn dnf_img_open_mem(buf: *const u8, len: usize, out: *mut *mut ImgFile) -> c_int;
    pub fn dnf_img_save_file(path: *const c_char, img: *mut ImgFile) -> c_int;
    pub fn dnf_img_free(img: *mut ImgFile);
    pub fn dnf_image_encode(
        rgba: *const u8,
        w: c_int,
        h: c_int,
        fmt: c_int,
        compress: c_int,
        out_data: *mut *mut u8,
        out_size: *mut c_int,
    ) -> c_int;
    pub fn dnf_image_decode_with_palette(
        img: *mut ImgFile,
        frame_index: c_int,
        palette_index: c_int,
        out_rgba: *mut *mut u8,
        out_w: *mut c_int,
        out_h: *mut c_int,
    ) -> c_int;
    pub fn dnf_free(ptr: *mut c_void);
}
