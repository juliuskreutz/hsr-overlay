#![windows_subsystem = "windows"]

use std::{
    ptr,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use eframe::{egui, epaint::ahash::HashMap};
use serde::Deserialize;
use serde_json::json;
use winapi::um::winuser::{
    DispatchMessageW, PeekMessageW, RegisterHotKey, TranslateMessage, MOD_SHIFT, MOD_WIN, MSG,
    PM_REMOVE, WM_HOTKEY,
};

type Callback = Arc<Mutex<Box<dyn Fn() + Send>>>;

fn register_global_hotkey(toggle: Arc<Mutex<bool>>, callback: Callback) {
    unsafe {
        RegisterHotKey(ptr::null_mut(), 1, (MOD_WIN | MOD_SHIFT) as u32, 'A' as u32);

        let mut msg = MSG {
            hwnd: ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: std::mem::zeroed(),
        };

        loop {
            let peek_value = PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE);

            if peek_value != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);

                if msg.message == WM_HOTKEY {
                    let mut lock = toggle.lock().unwrap();

                    callback.lock().unwrap()();

                    *lock = !*lock;
                }
            }

            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let p = std::env::temp_dir().join("hsr-overlay.lock");

    let mut f = fd_lock::RwLock::new(std::fs::File::create(p).unwrap());
    std::mem::forget(f.try_write().unwrap());

    let icon = eframe::IconData::try_from_png_bytes(include_bytes!("icon.png")).unwrap();

    let options = eframe::NativeOptions {
        always_on_top: true,
        decorated: false,
        centered: true,
        follow_system_theme: false,
        initial_window_size: Some(egui::vec2(400.0, 500.0)),
        icon_data: Some(icon),
        transparent: true,
        ..Default::default()
    };

    eframe::run_native(
        "Achievement Tracker",
        options,
        Box::new(|_| Box::<AchievementTracker>::default()),
    )
}

#[derive(Deserialize)]
struct Achievement {
    id: i64,
    name: String,
    related: Option<Vec<i64>>,
    #[serde(skip)]
    completed: bool,
    #[serde(skip)]
    disabled: bool,
}

struct AchievementTracker {
    toggle: Arc<Mutex<bool>>,
    callback: Callback,
    visible: bool,
    search: String,
    incomplete: bool,
    cursor: usize,
    client: reqwest::blocking::Client,
    username: String,
    password: String,
    authenticated: bool,
    achievement_ids: Vec<i64>,
    achievement_map: HashMap<i64, Achievement>,
}

impl Default for AchievementTracker {
    fn default() -> Self {
        let toggle = Arc::new(Mutex::new(false));
        let callback: Callback = Arc::new(Mutex::new(Box::new(|| {})));

        {
            let toggle = toggle.clone();
            let callback = callback.clone();
            thread::spawn(move || {
                register_global_hotkey(toggle, callback);
            });
        }

        let client = reqwest::blocking::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();

        let achievements: Vec<Achievement> = client
            .get("https://stardb.gg/api/achievements")
            .send()
            .unwrap()
            .json()
            .unwrap();

        let mut achievement_ids = Vec::new();
        let mut achievement_map = HashMap::default();

        for achievement in achievements {
            achievement_ids.push(achievement.id);

            achievement_map.insert(achievement.id, achievement);
        }

        Self {
            toggle,
            callback,
            visible: true,
            search: String::new(),
            incomplete: false,
            cursor: 0,
            client,
            username: String::new(),
            password: String::new(),
            authenticated: false,
            achievement_ids,
            achievement_map,
        }
    }
}

