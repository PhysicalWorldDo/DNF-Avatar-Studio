use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use encoding_rs::EUC_KR;

use crate::dnflib_sys::{self, ImgFile, NpkFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpkEntryInfo {
    pub index: usize,
    pub name: String,
    pub offset: i32,
    pub size: i32,
}

pub struct NpkArchive {
    ptr: NonNull<NpkFile>,
    _bytes: Vec<u8>,
}

impl NpkArchive {
    pub fn open_path(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|err| format!("读取 NPK 失败：{}", err))?;
        Self::open_bytes(bytes)
    }

    pub fn open_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        let mut out = std::ptr::null_mut();
        let rc = unsafe { dnflib_sys::dnf_npk_open_mem(bytes.as_ptr(), bytes.len(), &mut out) };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib 打开 NPK 失败，错误码 {rc}"));
        }

        let ptr = NonNull::new(out).ok_or_else(|| "dnflib 返回了空 NPK 指针".to_string())?;
        Ok(Self { ptr, _bytes: bytes })
    }

    pub fn entries(&self) -> Vec<NpkEntryInfo> {
        let npk = unsafe { self.ptr.as_ref() };
        if npk.entries.is_null() || self.entry_count() == 0 {
            return Vec::new();
        }

        (0..self.entry_count())
            .map(|index| {
                let entry = unsafe { &*npk.entries.add(index) };
                NpkEntryInfo {
                    index,
                    name: decode_entry_name(&entry.name),
                    offset: entry.offset,
                    size: entry.size,
                }
            })
            .collect()
    }

    pub fn entry_count(&self) -> usize {
        unsafe { self.ptr.as_ref().count.max(0) as usize }
    }

    pub fn read_img(&self, index: usize) -> Result<ImgArchive, String> {
        let mut out = std::ptr::null_mut();
        let rc = unsafe { dnflib_sys::dnf_npk_read_img(self.ptr.as_ptr(), index as i32, &mut out) };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib 读取 IMG 失败，错误码 {rc}"));
        }

        let ptr = NonNull::new(out).ok_or_else(|| "dnflib 返回了空 IMG 指针".to_string())?;
        Ok(ImgArchive {
            ptr,
            _owned_bytes: None,
        })
    }

    pub fn find_entry_index(&self, img_name: &str) -> Option<usize> {
        self.entries()
            .into_iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(img_name))
            .map(|entry| entry.index)
    }

    pub fn entry_bytes(&self, index: usize) -> Result<&[u8], String> {
        let entry = self
            .entries()
            .into_iter()
            .find(|entry| entry.index == index)
            .ok_or_else(|| format!("NPK entry {index} does not exist"))?;
        if entry.offset < 0 || entry.size <= 0 {
            return Err(format!("invalid NPK entry range for {}", entry.name));
        }
        let start = entry.offset as usize;
        let end = start
            .checked_add(entry.size as usize)
            .ok_or_else(|| format!("NPK entry range overflow for {}", entry.name))?;
        self._bytes
            .get(start..end)
            .ok_or_else(|| format!("NPK entry bytes out of range for {}", entry.name))
    }
}

impl Drop for NpkArchive {
    fn drop(&mut self) {
        unsafe { dnflib_sys::dnf_npk_free(self.ptr.as_ptr()) };
    }
}

pub struct NpkBuilder {
    ptr: NonNull<NpkFile>,
}

