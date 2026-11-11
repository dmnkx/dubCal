use chrono::{Datelike, Days, Local, Months, NaiveDate, NaiveTime, Timelike};
use eframe::egui::{self, RichText};

pub struct DatePicker {
    month: NaiveDate,
}
impl Default for DatePicker {
    fn default() -> Self {
        Self {
            month: Local::now().date_naive().with_day(1).unwrap(),
        }
    }
}

impl DatePicker {
    pub fn show(&mut self, ui: &mut egui::Ui, value: &mut String) -> egui::Response {
        let input_id = ui.make_persistent_id("date_input");
        let popup_id = ui.make_persistent_id("date_picker");
        let button = field(ui, value, "YYYY-MM-DD", input_id);
        if button.clicked() {
            self.month = NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .unwrap_or_else(|| Local::now().date_naive())
                .with_day(1)
                .unwrap();
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        let selected = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok();
        egui::popup::popup_below_widget(
            ui,
            popup_id,
            &button,
            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(260.0);
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 6.0);
                ui.spacing_mut().button_padding = egui::vec2(6.0, 5.0);
                egui::Frame::new().inner_margin(8).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                self.month.checked_sub_months(Months::new(1)).is_some(),
                                egui::Button::new("◀"),
                            )
                            .clicked()
                        {
                            self.month = self.month.checked_sub_months(Months::new(1)).unwrap();
                        }
                        ui.add_sized(
                            [160.0, 30.0],
                            egui::Label::new(
                                RichText::new(self.month.format("%Y년 %m월").to_string()).strong(),
                            ),
                        );
                        if ui
                            .add_enabled(
                                self.month.checked_add_months(Months::new(1)).is_some(),
                                egui::Button::new("▶"),
                            )
                            .clicked()
                        {
                            self.month = self.month.checked_add_months(Months::new(1)).unwrap();
                        }
                    });
                    egui::Grid::new(popup_id.with("days"))
                        .spacing([4.0, 4.0])
                        .show(ui, |ui| {
                            for label in ["일", "월", "화", "수", "목", "금", "토"] {
                                ui.add_sized(
                                    [30.0, 24.0],
                                    egui::Label::new(RichText::new(label).small().weak()),
                                );
                            }
                            ui.end_row();
                            for (index, date) in month_grid(self.month).into_iter().enumerate() {
                                if let Some(date) = date {
                                    let text = RichText::new(format!("{}", date.day()));
                                    let text = if date.month() != self.month.month() {
                                        text.weak()
                                    } else {
                                        text
                                    };
                                    let button = egui::Button::new(text)
                                        .selected(selected == Some(date))
                                        .corner_radius(6);
                                    if ui.add_sized([30.0, 28.0], button).clicked() {
                                        *value = date.to_string();
                                        ui.memory_mut(|memory| memory.close_popup());
                                    }
                                } else {
                                    ui.add_space(30.0);
                                }
                                if index % 7 == 6 {
                                    ui.end_row();
                                }
                            }
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("오늘").clicked() {
                            *value = Local::now().date_naive().to_string();
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                        if ui.button("닫기").clicked() {
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                    });
                });
            },
        );
        button
    }
}