impl eframe::App for AchievementTracker {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            frame.close();
            return;
        }

        {
            let ctx = ctx.clone();
            *self.callback.lock().unwrap() = Box::new(move || {
                ctx.request_repaint();
            });
        }

        let mut lock = self.toggle.lock().unwrap();
        if *lock {
            *lock = false;

            self.visible = !self.visible;

            self.search.clear();
            self.incomplete = false;

            self.cursor = 0;
        }

        frame.set_visible(self.visible);

        if self.visible {
            frame.focus();

            let panel_frame = egui::Frame {
                fill: egui::Color32::from_rgba_premultiplied(12, 12, 12, 220),
                ..egui::Frame::default()
            };

            if self.authenticated {
                ctx.memory_mut(|m| m.stop_text_input());

                egui::CentralPanel::default()
                    .frame(panel_frame)
                    .show(ctx, |ui| {
                        ui.heading(format!("Hi {}", self.username));

                        let achievements: Vec<i64> = self
                            .achievement_ids
                            .iter()
                            .copied()
                            .filter(|id| {
                                (self.search.is_empty()
                                    || self.achievement_map[id]
                                        .name
                                        .to_lowercase()
                                        .contains(&self.search.to_lowercase()))
                                    && !(self.incomplete
                                        && (self.achievement_map[id].completed
                                            || self.achievement_map[id].disabled))
                            })
                            .collect();

                        let mut scroll = false;
                        let events = ui.input(|i| i.events.clone()); // avoid dead-lock by cloning. TODO(emilk): optimize
                        for event in &events {
                            match event {
                                egui::Event::Key {
                                    key: egui::Key::Backspace,
                                    pressed: true,
                                    ..
                                } => {
                                    self.search.pop();

                                    self.cursor = 0;
                                }
                                egui::Event::Key {
                                    key: egui::Key::ArrowDown,
                                    pressed: true,
                                    ..
                                }
                                | egui::Event::Key {
                                    key: egui::Key::Tab,
                                    pressed: true,
                                    modifiers: egui::Modifiers::NONE,
                                    ..
                                } => {
                                    self.cursor =
                                        self.cursor.saturating_add(1).min(achievements.len() - 1);

                                    scroll = true;
                                }
                                egui::Event::Key {
                                    key: egui::Key::ArrowUp,
                                    pressed: true,
                                    ..
                                }
                                | egui::Event::Key {
                                    key: egui::Key::Tab,
                                    pressed: true,
                                    modifiers: egui::Modifiers::SHIFT,
                                    ..
                                } => {
                                    self.cursor = self.cursor.saturating_sub(1);

                                    scroll = true;
                                }
                                egui::Event::Key {
                                    key: egui::Key::Enter,
                                    pressed: true,
                                    ..
                                } => {
                                    toggle_achievement(
                                        achievements[self.cursor],
                                        &mut self.achievement_map,
                                        &self.client,
                                    );
                                }
                                egui::Event::Text(text) => {
                                    self.search.push_str(text);

                                    self.cursor = 0;
                                }
                                _ => {}
                            }
                        }

                        if !self.search.is_empty() {
                            ui.label(&self.search);
                        } else {
                            ui.label("Type to search");
                        }

                        ui.checkbox(&mut self.incomplete, "Only Incomplete");

                        egui::ScrollArea::new([false, true])
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.vertical_centered_justified(|ui| {
                                    for (i, id) in achievements.iter().enumerate() {
                                        let achievement = &self.achievement_map[id];

                                        let text = achievement.name.clone()
                                            + if achievement.completed {
                                                " ✅"
                                            } else if achievement.disabled {
                                                " -"
                                            } else {
                                                " ❌"
                                            };

                                        let label = if i == self.cursor {
                                            let label = ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(&text)
                                                        .color(egui::Color32::from_rgb(
                                                            255, 255, 255,
                                                        ))
                                                        .background_color(egui::Color32::from_rgb(
                                                            0, 0, 0,
                                                        )),
                                                )
                                                .sense(egui::Sense::click()),
                                            );

                                            label.request_focus();

                                            if scroll {
                                                ui.scroll_to_rect(label.rect, None);
                                            }

                                            label
                                        } else {
                                            ui.label(&text)
                                        };

                                        if label.hovered()
                                            && ctx.input(|i| {
                                                i.pointer.is_moving()
                                                    || i.scroll_delta != egui::vec2(0.0, 0.0)
                                            })
                                        {
                                            self.cursor = i;
                                        }

                                        if label.clicked() {
                                            toggle_achievement(
                                                *id,
                                                &mut self.achievement_map,
                                                &self.client,
                                            );
                                        }
                                    }
                                });
                            })
                    });
            } else {
                egui::CentralPanel::default()
                    .frame(panel_frame)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading("Please Login");

                            ui.label("Username");
                            ui.text_edit_singleline(&mut self.username);
                            ui.label("Password");
                            ui.add(egui::TextEdit::singleline(&mut self.password).password(true));

                            if ui.button("Login").clicked() {
                                self.authenticated = self
                                    .client
                                    .post("https://stardb.gg/api/users/auth/login")
                                    .json(&json!({
                                      "username": self.username,
                                      "password": self.password
                                    }))
                                    .send()
                                    .unwrap()
                                    .status()
                                    == reqwest::StatusCode::OK;

                                if self.authenticated {
                                    let completed: Vec<i64> = self
                                        .client
                                        .get("https://stardb.gg/api/users/me/achievements")
                                        .send()
                                        .unwrap()
                                        .json()
                                        .unwrap();

                                    for completed in completed {
                                        self.achievement_map
                                            .entry(completed)
                                            .and_modify(|a| a.completed = true);

                                        if let Some(related) =
                                            self.achievement_map[&completed].related.clone()
                                        {
                                            for related in related {
                                                self.achievement_map
                                                    .entry(related)
                                                    .and_modify(|a| a.disabled = true);
                                            }
                                        }
                                    }
                                }

                                self.password.clear();
                            }
                        });
                    });
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }
}

fn toggle_achievement(
    id: i64,
    achievement_map: &mut HashMap<i64, Achievement>,
    client: &reqwest::blocking::Client,
) {
    achievement_map.entry(id).and_modify(|a| {
        a.completed = !a.completed;
        a.disabled = false;
    });

    if let Some(related) = achievement_map[&id].related.clone() {
        for related in related {
            achievement_map.entry(related).and_modify(|a| {
                a.completed = false;
                a.disabled = true;
            });
        }
    }

    if achievement_map[&id].completed {
        client
            .put(format!("https://stardb.gg/api/users/me/achievements/{id}"))
            .send()
            .unwrap();
    } else {
        client
            .delete(format!("https://stardb.gg/api/users/me/achievements/{id}"))
            .send()
            .unwrap();
    }
}
