pub use crate::event::Event;
use crate::oauth::{self, DesktopClient, LoginProgress};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{Days, NaiveDate};
use rand::{rngs::OsRng, RngCore};
use reqwest::{blocking::Client, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const TOKEN: &str = "https://oauth2.googleapis.com/token";

#[derive(Deserialize)]
struct Token {
    access_token: String,
    refresh_token: Option<String>,
}

fn read_token(response: reqwest::blocking::Response) -> Result<Token> {
    let status = response.status();
    if !status.is_success() {
        let value = response.json::<serde_json::Value>().unwrap_or_default();
        let code = value
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        bail!("{}", oauth::token_error(code, status.as_u16()));
    }
    response
        .json()
        .context("Google 로그인 응답을 해석하지 못했습니다")
}

fn calendar_response(response: reqwest::blocking::Response) -> Result<reqwest::blocking::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.json::<serde_json::Value>().unwrap_or_default();
    bail!("{}", crate::api_error::message(status.as_u16(), &body));
}

pub struct Google {
    http: Client,
    client: DesktopClient,
    access: Option<(String, Instant)>,
}

impl Google {
    pub fn new(path: Option<&Path>) -> Result<Self> {
        const EMBEDDED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/oauth_embedded.json"));
        let fallback = std::env::current_exe()
            .map(|path| path.with_file_name("google-oauth.json"))
            .unwrap_or_else(|_| std::path::PathBuf::from("google-oauth.json"));
        let client = oauth::load(path, EMBEDDED, &fallback)?;
        Ok(Self {
            http: Client::builder().timeout(Duration::from_secs(20)).build()?,
            client,
            access: None,
        })
    }

