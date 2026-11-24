//! Calendar and place (Map menu), as OMSI 2's editor keeps them beside the map: where on
//! earth it lies and its time zone (`timezone.txt`: the sun and the clocks), summer time,
//! school and public holidays (`Holidays.txt`, kept with the timetable: its day types come
//! from them; the Lines window shows the same) and the number
//! plates of the AI's cars (`registrations.txt`).

use egui::{RichText, Vec2};
use omsi_map::calendar::{Dst, Holiday, HolidayRange};

use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::traffic::File;
use crate::ui::prop_row;
use crate::theme::pal;

#[derive(Default, PartialEq, Clone, Copy)]
enum Tab {
    #[default]
    Place,
    Holidays,
    Plates,
}

#[derive(Default)]
pub struct CalendarWindow {
    pub open: bool,
    tab: Tab,
    /// The year whose holidays are shown.
    year: i32,
    /// Summer time made for these years (EU rules).
    dst_years: (i32, i32),
    /// The plates as one text, while it is edited.
    plates: Option<String>,
    /// The pattern new plates are made of, and how many.
    pattern: String,
    count: usize,
}

/// Days from 1970-01-01 of a date (proleptic Gregorian).
fn days(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The last Sunday of a month, as its day.
fn last_sunday(y: i32, m: i32) -> i32 {
    let len = [31, if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(m - 1) as usize];
    // (1970-01-01 was a Thursday: 0 Thursday … 3 Sunday)
    let wd = (days(y, m, len) + 4).rem_euclid(7); // 0 Sunday
    len - wd as i32
}

/// Summer time as the EU keeps it: the last Sunday of March to the last of October (of
/// September until 1995), clocks forward at 2 and back at 3, by an hour.
pub fn eu_summer_time(from: i32, to: i32) -> Vec<Dst> {
    (from..=to)
        .map(|y| {
            let end = if y < 1996 { y * 10_000 + 900 + last_sunday(y, 9) } else { y * 10_000 + 1000 + last_sunday(y, 10) };
            Dst { start: y * 10_000 + 300 + last_sunday(y, 3), end, params: [2.0, 3.0, 1.0] }
        })
        .collect()
}

/// Plates of a pattern: `X` a letter, `9` a digit, the rest as it is ("B-XX 9999").
pub fn plates(pattern: &str, n: usize, seed: u64) -> Vec<String> {
    let mut x = seed | 1;
    let mut next = move |m: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % m
    };
    (0..n)
        .map(|_| {
            pattern
                .chars()
                .map(|c| match c {
                    'X' => (b'A' + next(26) as u8) as char,
                    '9' => (b'0' + next(10) as u8) as char,
                    c => c,
                })
                .collect()
        })
        .collect()
}

/// A date (yyyymmdd) of a year as day and month fields.
fn date_field(ui: &mut egui::Ui, date: &mut i32) -> bool {
    let (y, mut m, mut d) = (*date / 10_000, *date / 100 % 100, *date % 100);
    let a = ui.add(egui::DragValue::new(&mut d).range(1..=31).speed(0.2)).changed();
    ui.label(".");
    let b = ui.add(egui::DragValue::new(&mut m).range(1..=12).speed(0.1)).changed();
    if a || b {
        *date = y * 10_000 + m * 100 + d;
    }
    a || b
}

