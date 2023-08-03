#![windows_subsystem = "windows"]

use std::{
    ptr,
    sync::{Arc, Mutex},
    thread,
};

use eframe::egui;
use serde::Deserialize;
use serde_json::json;
use winapi::um::winuser::{
    DispatchMessageW, PeekMessageW, RegisterHotKey, TranslateMessage, MOD_SHIFT, MOD_WIN, MSG,
    PM_REMOVE, WM_HOTKEY,
};

fn register_global_hotkey(visible: Arc<Mutex<bool>>) {
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
                    let mut lock = visible.lock().unwrap();

                    *lock = !*lock;
                }
            }
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        always_on_top: true,
        decorated: false,
        centered: true,
        follow_system_theme: false,
        initial_window_size: Some(egui::vec2(400.0, 500.0)),
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
    #[serde(skip)]
    completed: bool,
}

struct AchievementTracker {
    visible: Arc<Mutex<bool>>,
    search: String,
    cursor: usize,
    client: reqwest::blocking::Client,
    username: String,
    password: String,
    authenticated: bool,
    achievements: Vec<Achievement>,
}

impl Default for AchievementTracker {
    fn default() -> Self {
        let visible = Arc::new(Mutex::new(true));

        {
            let visible = visible.clone();
            thread::spawn(move || {
                register_global_hotkey(visible);
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

        Self {
            visible,
            search: String::new(),
            cursor: 0,
            client,
            username: String::new(),
            password: String::new(),
            authenticated: false,
            achievements,
        }
    }
}

impl eframe::App for AchievementTracker {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            frame.close();
            return;
        }

        let visible = *self.visible.lock().unwrap();

        frame.set_visible(visible);

        if visible {
            frame.focus();

            if self.authenticated {
                ctx.memory_mut(|m| m.stop_text_input());

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.heading(format!("Hi {}", self.username));

                    let mut achievements: Vec<_> = self
                        .achievements
                        .iter_mut()
                        .filter(|a| {
                            self.search.is_empty()
                                || a.name.to_lowercase().contains(&self.search.to_lowercase())
                        })
                        .collect();

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
                            }
                            egui::Event::Key {
                                key: egui::Key::Enter,
                                pressed: true,
                                ..
                            } => {
                                toggle_achievement(achievements[self.cursor], &self.client);
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

                    egui::ScrollArea::new([false, true])
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.vertical_centered_justified(|ui| {
                                for (i, achievement) in achievements.iter_mut().enumerate() {
                                    let text = achievement.name.clone()
                                        + if achievement.completed {
                                            " ✅"
                                        } else {
                                            " ❌"
                                        };

                                    let label = if i == self.cursor {
                                        let label = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&text)
                                                    .color(egui::Color32::from_rgb(255, 255, 255))
                                                    .background_color(egui::Color32::from_rgb(
                                                        0, 0, 0,
                                                    )),
                                            )
                                            .sense(egui::Sense::click()),
                                        );

                                        label.request_focus();
                                        ui.scroll_to_rect(label.rect, None);

                                        label
                                    } else {
                                        ui.label(&text)
                                    };

                                    if label.hovered() && ctx.input(|i| i.pointer.is_moving()) {
                                        self.cursor = i;
                                    }

                                    if label.clicked() {
                                        toggle_achievement(achievement, &self.client);
                                    }
                                }
                            });
                        })
                });
            } else {
                egui::CentralPanel::default().show(ctx, |ui| {
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

                                self.achievements
                                    .iter_mut()
                                    .filter(|a| completed.contains(&a.id))
                                    .for_each(|a| a.completed = true);
                            }

                            self.password.clear();
                        }
                    });
                });
            }
        }

        if !visible {
            self.search.clear();
            self.cursor = 0;
        }

        ctx.request_repaint();
    }
}

fn toggle_achievement(achievement: &mut Achievement, client: &reqwest::blocking::Client) {
    achievement.completed = !achievement.completed;

    let id = achievement.id;
    if achievement.completed {
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
