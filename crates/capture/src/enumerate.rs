//! Enumeração de janelas e monitores capturáveis.

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MONITORINFOEXW, MonitorFromWindow,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetWindow, GetWindowLongW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    WS_CAPTION, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

use crate::heuristics::{WindowTraits, likely_game};
use crate::{CaptureError, CaptureTarget, SourceDescriptor};

/// Lista janelas (prováveis jogos primeiro) e depois monitores.
pub fn list_sources() -> Result<Vec<SourceDescriptor>, CaptureError> {
    let mut windows = enumerate_windows();
    windows.sort_by_key(|w| (!w.likely_game, w.title.to_lowercase()));
    let mut all = windows;
    all.extend(enumerate_monitors());
    Ok(all)
}

fn enumerate_windows() -> Vec<SourceDescriptor> {
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: o callback só escreve no Vec apontado por `lparam`, que vive durante a chamada.
    unsafe {
        let _ = EnumWindows(
            Some(collect_window),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        );
    }
    // SAFETY: consulta do PID do próprio processo.
    let own_pid = unsafe { GetCurrentProcessId() };
    handles
        .into_iter()
        .filter_map(|h| describe_window(h, own_pid))
        .collect()
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: `lparam` aponta para o Vec<HWND> passado por `enumerate_windows`.
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    true.into()
}

fn describe_window(hwnd: HWND, own_pid: u32) -> Option<SourceDescriptor> {
    // SAFETY: consultas somente leitura sobre um HWND vindo do EnumWindows; a janela
    // pode ter sido destruída nesse meio-tempo, caso em que as chamadas falham sem efeito.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return None;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok()
            && cloaked != 0
        {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if title.trim().is_empty() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own_pid {
            return None;
        }
        let mut client = RECT::default();
        GetClientRect(hwnd, &mut client).ok()?;
        let width = (client.right - client.left).max(0) as u32;
        let height = (client.bottom - client.top).max(0) as u32;
        if width < 64 || height < 64 {
            return None;
        }
        let process_name = process_name(pid);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let traits = WindowTraits {
            exe: process_name.as_deref(),
            covers_monitor: covers_monitor(hwnd),
            has_caption: style & WS_CAPTION.0 == WS_CAPTION.0,
            width,
            height,
        };
        Some(SourceDescriptor {
            target: CaptureTarget::Window(hwnd.0 as isize),
            likely_game: likely_game(traits),
            title,
            process_name,
            process_id: Some(pid),
            width,
            height,
        })
    }
}

fn covers_monitor(hwnd: HWND) -> bool {
    // SAFETY: consultas somente leitura de geometria.
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: handle aberto com acesso mínimo e fechado antes de retornar.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_owned)
    }
}

fn enumerate_monitors() -> Vec<SourceDescriptor> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    // SAFETY: o callback só escreve no Vec apontado por `lparam`, que vive durante a chamada.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize),
        );
    }
    monitors
        .into_iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            // SAFETY: estrutura com cbSize correto para MONITORINFOEXW.
            let ok = unsafe { GetMonitorInfoW(m, &mut info.monitorInfo) }.as_bool();
            if !ok {
                return None;
            }
            let r = info.monitorInfo.rcMonitor;
            let width = (r.right - r.left).max(0) as u32;
            let height = (r.bottom - r.top).max(0) as u32;
            let device = String::from_utf16_lossy(&info.szDevice)
                .trim_end_matches('\0')
                .to_owned();
            let primary = info.monitorInfo.dwFlags & 1 != 0;
            let title = format!(
                "Monitor {}{} ({}x{}) {}",
                i + 1,
                if primary { " — principal" } else { "" },
                width,
                height,
                device
            );
            Some(SourceDescriptor {
                target: CaptureTarget::Monitor(m.0 as isize),
                title: title.trim().to_owned(),
                process_name: None,
                process_id: None,
                width,
                height,
                likely_game: false,
            })
        })
        .collect()
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    // SAFETY: `lparam` aponta para o Vec<HMONITOR> passado por `enumerate_monitors`.
    let monitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    true.into()
}