impl CalendarWindow {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor) {
        if !self.open {
            self.plates = None;
            return;
        }
        let Some(doc) = ed.doc.as_mut() else { return };
        if self.year == 0 {
            if std::env::var("OPENMAPPER_CALENDAR_TAB").as_deref() == Ok("holidays") {
                self.tab = Tab::Holidays;
            }
            let y = doc.global.years.0;
            self.year = if y > 1900 { y } else { 2000 };
            self.dst_years = if doc.global.years.0 > 1900 { (doc.global.years.0.max(1980), doc.global.years.1.max(doc.global.years.0) + 30) } else { (1996, 2040) };
            self.pattern = "B-XX 9999".into();
            self.count = 100;
        }
        let before = doc.traffic.clone();
        // (the holidays are the timetable's: its day types come from them)
        let tt_before = doc.timetable.calendar.clone();
        let tt_whole = doc.timetable.clone();
        let (t, cal) = (&mut doc.traffic, &mut doc.timetable.calendar);
        let mut touched: Vec<File> = Vec::new();
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Calendar, tr("Calendar and place", "Календарь и место"))).id(egui::Id::new("calendar window")).open(&mut open).default_pos([340.0, 100.0]).default_size([460.0, 520.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::Place, tr("Place and time", "Место и время")), (Tab::Holidays, tr("Holidays", "Праздники и каникулы")), (Tab::Plates, tr("Number plates", "Номера машин"))] {
                    if ui.selectable_label(self.tab == tab, label).clicked() {
                        self.tab = tab;
                    }
                }
            });
            ui.separator();
            match self.tab {
                Tab::Place => {
                    let z = &mut t.timezone;
                    while z.location.len() < 2 {
                        z.location.push("0".into());
                    }
                    let mut lat = omsi_cfg::parse_f64(&z.location[0]);
                    let mut lon = omsi_cfg::parse_f64(&z.location[1]);
                    egui::Grid::new("place").num_columns(2).spacing(Vec2::new(8.0, 6.0)).show(ui, |ui| {
                        prop_row(ui, tr("Latitude", "Широта"), |ui| {
                            if ui.add(egui::DragValue::new(&mut lat).range(-90.0..=90.0).speed(0.01).max_decimals(6).suffix("°")).changed() {
                                z.location[0] = format!("{lat:.6}");
                                touched.push(File::TimeZone);
                            }
                        });
                        prop_row(ui, tr("Longitude", "Долгота"), |ui| {
                            if ui.add(egui::DragValue::new(&mut lon).range(-180.0..=180.0).speed(0.01).max_decimals(6).suffix("°")).changed() {
                                z.location[1] = format!("{lon:.6}");
                                touched.push(File::TimeZone);
                            }
                        });
                        prop_row(ui, tr("Time zone", "Часовой пояс"), |ui| {
                            if ui.add(egui::DragValue::new(&mut z.offset_hours).range(-12.0..=14.0).speed(0.05).max_decimals(2).prefix("UTC ")).changed() {
                                touched.push(File::TimeZone);
                            }
                        });
                    });
                    ui.label(RichText::new(tr("Where the sun stands and when it rises and sets come from here.", "Отсюда берутся положение солнца, восход и закат.")).small().color(pal().text_dim));
                    ui.add_space(8.0);
                    ui.label(RichText::new(tr("Summer time", "Летнее время")).strong());
                    let (a, b) = (z.dst.first().map(|d| d.start / 10_000), z.dst.last().map(|d| d.start / 10_000));
                    ui.label(match (a, b) {
                        (Some(a), Some(b)) => format!("{} {} ({a}–{b})", z.dst.len(), tr("years", "лет")),
                        _ => tr("none: the clocks are never put forward", "нет: часы не переводятся").to_string(),
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut self.dst_years.0).range(1900..=2100));
                        ui.label("–");
                        ui.add(egui::DragValue::new(&mut self.dst_years.1).range(1900..=2100));
                        if ui.button(icons::labelled(Icon::AutoFix, tr("EU rules for these years", "Правила ЕС на эти годы"))).on_hover_text(tr("Last Sunday of March to last Sunday of October, replacing what is set for these years", "С последнего воскресенья марта по последнее воскресенье октября, вместо заданного на эти годы")).clicked() {
                            let (from, to) = (self.dst_years.0.min(self.dst_years.1), self.dst_years.0.max(self.dst_years.1));
                            z.dst.retain(|d| d.start / 10_000 < from || d.start / 10_000 > to);
                            z.dst.extend(eu_summer_time(from, to));
                            z.dst.sort_by_key(|d| d.start);
                            touched.push(File::TimeZone);
                        }
                        if !z.dst.is_empty() && ui.button(tr("None", "Нет")).clicked() {
                            z.dst.clear();
                            touched.push(File::TimeZone);
                        }
                    });
                }
                Tab::Holidays => {
                    let c = &mut *cal;
                    ui.horizontal(|ui| {
                        ui.label(tr("Year", "Год"));
                        ui.add(egui::DragValue::new(&mut self.year).range(1900..=2100));
                        let n = c.holidays.iter().filter(|h| h.date / 10_000 == self.year).count() + c.ranges.iter().filter(|r| r.start / 10_000 == self.year).count();
                        ui.label(RichText::new(format!("{n} {}", tr("entries", "записей"))).color(pal().text_dim));
                    });
                    ui.label(RichText::new(tr("Public holidays run Sunday's timetable; school holidays the holiday one.", "В праздники ходит воскресное расписание, в каникулы — каникулярное.")).small().color(pal().text_dim));
                    let y = self.year;
                    egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(360.0).show(ui, |ui| {
                        ui.label(RichText::new(tr("Public holidays", "Праздники")).strong());
                        let mut remove = None;
                        let mut idx: Vec<usize> = (0..c.holidays.len()).filter(|&i| c.holidays[i].date / 10_000 == y).collect();
                        idx.sort_by_key(|&i| c.holidays[i].date);
                        for i in idx {
                            let h = &mut c.holidays[i];
                            ui.horizontal(|ui| {
                                date_field(ui, &mut h.date);
                                ui.add(egui::TextEdit::singleline(&mut h.name).desired_width(200.0));
                                if icons::small(ui, Icon::Delete, tr("Delete", "Удалить")).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            c.holidays.remove(i);
                        }
                        if ui.button(icons::labelled(Icon::Add, tr("Add a holiday", "Добавить праздник"))).clicked() {
                            c.holidays.push(Holiday { date: y * 10_000 + 101, name: tr("Holiday", "Праздник").into() });
                        }
                        ui.add_space(8.0);
                        ui.label(RichText::new(tr("School holidays", "Школьные каникулы")).strong());
                        let mut remove = None;
                        let mut idx: Vec<usize> = (0..c.ranges.len()).filter(|&i| c.ranges[i].start / 10_000 == y).collect();
                        idx.sort_by_key(|&i| c.ranges[i].start);
                        for i in idx {
                            let r = &mut c.ranges[i];
                            ui.horizontal(|ui| {
                                date_field(ui, &mut r.start);
                                ui.label("–");
                                // (school holidays over New Year end in the next year)
                                date_field(ui, &mut r.end);
                                ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(120.0));
                                if icons::small(ui, Icon::Delete, tr("Delete", "Удалить")).clicked() {
                                    remove = Some(i);
                                }
                            });
                            if r.end < r.start {
                                ui.label(RichText::new(tr("Ends before it starts", "Кончается раньше начала")).small().color(pal().warn));
                            }
                        }
                        if let Some(i) = remove {
                            c.ranges.remove(i);
                        }
                        if ui.button(icons::labelled(Icon::Add, tr("Add school holidays", "Добавить каникулы"))).clicked() {
                            c.ranges.push(HolidayRange { start: y * 10_000 + 701, end: y * 10_000 + 831, name: tr("Summer", "Лето").into() });
                        }
                    });
                }
                Tab::Plates => {
                    ui.label(RichText::new(tr("The AI's cars carry these, one a line.", "Эти номера носят машины ИИ, по одному в строке.")).small().color(pal().text_dim));
                    let text = self.plates.get_or_insert_with(|| t.registrations.join("\n"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.pattern).desired_width(110.0)).on_hover_text(tr("X: a letter, 9: a digit", "X: буква, 9: цифра"));
                        ui.add(egui::DragValue::new(&mut self.count).range(1..=5000).prefix("× "));
                        if ui.button(icons::labelled(Icon::Add, tr("Make", "Создать"))).on_hover_text(tr("Replaces the list with plates of the pattern", "Заменяет список номерами по образцу")).clicked() {
                            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7);
                            *text = plates(&self.pattern, self.count, seed).join("\n");
                        }
                    });
                    let n = text.lines().filter(|l| !l.trim().is_empty()).count();
                    ui.label(RichText::new(format!("{n} {}", tr("plates", "номеров"))).color(pal().text_dim));
                    egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                        ui.add(egui::TextEdit::multiline(text).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
                    });
                    let list: Vec<String> = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                    if list != t.registrations {
                        t.registrations = list;
                        touched.push(File::Registrations);
                    }
                }
            }
        });
        self.open = open;
        for f in touched {
            doc.traffic.touch(f);
        }
        let calendar = doc.timetable.calendar != tt_before;
        if calendar {
            doc.timetable.touch_calendar();
        }
        let traffic = doc.traffic != before;
        if traffic || calendar {
            ed.history.push_snapshot(tr("Calendar and place", "Календарь и место"), crate::history::Snapshot { traffic: traffic.then_some(before), timetable: calendar.then_some(tt_whole), ..Default::default() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eu_summer_time_matches_the_maps() {
        // (Grundorf's timezone.txt)
        let d = eu_summer_time(2018, 2019);
        assert_eq!((d[0].start, d[0].end), (20180325, 20181028));
        assert_eq!((d[1].start, d[1].end), (20190331, 20191027));
        assert_eq!(eu_summer_time(1996, 1996)[0].end, 19961027);
        assert_eq!((eu_summer_time(1985, 1985)[0].start, eu_summer_time(1985, 1985)[0].end), (19850331, 19850929));
    }

    #[test]
    fn plates_follow_their_pattern() {
        for p in plates("B-XX 9999", 50, 42) {
            let c: Vec<char> = p.chars().collect();
            assert_eq!(c.len(), 9);
            assert!(c[2].is_ascii_uppercase() && c[3].is_ascii_uppercase() && c[5..].iter().all(|d| d.is_ascii_digit()) && p.starts_with("B-"), "{p}");
        }
    }
}
