use std::{io, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MessageBoxW},
};

const MUTEX_NAME: &str = "Local\\Vermouth.Desktop.SingleInstance.9F3A2D61";

pub struct InstanceGuard {
    handle: HANDLE,
}

impl InstanceGuard {
    pub fn acquire() -> io::Result<Option<Self>> {
        let name = wide(MUTEX_NAME);

        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };

        let error = unsafe { GetLastError() };

        if handle.is_null() {
            return Err(io::Error::from_raw_os_error(error as i32));
        }

        if error == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }

        Ok(Some(Self { handle }))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}

pub fn show_message(message: &str, is_error: bool) {
    let title = wide("Vermouth");
    let message = wide(message);

    let style = MB_OK
        | if is_error {
            MB_ICONERROR
        } else {
            MB_ICONINFORMATION
        };

    unsafe {
        MessageBoxW(ptr::null_mut(), message.as_ptr(), title.as_ptr(), style);
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}
