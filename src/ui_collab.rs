//! File > Multiplayer: host the map open now or join one, who is in, the address to give
//! out, a chat.

use egui::{Color32, RichText};

use crate::collab::{Collab, TunnelState, MIN_PASSWORD};
use crate::editor::Editor;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::theme::pal;


pub fn window(ctx: &egui::Context, c: &mut Collab, ed: &mut Editor) {
    if !c.open {
        return;
    }
    let mut open = c.open;
    egui::Window::new(icons::labelled(Icon::People, tr("Multiplayer", "Совместная правка"))).id(egui::Id::new("collab window")).open(&mut open).default_pos([380.0, 110.0]).default_width(360.0).resizable(false).show(ctx, |ui| {
        if c.live().is_some() {
            live(ui, c, ed);
        } else if c.joining() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Joining… (a big map takes a while to come)", "Подключение… (большая карта передаётся не сразу)"));
            });
            if ui.button(tr("Cancel", "Отмена")).clicked() {
                c.cancel_join();
            }
        } else {
            start(ui, c, ed);
        }
        if let Some(e) = &c.error {
            ui.colored_label(pal().warn, e);
        }
    });
    c.open = open;
}

fn start(ui: &mut egui::Ui, c: &mut Collab, ed: &mut Editor) {
    if let Some(doc) = ed.doc.as_mut() {
        if ui.button(icons::labelled(Icon::Backup, tr("Recover a conflicting edit…", "Восстановить конфликтующую правку…"))).clicked() {
            if let Some(path) = rfd::FileDialog::new().set_directory(doc.dir.join("openMapper").join("conflicts")).add_filter("openMapper recovery", &["json"]).pick_file() {
                match crate::collab::recovery::restore(&path, doc, &mut ed.history) {
                    Ok(()) => {
                        ed.selected = None;
                        ed.also.clear();
                        ed.hover = None;
                        ed.toast(tr("Recovered your edit; undo is available", "Правка восстановлена; её можно отменить"));
                    }
                    Err(e) => ed.error(e.to_string()),
                }
            }
        }
    }
    ui.label(RichText::new(tr("Edit one map together: one of you hosts it, the others join. Roads, objects, ground and paint are shared as they are made; the timetable, the traffic and the descriptions stay the host's, and the host saves the map. Everybody needs the same objects and splines installed.", "Правка одной карты вместе: один открывает её, остальные подключаются. Дороги, объекты, рельеф и покраска передаются сразу; расписание, трафик и описания остаются у хоста, и сохраняет карту хост. У всех должны быть установлены те же объекты и сплайны.")).small().color(pal().text_dim));
    ui.add_space(4.0);
    egui::Grid::new("collab who").num_columns(2).show(ui, |ui| {
        ui.label(tr("Your name", "Ваше имя"));
        ui.add(egui::TextEdit::singleline(&mut c.name).char_limit(32).desired_width(200.0));
        ui.end_row();
        ui.label(tr("Password", "Пароль"));
        ui.add(egui::TextEdit::singleline(&mut c.password).password(true).desired_width(200.0));
        ui.end_row();
    });
    ui.separator();
    ui.heading(tr("Host this map", "Открыть доступ к этой карте"));
    ui.checkbox(&mut c.lan, tr("Others on this network can join", "Могут подключиться из этой сети"));
    ui.checkbox(&mut c.internet, tr("Others anywhere can join (a free Cloudflare tunnel, nothing to set up on the router)", "Могут подключиться откуда угодно (бесплатный туннель Cloudflare, без настройки роутера)"));
    ui.horizontal(|ui| {
        ui.label(tr("Port", "Порт"));
        ui.add(egui::DragValue::new(&mut c.port).range(1024..=65535));
    });
    // (no password: only when chosen here and confirmed, never on the internet; a password
    // typed takes it back)
    if !c.password.is_empty() || c.internet {
        c.no_password = false;
        c.asking_no_password = false;
    }
    if c.internet && c.password.chars().count() < MIN_PASSWORD {
        ui.colored_label(pal().warn, format!("{} {MIN_PASSWORD}", tr("Over the internet the password must have at least this many letters:", "Для интернета пароль должен быть не короче, букв:")));
    } else if c.password.is_empty() && c.no_password {
        ui.colored_label(pal().warn, tr("No password: anybody who reaches this computer's port can join and change the whole map.", "Без пароля: любой, кто достучится до порта этого компьютера, может подключиться и изменить всю карту."));
        if ui.small_button(icons::labelled(Icon::Lock, tr("Ask for a password after all", "Всё-таки спрашивать пароль"))).clicked() {
            c.no_password = false;
        }
    } else if c.password.is_empty() && c.asking_no_password {
        ui.colored_label(pal().warn, tr("Host without a password? Anybody on this network who finds the port can join and change the whole map.", "Открыть без пароля? Любой в этой сети, кто найдёт порт, сможет подключиться и изменить всю карту."));
        ui.horizontal(|ui| {
            if ui.button(icons::labelled(Icon::Unlock, tr("Yes, without a password", "Да, без пароля"))).clicked() {
                c.no_password = true;
                c.asking_no_password = false;
            }
            if ui.button(tr("No", "Нет")).clicked() {
                c.asking_no_password = false;
            }
        });
    } else if c.password.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tr("Type a password the others will need.", "Введите пароль, который понадобится остальным.")).small().color(pal().text_dim));
            if ui.small_button(icons::labelled(Icon::Unlock, tr("Host without a password…", "Открыть без пароля…"))).clicked() {
                c.asking_no_password = true;
            }
        });
    }
    let password_ok = if c.internet { c.password.chars().count() >= MIN_PASSWORD } else { !c.password.is_empty() || c.no_password };
    let can_host = ed.doc.is_some() && !c.name.trim().is_empty() && (c.lan || c.internet) && password_ok;
    if ui.add_enabled(can_host, egui::Button::new(icons::labelled(Icon::Public, tr("Host", "Стать хостом")))).on_disabled_hover_text(tr("Open a map, type a name and a password first", "Сначала откройте карту, введите имя и пароль")).clicked() {
        c.host(ed);
    }
    ui.separator();
    ui.heading(tr("Join a map", "Подключиться к карте"));
    ui.add(egui::TextEdit::singleline(&mut c.address).hint_text("192.168.0.10:27560  /  https://….trycloudflare.com").desired_width(f32::INFINITY));
    let unsaved = ed.doc.as_ref().map(|d| d.is_dirty()).unwrap_or(false);
    if unsaved {
        ui.colored_label(pal().warn, tr("The map open now has unsaved changes: joining closes it. Save it first.", "В открытой карте есть несохранённые изменения: подключение закроет её. Сначала сохраните."));
    }
    let can_join = !c.address.trim().is_empty() && !c.name.trim().is_empty() && !unsaved;
    if ui.add_enabled(can_join, egui::Button::new(icons::labelled(Icon::Login, tr("Join", "Подключиться")))).clicked() {
        c.join();
    }
}