impl NpkBuilder {
    pub fn create() -> Result<Self, String> {
        let mut out = std::ptr::null_mut();
        let rc = unsafe { dnflib_sys::dnf_npk_create(&mut out) };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib create NPK failed: {rc}"));
        }
        let ptr =
            NonNull::new(out).ok_or_else(|| "dnflib returned null NPK builder".to_string())?;
        Ok(Self { ptr })
    }

    pub fn add_entry(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Err(format!("empty IMG data for {name}"));
        }
        let (encoded_name, _, had_errors) = EUC_KR.encode(name);
        if had_errors {
            return Err(format!("entry name cannot be encoded as EUC-KR: {name}"));
        }
        let c_name = CString::new(encoded_name.as_ref())
            .map_err(|_| format!("entry name contains NUL byte: {name}"))?;
        let size = i32::try_from(data.len())
            .map_err(|_| format!("IMG data is too large for NPK entry: {name}"))?;
        let rc = unsafe {
            dnflib_sys::dnf_npk_add_entry(self.ptr.as_ptr(), c_name.as_ptr(), data.as_ptr(), size)
        };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib add NPK entry failed for {name}: {rc}"));
        }
        Ok(())
    }

    pub fn save_path_direct(
        &mut self,
        output_path: &Path,
        group_by_md5: bool,
    ) -> Result<(), String> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let c_path = CString::new(output_path.to_string_lossy().as_bytes())
            .map_err(|_| format!("output path contains NUL byte: {}", output_path.display()))?;
        let rc = unsafe {
            dnflib_sys::dnf_npk_save_file(
                c_path.as_ptr(),
                self.ptr.as_ptr(),
                if group_by_md5 { 1 } else { 0 },
            )
        };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!(
                "dnflib save NPK directly to {} failed: {rc}",
                output_path.display()
            ));
        }
        Ok(())
    }

    pub fn save_path_create_new(
        &mut self,
        output_path: &Path,
        group_by_md5: bool,
    ) -> Result<(), String> {
        let parent = output_path
            .parent()
            .ok_or_else(|| "output NPK has no parent directory".to_string())?;
        std::fs::create_dir_all(parent).map_err(|_| "无法创建输出 NPK 所在目录".to_string())?;
        let temporary_path = reserve_adjacent_temporary_path(parent)?;
        if let Err(error) = self.save_path_direct(&temporary_path, group_by_md5) {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(error);
        }

        let installed = install_complete_file_create_new(&temporary_path, output_path);
        let _ = std::fs::remove_file(&temporary_path);
        installed
    }

    pub fn save_path(&mut self, output_path: &Path, group_by_md5: bool) -> Result<(), String> {
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
        }
        let temp_path = std::env::temp_dir().join(format!(
            "dnf_avatar_export_{}_{}.npk",
            std::process::id(),
            unique_nanos()
        ));
        let c_path = CString::new(temp_path.to_string_lossy().as_bytes())
            .map_err(|_| format!("output path contains NUL byte: {}", temp_path.display()))?;
        let rc = unsafe {
            dnflib_sys::dnf_npk_save_file(
                c_path.as_ptr(),
                self.ptr.as_ptr(),
                if group_by_md5 { 1 } else { 0 },
            )
        };
        if rc != dnflib_sys::DNF_OK {
            let _ = std::fs::remove_file(&temp_path);
            return Err(format!("dnflib save NPK failed: {rc}"));
        }
        std::fs::copy(&temp_path, output_path).map_err(|err| {
            format!(
                "failed to copy {} to {}: {err}",
                temp_path.display(),
                output_path.display()
            )
        })?;
        let _ = std::fs::remove_file(temp_path);
        Ok(())
    }
}

fn reserve_adjacent_temporary_path(parent: &Path) -> Result<PathBuf, String> {
    for attempt in 0..128_u32 {
        let candidate = parent.join(format!(
            ".dnf_avatar_studio_{}_{}_{}.npk.tmp",
            std::process::id(),
            unique_nanos(),
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("无法在输出目录创建临时 NPK".to_string()),
        }
    }
    Err("无法分配唯一的临时 NPK 文件名".to_string())
}

fn install_complete_file_create_new(
    temporary_path: &Path,
    output_path: &Path,
) -> Result<(), String> {
    match std::fs::hard_link(temporary_path, output_path) {
        Ok(()) => return Ok(()),
        Err(error)
            if error.kind() == ErrorKind::AlreadyExists
                || std::fs::symlink_metadata(output_path).is_ok() =>
        {
            return Err("输出 NPK 已存在，禁止覆盖".to_string());
        }
        Err(_) => {}
    }

    let mut source = File::open(temporary_path).map_err(|_| "无法读取临时 NPK".to_string())?;
    let mut output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return Err("输出 NPK 已存在，禁止覆盖".to_string());
        }
        Err(_) => return Err("无法创建输出 NPK".to_string()),
    };
    if std::io::copy(&mut source, &mut output).is_err() || output.sync_all().is_err() {
        drop(output);
        let _ = std::fs::remove_file(output_path);
        return Err("写入输出 NPK 失败".to_string());
    }
    Ok(())
}

impl Drop for NpkBuilder {
    fn drop(&mut self) {
        unsafe { dnflib_sys::dnf_npk_free(self.ptr.as_ptr()) };
    }
}

