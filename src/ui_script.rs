//! The Scripts window (Map menu): the map's Lua script for openOMSI, and the object scripts
//! (`.osc`) of OMSI 2 and openOMSI, with the game's own check, and lights switched by a script.

use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Vec2};

use crate::doc::Sel;
use crate::editor::Editor;
use crate::editable;
use crate::i18n::tr;
use crate::icons::{self, Icon};
use crate::install::Roots;
use crate::scripting::{self, Behaviour, Diag};
use crate::ui::{Action, TEXT_DIM};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Lua,
    Object,
}

/// A script being edited: where it is, its text as saved and as it stands, what the check says.
struct Editing {
    path: PathBuf,
    saved: String,
    text: String,
    diags: Vec<Diag>,
    /// The check reads the text again.
    stale: bool,
    /// May be changed: it lies in the content folder.
    writable: bool,
}

impl Editing {
    fn dirty(&self) -> bool {
        self.text != self.saved
    }
}

pub struct ScriptsUi {
    pub open: bool,
    tab: Tab,
    /// The map the Lua text belongs to.
    lua_for: String,
    lua: Option<Editing>,
    lua_exists: bool,
    template: usize,
    /// The object whose scripts are listed, and its scripts.
    object: Option<String>,
    scripts: Vec<String>,
    picked: usize,
    osc: Option<Editing>,
    declared: (Vec<String>, Vec<String>, Vec<PathBuf>),
    light: Behaviour,
    light_color: [u8; 3],
    light_range: f32,
    message: Option<(String, bool)>,
}

impl Default for ScriptsUi {
    fn default() -> Self {
        ScriptsUi {
            open: false,
            tab: Tab::Lua,
            lua_for: String::new(),
            lua: None,
            lua_exists: false,
            template: 0,
            object: None,
            scripts: Vec::new(),
            picked: 0,
            osc: None,
            declared: Default::default(),
            light: Behaviour::Hours { from: 18.0, to: 23.0 },
            light_color: [255, 220, 160],
            light_range: 12.0,
            message: None,
        }
    }
}

fn read_text(path: &Path) -> String {
    std::fs::read(path).map(|b| omsi_cfg::decode_text(&b).replace("\r\n", "\n")).unwrap_or_default()
}

fn editing(path: PathBuf, text: String, writable: bool, check: impl Fn(&str) -> Vec<Diag>) -> Editing {
    let diags = check(&text);
    Editing { path, saved: text.clone(), text, diags, stale: false, writable }
}

/// A code box with the lines numbered by the check's findings below it.
fn editor(ui: &mut egui::Ui, e: &mut Editing, id: &str, rows: usize) {
    egui::ScrollArea::vertical().id_salt(id).max_height(rows as f32 * 15.0 + 8.0).show(ui, |ui| {
        let r = ui.add(egui::TextEdit::multiline(&mut e.text).code_editor().desired_rows(rows).desired_width(f32::INFINITY).interactive(e.writable).lock_focus(true));
        if r.changed() {
            e.stale = true;
        }
    });
}

fn diagnostics(ui: &mut egui::Ui, d: &[Diag]) {
    if d.is_empty() {
        ui.label(RichText::new(format!("{} {}", icons::Icon::CheckCircle.ch(), tr("No problems found", "Проблем не найдено"))).color(Color32::from_rgb(120, 200, 130)));
        return;
    }
    for x in d.iter().take(12) {
        let text = if x.line > 0 { format!("{} {}: {}", tr("line", "строка"), x.line, x.message) } else { x.message.clone() };
        ui.label(RichText::new(text).small().color(Color32::from_rgb(240, 130, 110)));
    }
    if d.len() > 12 {
        ui.label(RichText::new(format!("… +{}", d.len() - 12)).small().color(TEXT_DIM));
    }
}

