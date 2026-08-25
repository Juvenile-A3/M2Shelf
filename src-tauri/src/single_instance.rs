#[cfg(windows)]
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE},
};

#[cfg(windows)]
pub(crate) struct SingleInstanceGuard(HANDLE);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle was returned by CreateMutexW and is owned by this guard.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(windows)]
pub(crate) fn acquire() -> Result<Option<SingleInstanceGuard>, String> {
    let name = wide("Local\\M2Shelf-app.morimediashelf.desktop-v1");
    // SAFETY: the pointer is NUL terminated and remains valid for this call.
    let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(format!(
            "无法创建 M²Shelf 单实例锁：{}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: GetLastError is read immediately after CreateMutexW as required by Win32.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: this process owns the returned handle even when the mutex already exists.
        unsafe { CloseHandle(handle) };
        Ok(None)
    } else {
        Ok(Some(SingleInstanceGuard(handle)))
    }
}

#[cfg(windows)]
pub(crate) fn focus_existing_instance() {
    let title = wide("M²Shelf");
    // SAFETY: the title is NUL terminated. We only pass the returned HWND back to Win32.
    unsafe {
        let window = FindWindowW(ptr::null(), title.as_ptr());
        if !window.is_null() {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window);
        }
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(not(windows))]
pub(crate) struct SingleInstanceGuard;

#[cfg(not(windows))]
pub(crate) fn acquire() -> Result<Option<SingleInstanceGuard>, String> {
    Ok(Some(SingleInstanceGuard))
}

#[cfg(not(windows))]
pub(crate) fn focus_existing_instance() {}