pub struct ImgArchive {
    ptr: NonNull<ImgFile>,
    _owned_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameInfo {
    pub index: usize,
    pub format: i32,
    pub compress: i32,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub link_index: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub canvas_width: i32,
    pub canvas_height: i32,
    pub rgba: Vec<u8>,
}

impl ImgArchive {
    pub fn open_owned_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.is_empty() {
            return Err("IMG 数据为空".to_string());
        }
        let mut out = std::ptr::null_mut();
        let rc = unsafe { dnflib_sys::dnf_img_open_mem(bytes.as_ptr(), bytes.len(), &mut out) };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib 打开内存 IMG 失败，错误码 {rc}"));
        }
        let ptr = NonNull::new(out).ok_or_else(|| "dnflib 返回了空 IMG 指针".to_string())?;
        Ok(Self {
            ptr,
            _owned_bytes: Some(bytes),
        })
    }

    pub fn version(&self) -> i32 {
        unsafe { self.ptr.as_ref().version }
    }

    pub fn frame_info(&self, index: usize) -> Option<FrameInfo> {
        let img = unsafe { self.ptr.as_ref() };
        if img.frames.is_null() || index >= self.frame_count() {
            return None;
        }
        let frame = unsafe { &*img.frames.add(index) };
        Some(FrameInfo {
            index,
            format: frame.format,
            compress: frame.compress,
            width: frame.w,
            height: frame.h,
            x: frame.x,
            y: frame.y,
            canvas_width: frame.mw,
            canvas_height: frame.mh,
            link_index: frame.link_index,
        })
    }

    pub fn frame_data_size(&self, index: usize) -> Option<i32> {
        let img = unsafe { self.ptr.as_ref() };
        if img.frames.is_null() || index >= self.frame_count() {
            return None;
        }
        Some(unsafe { (*img.frames.add(index)).data_size })
    }

    pub fn frame_count(&self) -> usize {
        unsafe { self.ptr.as_ref().frame_count.max(0) as usize }
    }

    pub fn frames(&self) -> Vec<FrameInfo> {
        (0..self.frame_count())
            .filter_map(|index| self.frame_info(index))
            .collect()
    }

    pub fn palette_count(&self) -> usize {
        let count = unsafe { dnflib_sys::dnf_img_palette_count(self.ptr.as_ptr()) };
        count.max(0) as usize
    }

    pub fn decode_frame(&self, frame_index: usize) -> Result<DecodedFrame, String> {
        self.decode_frame_with_palette(frame_index, 0)
    }

    pub fn decode_frame_with_palette(
        &self,
        frame_index: usize,
        palette_index: usize,
    ) -> Result<DecodedFrame, String> {
        let frame_info = self
            .frame_info(frame_index)
            .ok_or_else(|| format!("IMG 不存在第 {frame_index} 帧"))?;
        let mut out_rgba = std::ptr::null_mut();
        let mut width = 0;
        let mut height = 0;
        let rc = unsafe {
            dnflib_sys::dnf_image_decode_with_palette(
                self.ptr.as_ptr(),
                frame_index as i32,
                palette_index as i32,
                &mut out_rgba,
                &mut width,
                &mut height,
            )
        };
        if rc != dnflib_sys::DNF_OK {
            return Err(format!("dnflib 解码帧失败，错误码 {rc}"));
        }
        if out_rgba.is_null() || width <= 0 || height <= 0 {
            return Err("dnflib 返回了无效帧数据".to_string());
        }

        let len = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4));
        let Some(len) = len else {
            unsafe { dnflib_sys::dnf_free(out_rgba.cast::<c_void>()) };
            return Err("dnflib 返回的帧尺寸溢出".to_string());
        };
        let rgba = unsafe { std::slice::from_raw_parts(out_rgba, len).to_vec() };
        unsafe { dnflib_sys::dnf_free(out_rgba.cast::<c_void>()) };

        Ok(DecodedFrame {
            width,
            height,
            x: frame_info.x,
            y: frame_info.y,
            canvas_width: frame_info.canvas_width,
            canvas_height: frame_info.canvas_height,
            rgba,
        })
    }
}

impl Drop for ImgArchive {
    fn drop(&mut self) {
        unsafe { dnflib_sys::dnf_img_free(self.ptr.as_ptr()) };
    }
}

fn decode_entry_name(raw: &[std::os::raw::c_char; 256]) -> String {
    let bytes: Vec<u8> = raw
        .iter()
        .map(|byte| *byte as u8)
        .take_while(|byte| *byte != 0)
        .collect();
    let (decoded, _, had_errors) = EUC_KR.decode(&bytes);
    if had_errors {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        decoded.into_owned()
    }
}

fn unique_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}
