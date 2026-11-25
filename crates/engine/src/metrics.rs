//! Real multi-process OS memory measurement (summing browser + GPU + utility +
//! all renderer processes) and compressed disk-only tab thumbnails (JPEG).

use seu_core::TabId;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessTreeMemory {
    pub total_private_bytes: u64,
    pub total_working_set_bytes: u64,
    pub process_count: usize,
}

impl ProcessTreeMemory {
    pub fn total_private_mb(self) -> u32 {
        (self.total_private_bytes / (1024 * 1024)) as u32
    }

    pub fn total_working_set_mb(self) -> u32 {
        (self.total_working_set_bytes / (1024 * 1024)) as u32
    }
}

pub fn thumbnails_dir() -> PathBuf {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = root.join("SeuNavegador").join("thumbs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn thumbnail_path_for_tab(tab_id: TabId) -> PathBuf {
    thumbnails_dir().join(format!("tab_{tab_id}.jpg"))
}

pub fn remove_tab_thumbnail(tab_id: TabId) {
    let path = thumbnail_path_for_tab(tab_id);
    let _ = std::fs::remove_file(path);
}

pub fn read_tab_thumbnail_bytes(file_name: &str) -> Option<Vec<u8>> {
    let stem = file_name.strip_prefix("tab_")?.strip_suffix(".jpg")?;
    if stem.is_empty() || !stem.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let path = thumbnails_dir().join(file_name);
    std::fs::read(path).ok()
}

#[cfg(target_os = "windows")]
pub fn measure_browser_tree_memory() -> ProcessTreeMemory {
    use std::collections::{HashMap, HashSet, VecDeque};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    let root_pid = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return ProcessTreeMemory::default();
    }

    let mut children_by_parent: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    if unsafe { Process32FirstW(snapshot, &mut entry) } != 0 {
        loop {
            children_by_parent
                .entry(entry.th32ParentProcessID)
                .or_default()
                .push(entry.th32ProcessID);
            if unsafe { Process32NextW(snapshot, &mut entry) } == 0 {
                break;
            }
        }
    }
    unsafe {
        CloseHandle(snapshot);
    }

    // BFS over every descendant process of the main browser PID.
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(root_pid);
    queue.push_back(root_pid);

    while let Some(pid) = queue.pop_front() {
        if let Some(kids) = children_by_parent.get(&pid) {
            for &kid in kids {
                if visited.insert(kid) {
                    queue.push_back(kid);
                }
            }
        }
    }

    let mut total_private: u64 = 0;
    let mut total_ws: u64 = 0;
    let mut counted: usize = 0;

    for pid in visited {
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if handle.is_null() {
            continue;
        }
        let mut pmc: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = unsafe {
            GetProcessMemoryInfo(
                handle,
                (&mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
                pmc.cb,
            )
        };
        unsafe {
            CloseHandle(handle);
        }
        if ok != 0 {
            total_private = total_private.saturating_add(pmc.PrivateUsage as u64);
            total_ws = total_ws.saturating_add(pmc.WorkingSetSize as u64);
            counted += 1;
        }
    }

    ProcessTreeMemory {
        total_private_bytes: total_private,
        total_working_set_bytes: total_ws,
        process_count: counted,
    }
}

#[cfg(not(target_os = "windows"))]
pub fn measure_browser_tree_memory() -> ProcessTreeMemory {
    ProcessTreeMemory::default()
}

/// Captures a child tab HWND into a compact compressed JPEG thumbnail on disk
/// (`240x150`, quality 65) and immediately drops the raw pixel buffer from RAM.
#[cfg(target_os = "windows")]
pub fn capture_hwnd_thumbnail_jpeg(raw_hwnd: isize, tab_id: TabId) -> bool {
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    const PW_RENDERFULLCONTENT: u32 = 0x0000_0002;

    if raw_hwnd == 0 {
        return false;
    }
    let hwnd = raw_hwnd as HWND;

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return false;
    }
    let src_w = (rect.right - rect.left).max(0) as usize;
    let src_h = (rect.bottom - rect.top).max(0) as usize;
    if src_w < 32 || src_h < 32 || src_w > 8192 || src_h > 8192 {
        return false;
    }

    let bgra = unsafe {
        let win_dc = GetDC(hwnd);
        if win_dc.is_null() {
            return false;
        }
        let mem_dc = CreateCompatibleDC(win_dc);
        if mem_dc.is_null() {
            ReleaseDC(hwnd, win_dc);
            return false;
        }
        let bmp = CreateCompatibleBitmap(win_dc, src_w as i32, src_h as i32);
        if bmp.is_null() {
            DeleteDC(mem_dc);
            ReleaseDC(hwnd, win_dc);
            return false;
        }
        let old = SelectObject(mem_dc, bmp);

        let printed = PrintWindow(hwnd, mem_dc, PW_RENDERFULLCONTENT);
        if printed == 0 {
            let _ = BitBlt(
                mem_dc,
                0,
                0,
                src_w as i32,
                src_h as i32,
                win_dc,
                0,
                0,
                SRCCOPY,
            );
        }

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: src_w as i32,
                biHeight: -(src_h as i32), // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [std::mem::zeroed(); 1],
        };

        let mut buf = vec![0u8; src_w * src_h * 4];
        let lines = GetDIBits(
            mem_dc,
            bmp,
            0,
            src_h as u32,
            buf.as_mut_ptr().cast(),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        DeleteObject(bmp);
        DeleteDC(mem_dc);
        ReleaseDC(hwnd, win_dc);

        if lines == 0 {
            return false;
        }
        buf
    };

    // Downscale to compact 240x150 RGB and encode as compressed JPEG directly to disk.
    let dst_w: usize = 240;
    let dst_h: usize = 150;
    let mut rgb = Vec::with_capacity(dst_w * dst_h * 3);
    for y in 0..dst_h {
        let sy = (y * src_h) / dst_h;
        for x in 0..dst_w {
            let sx = (x * src_w) / dst_w;
            let idx = (sy * src_w + sx) * 4;
            if idx + 2 < bgra.len() {
                rgb.push(bgra[idx + 2]); // R
                rgb.push(bgra[idx + 1]); // G
                rgb.push(bgra[idx]); // B
            } else {
                rgb.extend_from_slice(&[7, 11, 36]);
            }
        }
    }

    let path = thumbnail_path_for_tab(tab_id);
    let Ok(encoder) = jpeg_encoder::Encoder::new_file(&path, 65) else {
        return false;
    };
    encoder
        .encode(
            &rgb,
            dst_w as u16,
            dst_h as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .is_ok()
}

#[cfg(not(target_os = "windows"))]
pub fn capture_hwnd_thumbnail_jpeg(_raw_hwnd: isize, _tab_id: TabId) -> bool {
    false
}
