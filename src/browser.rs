/// Open OAuth HTTPS URLs through Windows' registered URL handler. Avoid parsing
/// the browser's registry command line or passing the URL through a shell.
pub fn open_login(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "로그인 주소가 올바르지 않습니다".to_string())?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("accounts.google.com") {
        return Err("Google 로그인 주소만 열 수 있습니다".into());
    }
    platform_open(url)
}

#[cfg(windows)]
fn platform_open(url: &str) -> Result<(), String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        },
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let open: Vec<u16> = "open\0".encode_utf16().collect();
    unsafe {
        let initialized = CoInitializeEx(
            null(),
            (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
        );
        let result = ShellExecuteW(
            null_mut(),
            open.as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        ) as isize;
        if initialized >= 0 {
            CoUninitialize();
        }
        if result <= 32 {
            Err(format!("기본 브라우저를 열지 못했습니다 (Windows 코드 {result}). 로그인 링크를 복사해 브라우저 주소창에 붙여넣어 주세요."))
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
fn platform_open(url: &str) -> Result<(), String> {
    webbrowser::open(url).map_err(|_| {
        "기본 브라우저를 열지 못했습니다. 로그인 링크를 복사해 직접 열어 주세요.".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn never_launches_untrusted_or_non_https_login_urls() {
        for url in [
            "file:///C:/test.exe",
            "http://accounts.google.com/",
            "https://accounts.google.com.evil.example/",
            "not a url",
        ] {
            assert!(open_login(url).is_err());
        }
    }
}