    fn entry(&self) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(
            "Desk Calendar Google OAuth",
            &self.client.client_id,
        )?)
    }

    pub fn connect(
        &mut self,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(LoginProgress),
    ) -> Result<()> {
        if cancelled.load(Ordering::Relaxed) {
            bail!("Google 연결을 취소했습니다");
        }
        progress(LoginProgress::Status("로그인 요청을 준비합니다…"));
        let listener = TcpListener::bind("127.0.0.1:0")
            .context("Google 로그인 응답을 받을 로컬 포트를 열 수 없습니다")?;
        listener.set_nonblocking(true)?;
        let redirect = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
        let verifier = random();
        let state = random();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut auth = Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
        auth.query_pairs_mut().extend_pairs([
            ("client_id", self.client.client_id.as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            (
                "scope",
                "https://www.googleapis.com/auth/calendar.events.owned",
            ),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ]);
        progress(LoginProgress::Ready(auth.to_string()));
        if let Err(error) = crate::browser::open_login(auth.as_str()) {
            progress(LoginProgress::BrowserError(error));
        }
        let deadline = Instant::now() + Duration::from_secs(180);
        let code = loop {
            if cancelled.load(Ordering::Relaxed) {
                bail!("Google 연결을 취소했습니다");
            }
            if Instant::now() >= deadline {
                bail!("Google 로그인 시간이 초과됐습니다");
            }
            let (mut stream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
            let mut line = String::new();
            use std::io::Read;
            if BufReader::new(&stream)
                .take(8192)
                .read_line(&mut line)
                .is_err()
            {
                continue;
            }
            let code = match oauth::callback(&line, &state) {
                oauth::Callback::Ignore => {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                oauth::Callback::Denied => {
                    let body = "Google authorization was not granted. Return to dubCal.";
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
                    bail!("Google 접근 권한이 허용되지 않았습니다. 권한 승인과 테스트 사용자 등록을 확인해 주세요");
                }
                oauth::Callback::Code(code) => code,
            };
            let body = "Authorization received. Return to dubCal.";
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            break code;
        };
        if cancelled.load(Ordering::Relaxed) {
            bail!("Google 연결을 취소했습니다");
        }
        progress(LoginProgress::Status(
            "Google 승인을 확인하고 로그인 정보를 저장합니다…",
        ));
        let response = self
            .http
            .post(TOKEN)
            .form(&[
                ("client_id", self.client.client_id.as_str()),
                ("client_secret", self.client.client_secret.as_str()),
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &redirect),
                ("code_verifier", &verifier),
            ])
            .send()
            .context(
                "Google 로그인 서버에 연결하지 못했습니다. 네트워크/프록시 설정을 확인해 주세요",
            )?;
        let token = read_token(response)?;
        if cancelled.load(Ordering::Relaxed) {
            bail!("Google 연결을 취소했습니다");
        }
        let refresh = token
            .refresh_token
            .context("Google이 갱신 토큰을 반환하지 않았습니다")?;
        self.entry()?
            .set_password(&refresh)
            .context("Windows 자격 증명 관리자에 로그인 정보를 저장하지 못했습니다")?;
        self.access = Some((token.access_token, Instant::now()));
        Ok(())
    }

    fn access(&mut self) -> Result<String> {
        if let Some((token, issued)) = &self.access {
            if issued.elapsed() < Duration::from_secs(3000) {
                return Ok(token.clone());
            }
        }
        let refresh = self
            .entry()?
            .get_password()
            .context("Google 연결 버튼으로 로그인하세요")?;
        let response = self
            .http
            .post(TOKEN)
            .form(&[
                ("client_id", self.client.client_id.as_str()),
                ("client_secret", self.client.client_secret.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", &refresh),
            ])
            .send()
            .context("Google 로그인 갱신 서버에 연결하지 못했습니다")?;
        let token = read_token(response)?;
        if let Some(refresh) = token.refresh_token {
            self.entry()?.set_password(&refresh)?;
        }
        self.access = Some((token.access_token.clone(), Instant::now()));
        Ok(token.access_token)
    }

    pub fn events(&mut self, start: NaiveDate, end: NaiveDate) -> Result<Vec<Event>> {
        use chrono::TimeZone;
        let bound = |date: NaiveDate| -> Result<String> {
            // Fetch with a one-day margin for all-day events in other time zones.
            Ok(chrono::Utc
                .from_utc_datetime(&date.and_time(chrono::NaiveTime::MIN))
                .to_rfc3339())
        };
        let min = bound(start - Days::new(1))?;
        let max = bound(end + Days::new(1))?;
        let token = self.access()?;
        let url = endpoint(None)?;
        let mut events = Vec::new();
        let mut next = None::<String>;
        loop {
            let mut request = self.http.get(url.clone()).bearer_auth(&token).query(&[
                ("timeMin", min.as_str()),
                ("timeMax", max.as_str()),
                ("singleEvents", "true"),
                ("orderBy", "startTime"),
                (
                    "fields",
                    "nextPageToken,items(id,summary,description,location,colorId,visibility,transparency,reminders,start,end,recurringEventId)",
                ),
            ]);
            if let Some(page) = &next {
                request = request.query(&[("pageToken", page)]);
            }
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Page {
                #[serde(default)]
                items: Vec<Event>,
                next_page_token: Option<String>,
            }
            let page: Page = calendar_response(request.send()?)?.json()?;
            events.extend(page.items);
            next = page.next_page_token;
            if next.is_none() {
                return Ok(events);
            }
        }
    }

    pub fn save(&mut self, id: Option<&str>, payload: &serde_json::Value) -> Result<()> {
        let token = self.access()?;
        let url = endpoint(id)?;
        let request = if id.is_some() {
            self.http.patch(url)
        } else {
            self.http.post(url)
        };
        calendar_response(request.json(payload).bearer_auth(token).send()?)?;
        Ok(())
    }

    pub fn delete(&mut self, id: &str) -> Result<()> {
        let token = self.access()?;
        calendar_response(
            self.http
                .delete(endpoint(Some(id))?)
                .bearer_auth(token)
                .send()?,
        )?;
        Ok(())
    }
}

fn endpoint(id: Option<&str>) -> Result<Url> {
    let mut url = Url::parse("https://www.googleapis.com/calendar/v3/calendars/primary/events")?;
    if let Some(id) = id {
        url.path_segments_mut().expect("hierarchical URL").push(id);
    }
    Ok(url)
}

fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
