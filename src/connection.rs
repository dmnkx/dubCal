use crate::{browser, oauth::LoginProgress};
use eframe::egui::{self, RichText};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::Duration;

pub struct Connection {
    pub open: bool,
    pub active: bool,
    pub queued: bool,
    pub cancelled: Arc<AtomicBool>,
    status: String,
    url: Option<String>,
    error: Option<String>,
    browser_job: Option<mpsc::Receiver<Result<(), String>>>,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            open: false,
            active: false,
            queued: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            status: String::new(),
            url: None,
            error: None,
            browser_job: None,
        }
    }
}

impl Connection {
    pub fn begin(&mut self, queued: bool) {
        *self = Self {
            open: true,
            active: true,
            queued,
            status: if queued {
                "진행 중인 작업이 끝나면 Google 연결을 시작합니다…"
            } else {
                "Google 연결을 준비합니다…"
            }
            .into(),
            ..Default::default()
        };
    }
    pub fn progress(&mut self, progress: LoginProgress) {
        if !self.active || self.cancelled.load(Ordering::Relaxed) {
            return;
        }
        match progress {
            LoginProgress::Status(status) => self.status = status.into(),
            LoginProgress::Ready(url) => {
                self.url = Some(url);
                self.status = "브라우저에서 Google 로그인과 권한 승인을 완료해 주세요.".into();
            }
            LoginProgress::BrowserError(error) => self.error = Some(error),
        }
    }
    pub fn warning(&mut self, error: String) {
        self.error = Some(error);
    }
    pub fn finish(&mut self, error: Option<String>) {
        self.active = false;
        self.queued = false;
        self.browser_job = None;
        self.url = None; // The callback listener has closed; don't reuse expired links.
        self.status = if self.cancelled.load(Ordering::Relaxed) {
            "Google 연결을 취소했습니다."
        } else if error.is_none() {
            "Google 연결이 완료되었습니다."
        } else {
            "Google 연결을 완료하지 못했습니다."
        }
        .into();
        self.error = error;
        self.open = true;
    }
    pub fn poll_browser(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.browser_job {
            match job.try_recv() {
                Ok(result) => {
                    self.error = result.err();
                    self.browser_job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.error = Some(
                        "브라우저 실행 결과를 확인하지 못했습니다. 링크를 복사해 직접 열어 주세요."
                            .into(),
                    );
                    self.browser_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
    }
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }
        let mut retry = false;
        egui::Window::new("Google 연결").id(egui::Id::new("google_connection"))
            .title_bar(false).collapsible(false).resizable(false).default_width(430.0)
            .vscroll(true).max_height((ctx.screen_rect().height() - 80.0).max(160.0))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.style()).inner_margin(24).corner_radius(14))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                ui.spacing_mut().button_padding = egui::vec2(14.0, 9.0);
                ui.heading("Google Calendar 연결");
                ui.label(RichText::new("dubCal · 브라우저에서 안전하게 로그인").small().weak());
                ui.horizontal_wrapped(|ui| { if self.active { ui.spinner(); } ui.label(&self.status); });
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.small_button("오류 내용 복사").clicked() { ctx.copy_text(error.clone()); }
                }
                if let Some(url) = self.url.clone().filter(|_| self.active) {
                    ui.label("브라우저가 열리지 않으면 아래 버튼을 사용하세요.");
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(self.browser_job.is_none(), egui::Button::new("로그인 페이지 열기")).clicked() {
                            let (sender, receiver) = mpsc::channel();
                            std::thread::spawn(move || { let _ = sender.send(browser::open_login(&url)); });
                            self.browser_job = Some(receiver);
                            ctx.request_repaint_after(Duration::from_millis(100));
                        }
                        if ui.button("로그인 링크 복사").clicked() {
                            if let Some(url) = &self.url { ctx.copy_text(url.clone()); }
                        }
                    });
                    ui.small("복사한 링크를 브라우저 주소창에 붙여넣어도 연결됩니다.");
                }
                if self.active {
                    ui.small("테스트 중인 OAuth 앱은 Google Auth Platform의 테스트 사용자에 본인 계정이 등록되어 있어야 합니다.");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if self.active {
                        if ui.add_enabled(!self.cancelled.load(Ordering::Relaxed), egui::Button::new("연결 취소")).clicked() {
                            self.cancelled.store(true, Ordering::Relaxed);
                            self.status = "연결을 취소하는 중…".into();
                        }
                    } else if ui.button("다시 연결").clicked() { retry = true; }
                    if ui.button("닫기").clicked() { self.open = false; }
                });
            });
        retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_failures_remain_visible_and_expired_links_are_removed() {
        let mut connection = Connection::default();
        connection.begin(false);
        connection.progress(LoginProgress::Ready(
            "https://accounts.google.com/demo".into(),
        ));
        assert!(connection.open && connection.active && connection.url.is_some());
        connection.finish(Some("offline".into()));
        assert!(connection.open && !connection.active && connection.url.is_none());
        assert_eq!(connection.error.as_deref(), Some("offline"));
        // Cross-channel completion can arrive before the last progress update.
        connection.progress(LoginProgress::Ready(
            "https://accounts.google.com/expired".into(),
        ));
        assert!(connection.url.is_none());
    }
    #[test]
    fn queued_login_can_be_cancelled_without_starting_a_browser() {
        let mut connection = Connection::default();
        connection.begin(true);
        assert!(connection.active && connection.queued);
        connection.cancelled.store(true, Ordering::Relaxed);
        connection.finish(None);
        assert!(!connection.active && !connection.queued);
        assert!(connection.status.contains("취소"));
    }
}