impl ScriptsUi {
    pub fn window(&mut self, ctx: &egui::Context, ed: &mut Editor, roots: Option<&Roots>, folder: &str, actions: &mut Vec<Action>) {
        if !self.open {
            return;
        }
        let Some(roots) = roots else { return };
        let Some(doc) = ed.doc.as_ref() else { return };
        let friendly = doc.global.friendly_name.clone();
        let selected_object = match ed.selected {
            Some(Sel::Object(k, id)) if ed.selection().len() == 1 => crate::selection::any_object(doc, k, id).map(|o| o.file.clone()),
            _ => None,
        };
        let mut open = self.open;
        egui::Window::new(icons::labelled(Icon::Code, tr("Scripts", "Скрипты"))).open(&mut open).default_size(Vec2::new(620.0, 560.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Lua, tr("openOMSI: the map's script", "openOMSI: скрипт карты"));
                ui.selectable_value(&mut self.tab, Tab::Object, tr("Object scripts (OMSI 2 and openOMSI)", "Скрипты объектов (OMSI 2 и openOMSI)"));
            });
            ui.separator();
            match self.tab {
                Tab::Lua => self.lua_tab(ui, roots, folder, &friendly),
                Tab::Object => self.object_tab(ui, roots, selected_object, actions),
            }
            if let Some((m, bad)) = &self.message {
                ui.add_space(4.0);
                ui.label(RichText::new(m).color(if *bad { Color32::from_rgb(240, 130, 110) } else { Color32::from_rgb(120, 200, 130) }));
            }
        });
        self.open = open;
    }

    fn lua_tab(&mut self, ui: &mut egui::Ui, roots: &Roots, folder: &str, friendly: &str) {
        if self.lua_for != folder {
            self.lua_for = folder.to_string();
            let body = scripting::read_lua(&roots.content, folder);
            self.lua_exists = body.is_some();
            self.lua = Some(editing(scripting::lua_path(&roots.content, folder), body.unwrap_or_default(), true, scripting::check_lua));
        }
        ui.label(RichText::new(tr("A Lua script that runs in openOMSI while this map is played: it reacts to what happens (a stop reached, a new hour, a new trip) and can show messages, change the traffic and the weather, move the bus. `on` and `every` work for this map only. OMSI 2 does not run Lua: its scripts belong to objects (the next tab).", "Скрипт на Lua, который openOMSI выполняет, пока играют на этой карте: он реагирует на происходящее (остановка, новый час, новый рейс), показывает сообщения, меняет движение и погоду, переставляет автобус. `on` и `every` работают только для этой карты. OMSI 2 Lua не выполняет: его скрипты принадлежат объектам (следующая вкладка).")).small().color(TEXT_DIM));
        ui.add_space(4.0);
        let templates = scripting::lua_templates();
        let mut insert = None;
        ui.horizontal(|ui| {
            ui.label(tr("Start from", "Взять за основу"));
            self.template = self.template.min(templates.len() - 1);
            egui::ComboBox::from_id_salt("lua template").selected_text(tr("a ready-made script…", "готовый скрипт…")).width(260.0).show_ui(ui, |ui| {
                for (i, t) in templates.iter().enumerate() {
                    if ui.selectable_value(&mut self.template, i, t.title).clicked() {
                        insert = Some(i);
                    }
                }
            });
        });
        let Some(e) = self.lua.as_mut() else { return };
        if let Some(i) = insert {
            if !e.text.is_empty() && !e.text.ends_with('\n') {
                e.text.push('\n');
            }
            if !e.text.trim().is_empty() {
                e.text.push('\n');
            }
            e.text.push_str(templates[i].text);
            e.stale = true;
        }
        editor(ui, e, "lua text", 22);
        if e.stale {
            e.diags = scripting::check_lua(&e.text);
            e.stale = false;
        }
        diagnostics(ui, &e.diags);
        ui.horizontal(|ui| {
            let dirty = e.dirty() || !self.lua_exists;
            if ui.add_enabled(dirty && !e.text.trim().is_empty(), egui::Button::new(icons::labelled(Icon::Save, tr("Save the script", "Сохранить скрипт")))).on_hover_text(tr("Written into the content folder's plugins, where openOMSI loads it again at once", "Записывается в plugins папки контента: openOMSI тут же загружает его заново")).clicked() {
                match scripting::write_lua(&roots.content, folder, friendly, &e.text) {
                    Ok(()) => {
                        e.saved = e.text.clone();
                        self.lua_exists = true;
                        self.message = Some((if e.diags.is_empty() { tr("Script saved", "Скрипт сохранён").to_string() } else { tr("Script saved, but it has problems: the game would not run it", "Скрипт сохранён, но в нём есть ошибки: игра его не выполнит").to_string() }, !e.diags.is_empty()));
                    }
                    Err(err) => self.message = Some((err.to_string(), true)),
                }
            }
            if ui.add_enabled(e.dirty(), egui::Button::new(tr("Revert", "Вернуть"))).clicked() {
                e.text = e.saved.clone();
                e.stale = true;
            }
            if self.lua_exists && ui.button(tr("Delete the script", "Удалить скрипт")).clicked() {
                match scripting::delete_lua(&roots.content, folder) {
                    Ok(()) => {
                        self.lua_exists = false;
                        e.text.clear();
                        e.saved.clear();
                        e.diags.clear();
                        self.message = Some((tr("Script deleted", "Скрипт удалён").to_string(), false));
                    }
                    Err(err) => self.message = Some((err.to_string(), true)),
                }
            }
        });
        ui.label(RichText::new(e.path.display().to_string()).small().color(TEXT_DIM));
    }

    /// Load the scripts of `rel` (when the selection moved to another object).
    fn load_object(&mut self, roots: &Roots, rel: Option<String>) {
        self.object = rel.clone();
        self.scripts.clear();
        self.osc = None;
        self.picked = 0;
        let Some(rel) = rel else { return };
        let Some(sco) = editable::sco_path(roots, &rel) else { return };
        let listed = scripting::sco_scripts(&omsi_cfg::decode_text(&std::fs::read(&sco).unwrap_or_default()));
        let dir = sco.parent().unwrap_or(Path::new("")).to_path_buf();
        self.declared = scripting::declared(&dir, &listed);
        self.scripts = listed.scripts;
        self.load_script(roots);
    }

    fn load_script(&mut self, roots: &Roots) {
        self.osc = None;
        let (Some(rel), Some(name)) = (self.object.clone(), self.scripts.get(self.picked).cloned()) else { return };
        let Some(sco) = editable::sco_path(roots, &rel) else { return };
        let path = omsi_cfg::resolve_path(sco.parent().unwrap_or(Path::new("")), &name);
        let writable = path.starts_with(&roots.content);
        let (vars, strs, consts) = self.declared.clone();
        self.osc = Some(editing(path.clone(), read_text(&path), writable, move |t| scripting::check_osc(t, &vars, &strs, &consts)));
    }

    fn object_tab(&mut self, ui: &mut egui::Ui, roots: &Roots, selected: Option<String>, actions: &mut Vec<Action>) {
        ui.label(RichText::new(tr("What a scenery object runs every frame: lamps that switch, signs that change, doors that open. Select an object to see its scripts; the game's own compiler checks them as you write. Scripts of the installation's objects can be read and learned from; to change one, make an object of your own (Edit the mesh makes a copy) and open it here.", "Что сценарный объект выполняет каждый кадр: лампы, которые включаются, вывески, которые меняются, двери, которые открываются. Выделите объект, чтобы увидеть его скрипты; встроенный компилятор игры проверяет их по мере набора. Скрипты объектов установки можно читать и изучать; чтобы изменить, сделайте собственный объект («Править меш» делает копию) и откройте его здесь.")).small().color(TEXT_DIM));
        ui.add_space(4.0);
        if self.object != selected {
            self.load_object(roots, selected.clone());
        }
        match &self.object {
            None => {
                ui.label(RichText::new(tr("Select one object on the map.", "Выделите на карте один объект.")).color(TEXT_DIM));
            }
            Some(rel) if self.scripts.is_empty() => {
                ui.label(format!("{}: {}", rel.rsplit('\\').next().unwrap_or(rel), tr("this object has no script", "у этого объекта нет скрипта")));
            }
            Some(rel) => {
                crate::ui::long_label(ui, rel.rsplit('\\').next().unwrap_or(rel), egui::TextStyle::Body, ui.visuals().text_color());
                let mut pick = self.picked;
                if self.scripts.len() > 1 {
                    ui.horizontal_wrapped(|ui| {
                        for (i, s) in self.scripts.iter().enumerate() {
                            ui.selectable_value(&mut pick, i, s.rsplit('\\').next().unwrap_or(s));
                        }
                    });
                }
                if pick != self.picked {
                    self.picked = pick;
                    self.load_script(roots);
                }
                let (vars, strs, consts) = self.declared.clone();
                if let Some(e) = self.osc.as_mut() {
                    if !e.writable {
                        ui.label(RichText::new(tr("From the installation: read only.", "Из установки: только чтение.")).small().color(TEXT_DIM));
                    }
                    editor(ui, e, "osc text", 16);
                    if e.stale {
                        e.diags = scripting::check_osc(&e.text, &vars, &strs, &consts);
                        e.stale = false;
                    }
                    diagnostics(ui, &e.diags);
                    // the variables it uses but the object's list lacks
                    let (nums, _) = scripting::used_vars(&e.text);
                    let missing: Vec<&String> = nums.iter().filter(|n| !vars.iter().any(|v| v.eq_ignore_ascii_case(n))).collect();
                    if !missing.is_empty() {
                        ui.label(RichText::new(format!("{}: {}", tr("Not in the object's variable list", "Нет в списке переменных объекта"), missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))).small().color(Color32::from_rgb(230, 190, 90)));
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(e.writable && e.dirty(), egui::Button::new(icons::labelled(Icon::Save, tr("Save the script", "Сохранить скрипт")))).clicked() {
                            match crate::doc::safe::write_atomic(&e.path, &omsi_cfg::encode_text(&e.text.replace('\n', "\r\n"))) {
                                Ok(()) => {
                                    e.saved = e.text.clone();
                                    crate::hot::own_change();
                                    self.message = Some((tr("Script saved", "Скрипт сохранён").to_string(), false));
                                }
                                Err(err) => self.message = Some((err.to_string(), true)),
                            }
                        }
                        if ui.add_enabled(e.dirty(), egui::Button::new(tr("Revert", "Вернуть"))).clicked() {
                            e.text = e.saved.clone();
                            e.stale = true;
                        }
                    });
                    ui.label(RichText::new(e.path.display().to_string()).small().color(TEXT_DIM));
                }
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new(tr("A light switched by a script", "Свет, включаемый скриптом")).strong());
        ui.label(RichText::new(tr("A new object: a lamp that glows and lights the ground while its script says so. The script is yours to change after.", "Новый объект: лампа, которая светит и освещает землю, пока так говорит её скрипт. Потом скрипт можно менять.")).small().color(TEXT_DIM));
        let kinds = [Behaviour::Hours { from: 18.0, to: 23.0 }, Behaviour::Blink { period: 2.0 }, Behaviour::Months { from: 12, to: 1 }];
        ui.horizontal_wrapped(|ui| {
            for k in kinds {
                let on = std::mem::discriminant(&k) == std::mem::discriminant(&self.light);
                if ui.selectable_label(on, k.label()).clicked() && !on {
                    self.light = k;
                }
            }
        });
        match &mut self.light {
            Behaviour::Hours { from, to } => {
                ui.horizontal(|ui| {
                    ui.label(tr("From", "С"));
                    ui.add(egui::DragValue::new(from).range(0.0..=24.0).speed(0.1).suffix(tr(" h", " ч")));
                    ui.label(tr("to", "до"));
                    ui.add(egui::DragValue::new(to).range(0.0..=24.0).speed(0.1).suffix(tr(" h", " ч")));
                });
            }
            Behaviour::Blink { period } => {
                ui.horizontal(|ui| {
                    ui.label(tr("A round takes", "Один круг"));
                    ui.add(egui::DragValue::new(period).range(0.4..=60.0).speed(0.1).suffix(tr(" s", " с")));
                });
            }
            Behaviour::Months { from, to } => {
                ui.horizontal(|ui| {
                    ui.label(tr("From month", "С месяца"));
                    ui.add(egui::DragValue::new(from).range(1..=12));
                    ui.label(tr("to", "по"));
                    ui.add(egui::DragValue::new(to).range(1..=12));
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label(tr("Colour", "Цвет"));
            ui.color_edit_button_srgb(&mut self.light_color);
            ui.label(tr("Reaches", "Светит на"));
            ui.add(egui::DragValue::new(&mut self.light_range).range(1.0..=100.0).suffix(" m"));
        });
        if ui.button(icons::labelled(Icon::Add, tr("Make the light", "Создать свет"))).clicked() {
            actions.push(Action::ScriptedLight(self.light, self.light_color, self.light_range));
        }
    }
}