#[derive(Default)]
pub struct TimePicker {
    hour: u32,
    minute: u32,
}
impl TimePicker {
    pub fn show(&mut self, ui: &mut egui::Ui, value: &mut String) -> egui::Response {
        let input_id = ui.make_persistent_id("time_input");
        let popup_id = ui.make_persistent_id("time_picker");
        let button = field(ui, value, "HH:MM", input_id);
        let opened = button.clicked();
        if opened {
            let time =
                parse_time(value).unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
            self.hour = time.hour();
            self.minute = time.minute();
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        egui::popup::popup_below_widget(
            ui,
            popup_id,
            &button,
            egui::popup::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(220.0);
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                egui::Frame::new().inner_margin(10).show(ui, |ui| {
                    ui.strong("시간 선택");
                    ui.columns(2, |columns| {
                        for (index, column) in columns.iter_mut().enumerate() {
                            column.label(if index == 0 {
                                "시 (0~23)"
                            } else {
                                "분 (0~59)"
                            });
                            let (current, count) = if index == 0 {
                                (&mut self.hour, 24)
                            } else {
                                (&mut self.minute, 60)
                            };
                            egui::ScrollArea::vertical()
                                .id_salt((popup_id, index))
                                .max_height(160.0)
                                .auto_shrink([false, false])
                                .show(column, |ui| {
                                    for number in 0..count {
                                        let response = ui.add_sized(
                                            [80.0, 28.0],
                                            egui::Button::new(format!("{number:02}"))
                                                .selected(*current == number),
                                        );
                                        if opened && *current == number {
                                            response.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if response.clicked() {
                                            *current = number;
                                        }
                                    }
                                });
                        }
                    });
                    ui.label(format!("선택한 시간  {:02}:{:02}", self.hour, self.minute));
                    ui.horizontal(|ui| {
                        if ui.button("적용").clicked() {
                            *value = format!("{:02}:{:02}", self.hour, self.minute);
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                        if ui.button("닫기").clicked() {
                            ui.memory_mut(|memory| memory.close_popup());
                        }
                    });
                });
            },
        );
        button
    }
}

fn field(ui: &mut egui::Ui, value: &mut String, hint: &str, id: egui::Id) -> egui::Response {
    let width = (ui.available_width() - ui.spacing().item_spacing.x - 44.0).max(80.0);
    ui.horizontal(|ui| {
        ui.add_sized(
            [width, 36.0],
            egui::TextEdit::singleline(value)
                .id(id)
                .hint_text(hint)
                .margin(egui::vec2(8.0, 9.0)),
        );
        ui.add_sized([44.0, 36.0], egui::Button::new("선택").corner_radius(6))
    })
    .inner
}

fn parse_time(value: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S%.f"))
        .ok()
}

fn month_grid(month: NaiveDate) -> [Option<NaiveDate>; 42] {
    let first = month.with_day(1).unwrap();
    let offset = i64::from(first.weekday().num_days_from_sunday());
    std::array::from_fn(|index| {
        let days = index as i64 - offset;
        if days < 0 {
            first.checked_sub_days(Days::new((-days) as u64))
        } else {
            first.checked_add_days(Days::new(days as u64))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn leap_february_grid_has_all_days_and_sunday_first() {
        let month = NaiveDate::from_ymd_opt(2024, 2, 1).unwrap();
        let grid = month_grid(month);
        assert_eq!(
            grid[0].unwrap(),
            NaiveDate::from_ymd_opt(2024, 1, 28).unwrap()
        );
        assert_eq!(
            grid.iter()
                .flatten()
                .filter(|date| date.month() == 2)
                .count(),
            29
        );
        assert_eq!(
            grid[32].unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()
        );
        assert_eq!(
            grid[41].unwrap(),
            NaiveDate::from_ymd_opt(2024, 3, 9).unwrap()
        );
    }
    #[test]
    fn year_boundary_and_existing_second_precision_are_supported() {
        let grid = month_grid(NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert!(grid.iter().flatten().any(|date| date.year() == 2027));
        assert_eq!(parse_time("23:59").unwrap().minute(), 59);
        assert_eq!(parse_time("09:15:34.123").unwrap().second(), 34);
        assert!(parse_time("25:00").is_none());
    }
    #[test]
    fn clicking_a_calendar_day_updates_the_editable_field() {
        let ctx = egui::Context::default();
        let mut picker = DatePicker::default();
        let mut value = "2026-11-11".to_owned();
        let button_rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |events: Vec<egui::Event>| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.set_width(220.0);
                        button_rect.set(picker.show(ui, &mut value).rect);
                    });
                },
            )
        };
        let first = draw(vec![]);
        drop(first);
        let button = button_rect.get().center();
        for pressed in [true, false] {
            let _ = draw(vec![
                egui::Event::PointerMoved(button),
                egui::Event::PointerButton {
                    pos: button,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let output = draw(vec![]);
        fn find(shape: &egui::epaint::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text == "15" => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(find),
                _ => None,
            }
        }
        let day = output
            .shapes
            .iter()
            .find_map(|shape| find(&shape.shape))
            .expect("Calendar day should be visible");
        for pressed in [true, false] {
            let _ = draw(vec![
                egui::Event::PointerMoved(day),
                egui::Event::PointerButton {
                    pos: day,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        drop(draw);
        assert_eq!(value, "2026-11-15");
    }

    #[test]
    fn manual_input_still_edits_the_date_and_time_fields() {
        for date_field in [true, false] {
            let ctx = egui::Context::default();
            let mut date = DatePicker::default();
            let mut time = TimePicker::default();
            let mut value = String::new();
            let mut input_id = egui::Id::NULL;
            let mut draw = |ctx: &egui::Context| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    input_id = ui.make_persistent_id(if date_field {
                        "date_input"
                    } else {
                        "time_input"
                    });
                    if date_field {
                        date.show(ui, &mut value);
                    } else {
                        time.show(ui, &mut value);
                    }
                });
            };
            let _ = ctx.run(egui::RawInput::default(), &mut draw);
            drop(draw);
            ctx.memory_mut(|memory| memory.request_focus(input_id));
            let text = if date_field { "2028-02-29" } else { "23:59" };
            let input = egui::RawInput {
                events: vec![egui::Event::Text(text.into())],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if date_field {
                        date.show(ui, &mut value);
                    } else {
                        time.show(ui, &mut value);
                    }
                });
            });
            assert_eq!(value, text);
        }
    }
}
