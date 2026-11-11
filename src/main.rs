#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_error;
mod appearance;
mod browser;
mod connection;
mod desktop;
mod event;
mod geometry;
mod google;
mod oauth;
mod pickers;
mod schedule;

use appearance::{Appearance, Theme};
use chrono::{Datelike, Days, Local, Months, NaiveDate};
use eframe::egui::{self, RichText};
use google::{Event, Google};
use schedule::{EventDraft, Reminder, EVENT_COLORS};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::{Duration, Instant},
};

type Range = (NaiveDate, NaiveDate);
type JobResult = anyhow::Result<JobOutput>;
struct JobOutput {
    events: Option<Vec<Event>>,
    refresh_error: Option<String>,
}

#[derive(Clone, Copy)]
enum MenuAction {
    PreviousMonth,
    NextMonth,
    Today,
    Connect,
    Refresh,
    Settings,
    RetryDesktop,
    Close,
}

enum Action {
    Load,
    Connect {
        cancelled: Arc<AtomicBool>,
    },
    Save {
        id: Option<String>,
        payload: serde_json::Value,
    },
    Delete(String),
}

struct Editor {
    id: Option<String>,
    draft: EventDraft,
    confirm_delete: bool,
    focus_title: bool,
    pending: bool,
    error: Option<String>,
    dates: [pickers::DatePicker; 2],
    times: [pickers::TimePicker; 2],
}

impl Editor {
    fn new(date: NaiveDate) -> Self {
        Self {
            id: None,
            draft: EventDraft::new(date),
            confirm_delete: false,
            focus_title: true,
            pending: false,
            error: None,
            dates: Default::default(),
            times: Default::default(),
        }
    }

    fn from_event(event: &Event, date: NaiveDate) -> Self {
        Self {
            id: Some(event.id.clone()),
            draft: event.draft(date),
            confirm_delete: false,
            focus_title: true,
            pending: false,
            error: None,
            dates: Default::default(),
            times: Default::default(),
        }
    }
}

struct Calendar {
    month: NaiveDate,
    events: Vec<Event>,
    loaded: Option<Range>,
    job: Option<mpsc::Receiver<JobResult>>,
    worker: mpsc::Sender<(Action, Range, mpsc::Sender<JobResult>)>,
    editor: Option<Editor>,
    error: Option<String>,
    refreshed: Instant,
    desktop: desktop::Desktop,
    appearance: Appearance,
    settings_open: bool,
    settings_dirty: bool,
    settings_error: Option<String>,
    applied_theme: Option<Theme>,
    menu_position: egui::Pos2,
    connection: connection::Connection,
    login_progress: mpsc::Receiver<oauth::LoginProgress>,
    job_is_connect: bool,
}

fn menu_card(
    ui: &mut egui::Ui,
    title: &str,
    subtitle: &str,
    palette: &appearance::Palette,
    destructive: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 60.0), egui::Sense::click());
    let fill = if response.hovered() || response.has_focus() || response.is_pointer_button_down_on()
    {
        palette.today
    } else {
        palette.cell
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    let title_color = if !ui.is_enabled() {
        palette.muted
    } else if destructive {
        ui.visuals().error_fg_color
    } else {
        palette.text
    };
    ui.painter().text(
        rect.min + egui::vec2(14.0, 10.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(16.0),
        title_color,
    );
    ui.painter().text(
        rect.min + egui::vec2(14.0, 35.0),
        egui::Align2::LEFT_TOP,
        subtitle,
        egui::FontId::proportional(11.0),
        palette.muted,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), title)
    });
    if ui.is_enabled() {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([840.0, 580.0])
            .with_min_inner_size([700.0, 500.0])
            .with_decorations(false)
            .with_taskbar(false)
            .with_resizable(false)
            .with_position([40.0, 60.0]),
        ..Default::default()
    };
    eframe::run_native(
        "dubCal",
        options,
        Box::new(move |cc| {
            install_font(&cc.egui_ctx);
            Ok(Box::new(Calendar::new(path)))
        }),
    )
}