fn live(ui: &mut egui::Ui, c: &mut Collab, ed: &mut Editor) {
    let settled = c.settled() && !ed.history.is_open();
    let mut leave = false;
    {
        let Some(l) = c.live() else { return };
        ui.label(RichText::new(&l.map.name).strong());
        match &l.hosting {
            Some(h) => {
                ui.label(tr("You host this map. Give the others one of these:", "Вы хост этой карты. Дайте остальным один из адресов:"));
                for (a, what) in &h.addrs {
                    copy_row(ui, a, what);
                }
                if !h.addrs.is_empty() {
                    ui.label(RichText::new(tr("On this network the connection is not encrypted: somebody listening in can see the map, and from what they catch try passwords on their own computer. Use a long password, or the internet tunnel (encrypted).", "В локальной сети соединение не шифруется: тот, кто подслушивает, видит карту и по перехваченному может подбирать пароль на своём компьютере. Используйте длинный пароль или туннель через интернет (он шифруется).")).small().color(pal().warn));
                }
                ui.label(RichText::new(tr("Everybody who joins can change the whole map.", "Каждый подключившийся может изменить всю карту.")).small().color(pal().text_dim));
                match &*h.tunnel.lock().unwrap_or_else(|e| e.into_inner()) {
                    TunnelState::Off => {}
                    TunnelState::Starting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(tr("Opening the Cloudflare tunnel… (fetched the first time)", "Открывается туннель Cloudflare… (в первый раз скачивается)"));
                        });
                    }
                    t @ TunnelState::Up(_) => match t.url() {
                        Some(u) => copy_row(ui, &u, tr("internet", "интернет")),
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(tr("Waiting for Cloudflare's address…", "Ожидание адреса от Cloudflare…"));
                            });
                        }
                    },
                    TunnelState::Failed => {
                        ui.colored_label(pal().warn, tr("The Cloudflare tunnel could not be opened (no cloudflared, or no internet). The LAN addresses still work.", "Не удалось открыть туннель Cloudflare (нет cloudflared или интернета). Адреса в локальной сети работают."));
                    }
                }
            }
            None => {
                ui.label(tr("You joined this map. The host saves it.", "Вы подключены к этой карте. Сохраняет её хост."));
            }
        }
        ui.label(RichText::new(if settled { tr("Everything of yours is with the others", "Все ваши правки у остальных") } else { tr("Sending your edits…", "Отправка ваших правок…") }).small().color(pal().text_dim));
        ui.separator();
        ui.label(format!("{} ({})", tr("In the session", "В сессии"), l.peers.len()));
        for p in &l.peers {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, Color32::from_rgb(p.color[0], p.color[1], p.color[2]));
                // (names and chat are shown as plain text)
                let mut t = RichText::new(&p.name);
                if p.id == l.you.id {
                    t = t.strong();
                }
                ui.label(t);
            });
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
            for (who, text) in &l.chat {
                let p = l.peers.iter().find(|p| p.id == *who);
                let color = p.map(|p| Color32::from_rgb(p.color[0], p.color[1], p.color[2])).unwrap_or(pal().text_dim);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(p.map(|p| p.name.as_str()).unwrap_or("?")).color(color).strong());
                    ui.label(text);
                });
            }
        });
    }
    let r = ui.add(egui::TextEdit::singleline(&mut c.say).char_limit(500).hint_text(tr("Say something…", "Написать…")).desired_width(f32::INFINITY));
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        c.say();
        r.request_focus();
    }
    ui.separator();
    if ui.button(icons::labelled(Icon::Logout, tr("Leave", "Выйти"))).clicked() {
        leave = true;
    }
    if leave {
        c.leave(ed);
    }
}

fn copy_row(ui: &mut egui::Ui, text: &str, what: &str) {
    ui.horizontal(|ui| {
        ui.monospace(text);
        ui.label(RichText::new(what).small().color(pal().text_dim));
        if icons::small(ui, Icon::Copy, tr("Copy", "Копировать")).clicked() {
            ui.ctx().copy_text(text.to_string());
        }
    });
}
