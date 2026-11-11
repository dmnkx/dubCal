use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

// Deliberately omit Debug: avoid logging credentials in diagnostics.
#[derive(Deserialize)]
pub struct DesktopClient {
    pub client_id: String,
    pub client_secret: String,
}
#[derive(Deserialize)]
struct Credentials {
    installed: DesktopClient,
}

fn parse(bytes: &[u8]) -> Result<DesktopClient> {
    // Do not include serde's error text, which may contain user-supplied values.
    let config = serde_json::from_slice::<Credentials>(bytes)
        .map_err(|_| anyhow::anyhow!("Google OAuth 데스크톱 앱 설정 형식이 올바르지 않습니다"))?;
    anyhow::ensure!(
        !config.installed.client_id.trim().is_empty()
            && !config.installed.client_secret.trim().is_empty(),
        "Google OAuth 클라이언트 정보가 비어 있습니다"
    );
    Ok(config.installed)
}

pub fn load(explicit: Option<&Path>, embedded: &[u8], fallback: &Path) -> Result<DesktopClient> {
    if let Some(path) = explicit {
        return parse(&std::fs::read(path).context("지정한 OAuth JSON 파일을 열 수 없습니다")?);
    }
    if !embedded.is_empty() {
        return parse(embedded);
    }
    parse(&std::fs::read(fallback).context("Google 연결 설정이 없습니다. OAuth 설정을 포함한 dubCal 빌드 또는 google-oauth.json이 필요합니다")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    const DEMO: &[u8] =
        br#"{"installed":{"client_id":"demo-client","client_secret":"demo-value"}}"#;
    #[test]
    fn embedded_configuration_works_without_a_json_file() {
        let client = load(None, DEMO, Path::new("/missing/oauth.json")).unwrap();
        assert_eq!(client.client_id, "demo-client");
    }
    #[test]
    fn explicit_file_is_not_silently_ignored() {
        assert!(load(
            Some(Path::new("/missing/override.json")),
            DEMO,
            Path::new("unused")
        )
        .is_err());
    }
    #[test]
    fn rejects_wrong_client_types_and_empty_fields_without_echoing_values() {
        assert!(parse(br#"{"web":{"client_id":"demo"}}"#).is_err());
        assert!(parse(br#"{"installed":{"client_id":"","client_secret":"demo"}}"#).is_err());
        let message = parse(br#"{"installed":{"client_id":123,"client_secret":"DO_NOT_ECHO"}}"#)
            .err()
            .unwrap()
            .to_string();
        assert!(!message.contains("DO_NOT_ECHO"));
        assert!(load(None, b"", Path::new("/missing/oauth.json")).is_err());
    }
}

pub enum LoginProgress {
    Status(&'static str),
    Ready(String),
    BrowserError(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Callback {
    Ignore,
    Denied,
    Code(String),
}

/// Reject unrelated requests and mismatched state without aborting a valid login.
pub fn callback(request: &str, state: &str) -> Callback {
    let mut parts = request.split_whitespace();
    if parts.next() != Some("GET") {
        return Callback::Ignore;
    }
    let Some(path) = parts.next().filter(|path| path.starts_with("/?")) else {
        return Callback::Ignore;
    };
    let Ok(url) = url::Url::parse(&format!("http://localhost{path}")) else {
        return Callback::Ignore;
    };
    let parameters: std::collections::HashMap<_, _> = url.query_pairs().collect();
    if parameters.get("state").map(|value| value.as_ref()) != Some(state) {
        return Callback::Ignore;
    }
    if parameters.contains_key("error") {
        return Callback::Denied;
    }
    match parameters.get("code").filter(|code| !code.is_empty()) {
        Some(code) => Callback::Code(code.to_string()),
        None => Callback::Ignore,
    }
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    #[test]
    fn only_accepts_the_current_login_session() {
        assert_eq!(
            callback("GET /?state=expected&code=a%2Bb HTTP/1.1", "expected"),
            Callback::Code("a+b".into())
        );
        for request in [
            "GET /?state=wrong&code=x HTTP/1.1",
            "GET /?code=x HTTP/1.1",
            "GET /favicon.ico HTTP/1.1",
            "POST /?state=expected&code=x HTTP/1.1",
        ] {
            assert_eq!(callback(request, "expected"), Callback::Ignore);
        }
    }
    #[test]
    fn denial_is_acknowledged_and_empty_codes_are_ignored() {
        assert_eq!(
            callback(
                "GET /?state=expected&error=access_denied HTTP/1.1",
                "expected"
            ),
            Callback::Denied
        );
        assert_eq!(
            callback("GET /?state=expected&code= HTTP/1.1", "expected"),
            Callback::Ignore
        );
    }
}

pub fn token_error(code: &str, status: u16) -> String {
    let reason = match code {
        "invalid_client" | "unauthorized_client" => {
            "OAuth 클라이언트 ID/보안 비밀번호 또는 데스크톱 앱 유형을 확인해 주세요"
        }
        "invalid_grant" => "로그인이 만료되었거나 취소되었습니다. Google 연결을 다시 진행해 주세요",
        "access_denied" | "org_internal" => {
            "Google 접근 권한과 OAuth 대상/테스트 사용자 등록을 확인해 주세요"
        }
        "invalid_scope" => "Google Calendar 접근 권한 설정을 확인해 주세요",
        "redirect_uri_mismatch" => {
            "데스크톱 앱 OAuth 클라이언트와 로그인 응답 주소를 확인해 주세요"
        }
        _ => "Google 인증 서버가 요청을 거부했습니다. 잠시 후 다시 연결해 주세요",
    };
    format!("{reason} (HTTP {status})")
}