fn install_font(ctx: &egui::Context) {
    let windows = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    if let Ok(font) = std::fs::read(windows.join("Fonts").join("malgun.ttf")) {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("malgun".into(), egui::FontData::from_owned(font).into());
        for family in fonts.families.values_mut() {
            family.push("malgun".into());
        }
        ctx.set_fonts(fonts);
    }
}

impl Calendar {
    fn new(path: Option<PathBuf>) -> Self {
        let (worker, inbox) = mpsc::channel::<(Action, Range, mpsc::Sender<JobResult>)>();
        let (login_updates, login_progress) = mpsc::channel();
        std::thread::spawn(move || {
            let mut google = None;
            while let Ok((action, (start, end), reply)) = inbox.recv() {
                let result = (|| {
                    if google.is_none() {
                        google = Some(Google::new(path.as_deref())?);
                    }
                    let google = google.as_mut().unwrap();
                    let loading = matches!(&action, Action::Load);
                    match action {
                        Action::Load => {}
                        Action::Connect { cancelled } => {
                            google.connect(&cancelled, |progress| {
                                let _ = login_updates.send(progress);
                            })?
                        }
                        Action::Save { id, payload } => google.save(id.as_deref(), &payload)?,
                        Action::Delete(id) => google.delete(&id)?,
                    }
                    match google.events(start, end) {
                        Ok(events) => Ok(JobOutput { events: Some(events), refresh_error: None }),
                        Err(error) if !loading => Ok(JobOutput { events: None,
                            refresh_error: Some(format!("작업은 완료했지만 달력 갱신에 실패했습니다. 새로고침해 주세요: {error:#}")) }),
                        Err(error) => Err(error),
                    }
                })();
                let _ = reply.send(result);
            }
        });
        let (appearance, settings_error) = Appearance::load();
        let today = Local::now().date_naive();
        let mut calendar = Self {
            month: today.with_day(1).unwrap(),
            events: Vec::new(),
            loaded: None,
            job: None,
            worker,
            editor: None,
            error: None,
            refreshed: Instant::now(),
            desktop: desktop::Desktop::default(),
            appearance,
            settings_error,
            settings_open: false,
            settings_dirty: false,
            applied_theme: None,
            menu_position: egui::Pos2::ZERO,
            connection: connection::Connection::default(),
            login_progress,
            job_is_connect: false,
        };
        calendar.send(Action::Load);
        calendar
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("dubCal 설정")
            .open(&mut open)
            .resizable(false)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.label("테마");
                egui::ComboBox::from_id_salt("theme")
                    .selected_text(self.appearance.theme.name())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (heading, themes) in
                            [("라이트", &Theme::ALL[..5]), ("다크", &Theme::ALL[5..])]
                        {
                            ui.strong(heading);
                            for &theme in themes {
                                let color = theme.palette().accent;
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, "●");
                                    self.settings_dirty |= ui
                                        .selectable_value(
                                            &mut self.appearance.theme,
                                            theme,
                                            theme.name(),
                                        )
                                        .changed();
                                });
                            }
                            ui.separator();
                        }
                    });
                ui.add_space(10.0);
                self.settings_dirty |= ui
                    .add(
                        egui::Slider::new(&mut self.appearance.transparency, 0..=80)
                            .text("투명도")
                            .suffix("%"),
                    )
                    .changed();
                ui.small("0%: 불투명 · 80%: 가장 투명");
                ui.small("글자와 버튼을 포함한 위젯 전체에 적용됩니다.");
                ui.add_space(8.0);
                if ui.button("기본값 복원").clicked() {
                    self.appearance = Appearance {
                        geometry: self.appearance.geometry,
                        ..Default::default()
                    };
                    self.settings_dirty = true;
                }
                if let Some(error) = self.desktop.opacity_error() {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(error) = &self.settings_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.small("설정은 자동 저장됩니다. 이동: Alt + 마우스 드래그");
            });
        self.settings_open = open;
    }

    fn apply_appearance(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.appearance.theme) {
            self.appearance.theme.apply(ctx);
            self.applied_theme = Some(self.appearance.theme);
            ctx.request_repaint();
        }
        self.desktop.set_opacity(self.appearance.opacity());
    }

    fn show_menu(&mut self, ui: &mut egui::Ui, opened_now: bool) {
        let popup_id = egui::Id::new("widget_context_menu");
        if !ui.memory(|memory| memory.is_popup_open(popup_id)) {
            return;
        }
        let ctx = ui.ctx().clone();
        let palette = self.appearance.theme.palette();
        let mut action = None;
        let mut close = false;
        let response = egui::Area::new(popup_id)
            .kind(egui::UiKind::Popup)
            .order(egui::Order::Foreground)
            .fixed_pos(self.menu_position)
            .constrain_to(ctx.screen_rect())
            .default_width(340.0)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(20)
                    .corner_radius(14)
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 12.0);
                        ui.spacing_mut().button_padding = egui::vec2(12.0, 8.0);
                        ui.horizontal(|ui| {
                            ui.heading("dubCal");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("닫기").clicked() {
                                        close = true;
                                    }
                                },
                            );
                        });
                        ui.label(
                            RichText::new(self.month.format("%Y년 %m월").to_string())
                                .small()
                                .color(palette.muted),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("widget_menu_scroll")
                            .max_height((ctx.screen_rect().height() - 180.0).max(120.0))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new("날짜 이동")
                                        .small()
                                        .strong()
                                        .color(palette.muted),
                                );
                                ui.add_enabled_ui(self.job.is_none(), |ui| {
                                    let width = (ui.available_width()
                                        - 2.0 * ui.spacing().item_spacing.x)
                                        / 3.0;
                                    ui.horizontal(|ui| {
                                        for (label, value) in [
                                            ("이전 달", MenuAction::PreviousMonth),
                                            ("이번 달", MenuAction::Today),
                                            ("다음 달", MenuAction::NextMonth),
                                        ] {
                                            if ui
                                                .add_sized(
                                                    [width, 38.0],
                                                    egui::Button::new(label).corner_radius(8),
                                                )
                                                .clicked()
                                            {
                                                action = Some(value);
                                            }
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("GOOGLE CALENDAR")
                                        .small()
                                        .strong()
                                        .color(palette.muted),
                                );
                                if menu_card(
                                    ui,
                                    "Google 연결",
                                    "계정을 연결하거나 다시 로그인합니다",
                                    &palette,
                                    false,
                                )
                                .clicked()
                                {
                                    action = Some(MenuAction::Connect);
                                }
                                ui.add_enabled_ui(self.job.is_none(), |ui| {
                                    if menu_card(
                                        ui,
                                        "새로고침",
                                        "Google Calendar의 최신 일정을 불러옵니다",
                                        &palette,
                                        false,
                                    )
                                    .clicked()
                                    {
                                        action = Some(MenuAction::Refresh);
                                    }
                                });
                                if self.job.is_some() {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label("일정 처리 중…");
                                    });
                                }
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("위젯 관리")
                                        .small()
                                        .strong()
                                        .color(palette.muted),
                                );
                                if menu_card(
                                    ui,
                                    "설정",
                                    "테마와 투명도를 변경합니다",
                                    &palette,
                                    false,
                                )
                                .clicked()
                                {
                                    action = Some(MenuAction::Settings);
                                }
                                if menu_card(ui, "종료", "dubCal 위젯을 닫습니다", &palette, true)
                                    .clicked()
                                {
                                    action = Some(MenuAction::Close);
                                }
                                if let Some(error) = self.desktop.error() {
                                    ui.colored_label(ui.visuals().error_fg_color, error);
                                    if menu_card(
                                        ui,
                                        "바탕화면 연결 다시 시도",
                                        "달력을 바탕화면에 다시 연결합니다",
                                        &palette,
                                        false,
                                    )
                                    .clicked()
                                    {
                                        action = Some(MenuAction::RetryDesktop);
                                    }
                                }
                                for error in [
                                    self.error.as_deref(),
                                    self.settings_error.as_deref(),
                                    self.desktop.opacity_error(),
                                ]
                                .into_iter()
                                .flatten()
                                {
                                    ui.colored_label(ui.visuals().error_fg_color, error);
                                }
                            });
                        ui.separator();
                        ui.label(
                            RichText::new("이동  Alt + 드래그  ·  크기  오른쪽 아래")
                                .small()
                                .color(palette.muted),
                        );
                    });
            });
        if close
            || ctx.input(|input| input.key_pressed(egui::Key::Escape))
            || (!opened_now && response.response.clicked_elsewhere())
        {
            ctx.memory_mut(|memory| memory.close_popup());
        }
        if let Some(action) = action {
            if !matches!(
                action,
                MenuAction::PreviousMonth | MenuAction::NextMonth | MenuAction::Today
            ) {
                ui.memory_mut(|memory| memory.close_popup());
            }
            match action {
                MenuAction::PreviousMonth => self.change_month(self.month - Months::new(1)),
                MenuAction::NextMonth => self.change_month(self.month + Months::new(1)),
                MenuAction::Today => self.change_month(Local::now().date_naive()),
                MenuAction::Connect => self.begin_connection(),
                MenuAction::Refresh => {
                    self.send(Action::Load);
                }
                MenuAction::Settings => self.settings_open = true,
                MenuAction::RetryDesktop => self.desktop.retry(),
                MenuAction::Close => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    fn begin_connection(&mut self) {
        if self.connection.active {
            self.connection.open = true;
            return;
        }
        self.connection.begin(self.job.is_some());
        if !self.connection.queued {
            self.send_connection();
        }
    }

    fn send_connection(&mut self) {
        self.connection.queued = false;
        let cancelled = self.connection.cancelled.clone();
        if !self.send(Action::Connect { cancelled }) {
            self.connection.finish(Some(
                self.error
                    .clone()
                    .unwrap_or_else(|| "Google 연결 작업을 시작하지 못했습니다".into()),
            ));
        }
    }

    fn range(&self) -> Range {
        let offset = self.month.weekday().num_days_from_sunday();
        let start = self.month - Days::new(offset.into());
        (start, start + Days::new(42))
    }

    fn send(&mut self, action: Action) -> bool {
        if self.job.is_some() {
            return false;
        }
        let (reply, result) = mpsc::channel();
        let connecting = matches!(&action, Action::Connect { .. });
        if self.worker.send((action, self.range(), reply)).is_err() {
            self.error = Some("일정 작업을 시작할 수 없습니다".into());
            return false;
        }
        self.error = None;
        self.job = Some(result);
        self.job_is_connect = connecting;
        true
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job {
            match job.try_recv() {
                Ok(result) => {
                    if self.job_is_connect {
                        self.connection
                            .finish(result.as_ref().err().map(|error| format!("{error:#}")));
                        if let Ok(output) = &result {
                            if let Some(error) = &output.refresh_error {
                                self.connection.warning(error.clone());
                            }
                        }
                        self.job_is_connect = false;
                    }
                    self.job = None;
                    self.refreshed = Instant::now();
                    match result {
                        Ok(output) => {
                            if self.editor.as_ref().is_some_and(|editor| editor.pending) {
                                self.editor = None;
                            }
                            if let Some(events) = output.events {
                                self.events = events;
                                self.loaded = Some(self.range());
                            }
                            self.error = output.refresh_error;
                        }
                        Err(error) => {
                            let message = format!("{error:#}");
                            if let Some(editor) =
                                self.editor.as_mut().filter(|editor| editor.pending)
                            {
                                editor.pending = false;
                                editor.error = Some(message.clone());
                            }
                            self.error = Some(message);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.job_is_connect {
                        self.connection
                            .finish(Some("Google 연결 작업이 중단됐습니다".into()));
                        self.job_is_connect = false;
                    }
                    self.job = None;
                    self.error = Some("일정 작업이 중단됐습니다".into());
                    if let Some(editor) = self.editor.as_mut().filter(|editor| editor.pending) {
                        editor.pending = false;
                        editor.error = self.error.clone();
                    }
                }
            }
        } else if self.loaded.is_some() && self.error.is_none() {
            let interval = Duration::from_secs(300);
            let remaining = interval.saturating_sub(self.refreshed.elapsed());
            if remaining.is_zero() {
                self.send(Action::Load);
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(remaining);
            }
        }
        if self.connection.queued {
            if self
                .connection
                .cancelled
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                self.connection.finish(None);
            } else if self.job.is_none() {
                self.send_connection();
            }
        }
    }

    fn change_month(&mut self, date: NaiveDate) {
        let month = date.with_day(1).unwrap();
        if month != self.month {
            self.month = month;
            self.send(Action::Load);
        }
    }

    fn show_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        let mut open = true;
        let mut action = None;
        let palette = self.appearance.theme.palette();
        let busy = self.job.is_some();
        let frame = egui::Frame::window(&ctx.style())
            .inner_margin(24)
            .corner_radius(14);
        egui::Window::new("일정 편집")
            .id(egui::Id::new("event_editor"))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
                ui.spacing_mut().button_padding = egui::vec2(14.0, 9.0);
                ui.horizontal(|ui| {
                    ui.heading(if editor.id.is_some() {
                        "일정 수정"
                    } else {
                        "새 일정"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!editor.pending, egui::Button::new("닫기"))
                            .clicked()
                        {
                            open = false;
                        }
                    });
                });
                ui.label(
                    RichText::new("Google Calendar · 기본 캘린더")
                        .small()
                        .color(palette.muted),
                );
                egui::ScrollArea::vertical()
                    .id_salt("event_form_scroll")
                    .max_height((ctx.screen_rect().height() - 260.0).max(120.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if editor.pending {
                            ui.disable();
                        }
                        ui.add_space(2.0);
                        ui.label(RichText::new("일정 제목").strong());
                        let title_field = ui.add(
                            egui::TextEdit::singleline(&mut editor.draft.title)
                                .hint_text("어떤 일정인가요?")
                                .font(egui::TextStyle::Heading)
                                .margin(egui::vec2(12.0, 10.0))
                                .desired_width(f32::INFINITY),
                        );
                        if editor.focus_title {
                            title_field.request_focus();
                            editor.focus_title = false;
                        }
                        ui.checkbox(&mut editor.draft.all_day, "종일 일정");
                        egui::Frame::new()
                            .fill(palette.cell)
                            .corner_radius(10)
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.columns(2, |columns| {
                                    for (index, column) in columns.iter_mut().enumerate() {
                                        column.label(
                                            RichText::new(if index == 0 {
                                                "시작"
                                            } else {
                                                "종료"
                                            })
                                            .strong(),
                                        );
                                        let (date, time) = if index == 0 {
                                            (
                                                &mut editor.draft.start_date,
                                                &mut editor.draft.start_time,
                                            )
                                        } else {
                                            (&mut editor.draft.end_date, &mut editor.draft.end_time)
                                        };
                                        column.push_id(index, |ui| {
                                            editor.dates[index].show(ui, date);
                                            ui.add_enabled_ui(!editor.draft.all_day, |ui| {
                                                editor.times[index].show(ui, time);
                                            });
                                        });
                                    }
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(if editor.draft.all_day {
                                        "종료 날짜를 포함한 기간 동안 표시됩니다."
                                    } else {
                                        "24시간제 · 이 PC의 현지 시간 기준"
                                    })
                                    .small()
                                    .color(palette.muted),
                                );
                            });
                        ui.label(RichText::new("장소 / 온라인 회의 링크").strong());
                        ui.add(
                            egui::TextEdit::singleline(&mut editor.draft.location)
                                .hint_text("회의실, 주소 또는 회의 링크")
                                .margin(egui::vec2(12.0, 9.0))
                                .desired_width(f32::INFINITY),
                        );
                        ui.label(RichText::new("설명 / 메모").strong());
                        ui.add(
                            egui::TextEdit::multiline(&mut editor.draft.description)
                                .hint_text("준비물, 안건, 참고 링크 등을 적어 주세요")
                                .desired_rows(3)
                                .margin(egui::vec2(12.0, 10.0))
                                .desired_width(f32::INFINITY),
                        );
                        egui::CollapsingHeader::new("색상 · 공개 범위 · 알림")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.columns(2, |columns| {
                                    columns[0].label("일정 색상");
                                    let selected = EVENT_COLORS
                                        .iter()
                                        .find(|c| c.0 == editor.draft.color_id)
                                        .map(|c| c.1)
                                        .unwrap_or("색상 선택");
                                    egui::ComboBox::from_id_salt("event_color")
                                        .selected_text(selected)
                                        .width(160.0)
                                        .show_ui(&mut columns[0], |ui| {
                                            for (id, name, color) in EVENT_COLORS {
                                                let color = egui::Color32::from_rgb(
                                                    (color >> 16) as u8,
                                                    (color >> 8) as u8,
                                                    color as u8,
                                                );
                                                ui.horizontal(|ui| {
                                                    ui.colored_label(color, "●");
                                                    ui.selectable_value(
                                                        &mut editor.draft.color_id,
                                                        id.to_owned(),
                                                        name,
                                                    );
                                                });
                                            }
                                        });
                                    columns[1].label("일정 상태");
                                    egui::ComboBox::from_id_salt("event_availability")
                                        .selected_text(if editor.draft.busy {
                                            "바쁨"
                                        } else {
                                            "한가함"
                                        })
                                        .width(160.0)
                                        .show_ui(&mut columns[1], |ui| {
                                            ui.selectable_value(
                                                &mut editor.draft.busy,
                                                true,
                                                "바쁨",
                                            );
                                            ui.selectable_value(
                                                &mut editor.draft.busy,
                                                false,
                                                "한가함",
                                            );
                                        });
                                });
                                ui.label("공개 범위");
                                let visibility_name = match editor.draft.visibility.as_str() {
                                    "public" => "공개",
                                    "private" | "confidential" => "비공개",
                                    _ => "캘린더 기본 설정",
                                };
                                egui::ComboBox::from_id_salt("event_visibility")
                                    .selected_text(visibility_name)
                                    .width(200.0)
                                    .show_ui(ui, |ui| {
                                        for (value, label) in [
                                            ("default", "캘린더 기본 설정"),
                                            ("private", "비공개"),
                                            ("public", "공개"),
                                        ] {
                                            ui.selectable_value(
                                                &mut editor.draft.visibility,
                                                value.to_owned(),
                                                label,
                                            );
                                        }
                                    });
                                ui.separator();
                                ui.label(RichText::new("알림").strong());
                                ui.checkbox(
                                    &mut editor.draft.default_reminders,
                                    "캘린더의 기본 알림 사용",
                                );
                                if !editor.draft.default_reminders {
                                    let mut remove = None;
                                    for (index, reminder) in
                                        editor.draft.reminders.iter_mut().enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            egui::ComboBox::from_id_salt((
                                                "reminder_method",
                                                index,
                                            ))
                                            .selected_text(if reminder.method == "email" {
                                                "이메일"
                                            } else {
                                                "팝업"
                                            })
                                            .width(90.0)
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    ui.selectable_value(
                                                        &mut reminder.method,
                                                        "popup".into(),
                                                        "팝업",
                                                    );
                                                    ui.selectable_value(
                                                        &mut reminder.method,
                                                        "email".into(),
                                                        "이메일",
                                                    );
                                                },
                                            );
                                            ui.add(
                                                egui::DragValue::new(&mut reminder.minutes)
                                                    .range(0..=40320)
                                                    .suffix(" 분 전"),
                                            );
                                            if ui.small_button("삭제").clicked() {
                                                remove = Some(index);
                                            }
                                        });
                                    }
                                    if let Some(index) = remove {
                                        editor.draft.reminders.remove(index);
                                    }
                                    if editor.draft.reminders.is_empty() {
                                        ui.weak("설정된 알림이 없습니다.");
                                    }
                                    if ui
                                        .add_enabled(
                                            editor.draft.reminders.len() < 5,
                                            egui::Button::new("+ 알림 추가"),
                                        )
                                        .clicked()
                                    {
                                        editor.draft.reminders.push(Reminder {
                                            method: "popup".into(),
                                            minutes: 10,
                                        });
                                    }
                                    ui.small("최대 5개 · 1일 전 = 1440분 · 최대 4주 전");
                                }
                            });
                    });
                let payload = editor.draft.payload(editor.id.is_some());
                if let Err(error) = &payload {
                    if !editor.draft.title.trim().is_empty() {
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                }
                if let Some(error) = &editor.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if editor.pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Google Calendar에 반영 중…");
                    });
                }
                ui.separator();
                if editor.confirm_delete {
                    ui.label("Google Calendar에서도 이 일정이 삭제됩니다.");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("삭제 확인"))
                            .clicked()
                        {
                            if let Some(id) = &editor.id {
                                action = Some(Action::Delete(id.clone()));
                            }
                        }
                        if ui
                            .add_enabled(!editor.pending, egui::Button::new("돌아가기"))
                            .clicked()
                        {
                            editor.confirm_delete = false;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if editor.id.is_some()
                            && ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(
                                        RichText::new("일정 삭제")
                                            .color(ui.visuals().error_fg_color),
                                    ),
                                )
                                .clicked()
                        {
                            editor.confirm_delete = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(
                                    payload.is_ok() && !busy,
                                    egui::Button::new("저장")
                                        .min_size(egui::vec2(100.0, 38.0))
                                        .fill(palette.today),
                                )
                                .clicked()
                            {
                                if let Ok(payload) = &payload {
                                    action = Some(Action::Save {
                                        id: editor.id.clone(),
                                        payload: payload.clone(),
                                    });
                                }
                            }
                            if ui
                                .add_enabled(
                                    !editor.pending,
                                    egui::Button::new("취소").min_size(egui::vec2(80.0, 38.0)),
                                )
                                .clicked()
                            {
                                open = false;
                            }
                        });
                    });
                }
            });
        if let Some(action) = action {
            let sent = self.send(action);
            if let Some(editor) = &mut self.editor {
                editor.pending = sent;
                editor.error = if sent { None } else { self.error.clone() };
            }
        }
        if !open {
            self.editor = None;
        }
    }
}

impl eframe::App for Calendar {
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Some(geometry) = self.desktop.geometry() {
            self.appearance.geometry = Some(geometry);
        }
        if let Err(error) = self.appearance.save() {
            eprintln!("설정 저장 실패: {error}");
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.desktop
            .attach(frame, self.appearance.geometry, ctx.pixels_per_point());
        self.desktop.keep_visible();
        self.apply_appearance(ctx);
        let palette = self.appearance.theme.palette();
        let mut resizing = false;
        let moving = ctx.input(|i| {
            i.modifiers.alt && i.pointer.primary_down() && i.pointer.interact_pos().is_some()
        });
        while let Ok(progress) = self.login_progress.try_recv() {
            self.connection.progress(progress);
        }
        self.connection.poll_browser(ctx);
        self.poll(ctx);
        let today = Local::now().date_naive();
        let menu_position = ctx.input(|i| {
            (i.modifiers.alt && i.pointer.button_clicked(egui::PointerButton::Secondary))
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        if let Some(position) = menu_position {
            self.menu_position = position;
            ctx.memory_mut(|memory| memory.open_popup(egui::Id::new("widget_context_menu")));
        }
        egui::TopBottomPanel::top("month_heading").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                ui.heading(self.month.format("%Y년 %m월").to_string());
            });
            ui.add_space(8.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            let (start, end) = self.range();
            let gap = 4.0;
            let width = (ui.available_width() - 6.0 * gap) / 7.0;
            let height = ((ui.available_height() - 30.0 - 6.0 * gap) / 6.0).max(52.0);
            ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
            ui.horizontal(|ui| {
                for label in ["일", "월", "화", "수", "목", "금", "토"] {
                    ui.add_sized([width, 24.0], egui::Label::new(RichText::new(label).strong()));
                }
            });
            for week in 0..6 {
                ui.horizontal(|ui| {
                    for weekday in 0..7 {
                        let date = start + Days::new(week * 7 + weekday);
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                        let color = if date == today {
                            palette.today
                        } else if date.month() == self.month.month() {
                            palette.cell
                        } else {
                            palette.outside
                        };
                        ui.painter().rect_filled(rect, 6.0, color);
                        let mut cell = ui.new_child(egui::UiBuilder::new()
                            .id_salt(date).max_rect(rect.shrink(5.0)).layout(egui::Layout::top_down(egui::Align::Min)));
                        cell.set_clip_rect(rect);
                        let day = cell.add(egui::Label::new(RichText::new(date.day().to_string()).strong().color(if date.month() == self.month.month() { palette.text } else { palette.muted }))
                            .sense(egui::Sense::click()));
                        let mut new_event = day.double_clicked();
                        if self.loaded == Some((start, end)) {
                            egui::ScrollArea::vertical().id_salt(date)
                                .max_height(cell.available_height()).auto_shrink([false, true])
                                .show(&mut cell, |ui| {
                                    for event in self.events.iter().filter(|event| event.occurs_on(date)) {
                                        let title = match event.start.date_time {
                                            Some(time) if time.with_timezone(&Local).date_naive() == date =>
                                                format!("{}  {}", time.with_timezone(&Local).format("%H:%M"), event.title()),
                                            _ => event.title().to_owned(),
                                        };
                                        let tooltip = format!("{}\n{}\n{}", title, event.location.as_deref().unwrap_or(""), event.description.as_deref().unwrap_or(""));
                                        let row = ui.horizontal(|ui| {
                                            if let Some((_, _, color)) = EVENT_COLORS.iter().find(|c| Some(c.0) == event.color_id.as_deref()) {
                                                let color = *color;
                                                ui.colored_label(egui::Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8), "●");
                                            }
                                            ui.add(egui::Label::new(RichText::new(&title).color(palette.text))
                                                .truncate().sense(egui::Sense::click())).on_hover_text(tooltip.trim())
                                        }).inner;
                                        if row.double_clicked() && self.job.is_none() {
                                            if event.recurring_event_id.is_some() {
                                                self.error = Some("반복 일정은 Google Calendar에서 수정할 수 있습니다".into());
                                            } else {
                                                self.editor = Some(Editor::from_event(event, date));
                                            }
                                        }
                                    }
                                });
                        }
                        let (_, blank) = cell.allocate_exact_size(cell.available_size(), egui::Sense::click());
                        new_event |= blank.double_clicked();
                        if new_event && self.job.is_none() {
                            self.editor = Some(Editor::new(date));
                        }
                    }
                });
            }
            let corner = ui.max_rect().right_bottom();
            let rect = egui::Rect::from_min_max(corner - egui::vec2(18.0, 18.0), corner);
            let grip = ui.interact(rect, ui.id().with("resize_grip"), egui::Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeNwSe)
                .on_hover_text("드래그하여 크기 조절");
            for inset in [4.0, 8.0, 12.0] {
                ui.painter().line_segment([corner - egui::vec2(inset, 1.0), corner - egui::vec2(1.0, inset)],
                    egui::Stroke::new(1.5_f32, palette.muted));
            }
            resizing = grip.dragged();
            self.show_menu(ui, menu_position.is_some());
        });
        self.show_editor(ctx);
        self.show_settings(ctx);
        if self.connection.show(ctx) {
            self.begin_connection();
        }
        self.apply_appearance(ctx);
        self.desktop.resize(resizing);
        self.desktop.drag(moving && !resizing);
        if let Some(geometry) = self.desktop.geometry() {
            if self.appearance.geometry != Some(geometry) {
                self.appearance.geometry = Some(geometry);
                self.settings_dirty = true;
            }
        }
        if self.settings_dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.settings_error = self
                .appearance
                .save()
                .err()
                .map(|error| format!("설정 저장 실패: {error}"));
            self.settings_dirty = false;
        }
        if moving || resizing {
            ctx.request_repaint();
        }
    }
}
