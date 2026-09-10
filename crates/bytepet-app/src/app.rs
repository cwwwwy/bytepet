use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytepet_core::config::{AppConfig, AppPaths};
use bytepet_core::deepseek::{save_api_key, DeepSeekClient};
use bytepet_core::memory::{EventKind, PetMemory};
use bytepet_core::persona::{Persona, PersonaStore};
use bytepet_core::pet::state::{PetEngine, PetState};
use bytepet_core::pet::{PetAtlas, PetEntry, PetLibrary};
use bytepet_core::state_server::{Health, StateEvent, StateServer};
use eframe::egui;

use crate::greeting;

const PET_WINDOW_MIN_WIDTH: f32 = 220.0;
const BUBBLE_AREA_HEIGHT: f32 = 110.0;

pub struct BytePetApp {
    paths: AppPaths,
    config: AppConfig,
    personas: PersonaStore,
    persona: Persona,
    memory: PetMemory,
    pet: Option<PetRuntime>,
    bubble: Option<Bubble>,
    settings_open: bool,
    status: String,
    greeting_draft: String,
    api_key_draft: String,
    greeting_rx: Option<Receiver<std::result::Result<String, String>>>,
    greeting_inflight: bool,
    last_greeting_at: Option<Instant>,
    fonts_installed: bool,
    pet_visible: bool,
    last_passthrough: Option<bool>,
    walk_direction: f32,
    next_walk_at: Instant,
    walk_until: Option<Instant>,
    walk_origin_x: Option<f32>,
    last_user_action: Instant,
    last_walk_tick: Instant,
    last_click_at: Option<Instant>,
    pending_single_click: bool,
    menu_open: bool,
    menu_pos: Option<egui::Pos2>,
    /// Pet library (Codex / UniPet / local roots) and its current contents.
    library: PetLibrary,
    pets: Vec<PetEntry>,
    selected_pet: Option<String>,
    pet_preview: Option<(String, egui::TextureHandle)>,
    /// Which side the cursor was on when the pet last glanced (-1/0/1).
    glance_side: i8,
    last_glance_at: Option<Instant>,
    tray: Option<tray_icon::TrayIcon>,
    tray_open_settings_id: Option<tray_icon::menu::MenuId>,
    tray_toggle_id: Option<tray_icon::menu::MenuId>,
    tray_quit_id: Option<tray_icon::menu::MenuId>,
    tray_events: Option<Receiver<tray_icon::menu::MenuEvent>>,
    settings_pos: Option<egui::Pos2>,
    /// Local state protocol (Codex hooks -> pet).
    state_server: Option<StateServer>,
    state_events: Option<Receiver<StateEvent>>,
    state_server_port: u16,
    last_health_at: Instant,
}

struct PetRuntime {
    entry: PetEntry,
    atlas: PetAtlas,
    engine: PetEngine,
    textures: Vec<Option<egui::TextureHandle>>,
    cell_size: egui::Vec2,
    anim_started: Instant,
    last_state: PetState,
    last_sprite: u32,
}

struct Bubble {
    text: String,
    until: Instant,
}

impl BytePetApp {
    pub fn new(paths: AppPaths, mut config: AppConfig) -> Result<Self> {
        paths.ensure()?;

        let personas = PersonaStore::new(paths.personas_dir.clone());
        personas.ensure()?;
        let persona = config
            .active_persona
            .as_ref()
            .and_then(|id| personas.get(id).ok().flatten())
            .or_else(|| {
                personas
                    .list()
                    .ok()
                    .and_then(|list| list.into_iter().next())
            })
            .unwrap_or_default();
        if config.active_persona.as_deref() != Some(persona.id.as_str()) {
            config.active_persona = Some(persona.id.clone());
        }

        let library = PetLibrary::discover(paths.pets_dir.clone());
        if !config.default_pet_seeded {
            if let Some(installed) = bytepet_core::pet::default_pet::seed_if_empty(&library)? {
                config.active_pet = Some(installed.id);
            }
            config.default_pet_seeded = true;
        }
        let pets = library.list();
        let entry = config
            .active_pet
            .as_ref()
            .and_then(|id| pets.iter().find(|pet| &pet.id == id).cloned())
            .or_else(|| pets.first().cloned());
        let pet = match entry {
            Some(entry) => {
                config.active_pet = Some(entry.id.clone());
                Some(PetRuntime::load(entry)?)
            }
            None => None,
        };

        let memory = PetMemory::open(&paths.memory_file)?;
        if let Some(pet) = &pet {
            memory.record_event(
                &persona.id,
                EventKind::AppStart,
                Some(format!("BytePet 启动：{}", pet.entry.display_name)),
            )?;
        }
        config.save(&paths.config_file)?;

        let greeting_draft = persona.greeting.clone().unwrap_or_default();
        let fallback = greeting::fallback_greeting(&persona);
        let selected_pet = config.active_pet.clone();
        let state_port = config.state_server.port;
        let mut app = Self {
            paths,
            config,
            personas,
            persona,
            memory,
            pet,
            bubble: Some(Bubble {
                text: fallback,
                until: Instant::now() + Duration::from_secs(8),
            }),
            settings_open: false,
            status: String::new(),
            greeting_draft,
            api_key_draft: String::new(),
            greeting_rx: None,
            greeting_inflight: false,
            last_greeting_at: None,
            fonts_installed: false,
            pet_visible: true,
            last_passthrough: None,
            walk_direction: 1.0,
            next_walk_at: Instant::now() + Duration::from_secs(45 * 60),
            walk_until: None,
            walk_origin_x: None,
            last_user_action: Instant::now(),
            last_walk_tick: Instant::now(),
            last_click_at: None,
            pending_single_click: false,
            menu_open: false,
            menu_pos: None,
            selected_pet,
            pet_preview: None,
            glance_side: 0,
            last_glance_at: None,
            library,
            pets,
            tray: None,
            tray_open_settings_id: None,
            tray_toggle_id: None,
            tray_quit_id: None,
            tray_events: None,
            settings_pos: None,
            state_server: None,
            state_events: None,
            state_server_port: state_port,
            last_health_at: Instant::now(),
        };
        let _ = app.trigger_greeting("startup", true);
        app.sync_state_server();
        Ok(app)
    }

    pub fn initialize(&mut self, creation_context: &eframe::CreationContext<'_>) {
        self.install_tray(creation_context.egui_ctx.clone());
    }

    fn pet_size(&self) -> egui::Vec2 {
        self.pet
            .as_ref()
            .map(|pet| pet.cell_size * self.config.window.scale.clamp(0.5, 3.0))
            .unwrap_or_else(|| egui::vec2(192.0, 208.0))
    }

    fn pet_window_size(&self) -> egui::Vec2 {
        let pet_size = self.pet_size();
        egui::vec2(
            pet_size.x.max(PET_WINDOW_MIN_WIDTH),
            pet_size.y + BUBBLE_AREA_HEIGHT,
        )
    }

    fn apply_viewport(&self, ctx: &egui::Context, window_size: egui::Vec2) {
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            if self.config.window.always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            },
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(window_size));
    }

    fn draw_pet(&mut self, root_ui: &mut egui::Ui, window_size: egui::Vec2) {
        if self.pet.is_none() {
            self.draw_missing_pet(root_ui);
            return;
        }

        let bubble_text = self
            .bubble
            .as_ref()
            .filter(|bubble| Instant::now() < bubble.until)
            .map(|bubble| bubble.text.clone());
        let pet = self.pet.as_mut().expect("checked above");
        let menu_open = self.menu_open;
        let menu_pos = self.menu_pos;

        let scale = self.config.window.scale.clamp(0.5, 3.0);
        let cell = pet.cell_size * scale;
        let total = window_size;
        let mut clicked = false;
        let mut dragging = false;
        let mut open_settings = false;
        let mut quit = false;
        let mut toggle_menu = false;
        let mut menu_pointer = None;
        let mut close_menu = false;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(total, egui::Sense::click_and_drag());
                let pet_x = rect.left() + (rect.width() - cell.x).max(0.0) * 0.5;
                let pet_rect =
                    egui::Rect::from_min_size(egui::pos2(pet_x, rect.bottom() - cell.y), cell);

                let elapsed_ms = pet.anim_started.elapsed().as_secs_f32() * 1000.0;
                let sprite = pet.current_sprite(elapsed_ms);
                if let Some(texture_id) = pet.texture_for(ui.ctx(), sprite) {
                    // SizedTexture uses `ImageFit::Exact`, so hand it the scaled
                    // size or the 大小 setting would be ignored.
                    let image = egui::Image::new(egui::load::SizedTexture::new(texture_id, cell));
                    ui.put(pet_rect, image);
                }

                if let Some(text) = &bubble_text {
                    let bubble_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.left(), rect.top()),
                        egui::vec2(rect.width(), BUBBLE_AREA_HEIGHT),
                    );
                    let galley = ui.painter().layout(
                        text.clone(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                        (bubble_rect.width() - 16.0).max(80.0),
                    );
                    ui.painter().rect_filled(
                        bubble_rect,
                        egui::CornerRadius::same(10),
                        egui::Color32::from_rgba_unmultiplied(24, 24, 28, 220),
                    );
                    let text_pos = bubble_rect.center() - galley.size() * 0.5;
                    ui.painter().galley(text_pos, galley, egui::Color32::WHITE);
                }

                if menu_open {
                    let desired = egui::vec2(140.0, 64.0);
                    let anchor = menu_pos.unwrap_or_else(|| rect.left_top());
                    let pos = egui::pos2(
                        anchor.x.clamp(
                            rect.left() + 4.0,
                            (rect.right() - desired.x - 4.0).max(rect.left() + 4.0),
                        ),
                        anchor.y.clamp(
                            rect.top() + 4.0,
                            (rect.bottom() - desired.y - 4.0).max(rect.top() + 4.0),
                        ),
                    );
                    egui::Area::new(egui::Id::new("pet-context-menu"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(pos)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(120.0);
                                if ui.button("打开设置").clicked() {
                                    open_settings = true;
                                    close_menu = true;
                                }
                                if ui.button("关闭宠物").clicked() {
                                    quit = true;
                                    close_menu = true;
                                }
                            });
                        });
                }

                dragging = response.drag_started() || response.dragged();
                clicked = response.clicked();
                if response.secondary_clicked() {
                    toggle_menu = true;
                    menu_pointer = response.interact_pointer_pos();
                }
            });

        if dragging {
            self.last_user_action = Instant::now();
            root_ui
                .ctx()
                .send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if toggle_menu {
            self.menu_open = !self.menu_open;
            self.menu_pos = menu_pointer;
        }
        if close_menu {
            self.menu_open = false;
            self.menu_pos = None;
        }
        if clicked {
            self.menu_open = false;
            self.menu_pos = None;
            let now = Instant::now();
            let is_double = self
                .last_click_at
                .is_some_and(|last| now.duration_since(last) <= Duration::from_millis(320));
            if is_double {
                self.last_click_at = None;
                self.pending_single_click = false;
                self.on_double_click();
            } else {
                self.last_click_at = Some(now);
                self.pending_single_click = true;
            }
        }
        if open_settings {
            self.settings_open = true;
            self.settings_pos = None;
        }
        if quit {
            root_ui
                .ctx()
                .send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn draw_missing_pet(&mut self, root_ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("没有找到宠物\n右键打开设置");
                });
            });
    }

    fn show_settings_viewport(&mut self, ctx: &egui::Context) {
        let viewport_id = egui::ViewportId::from_hash_of("bytepet-settings");
        let size = egui::vec2(640.0, 720.0);
        // Place the settings window next to the pet, clamped to the monitor,
        // the first time it opens. After that the user owns the position.
        let position = match self.settings_pos {
            Some(position) => position,
            None => {
                let position = self.default_settings_position(ctx, size);
                self.settings_pos = Some(position);
                position
            }
        };
        let builder = egui::ViewportBuilder::default()
            .with_title("BytePet 设置")
            .with_inner_size([size.x, size.y])
            .with_min_inner_size([420.0, 480.0])
            .with_decorations(true)
            .with_transparent(false)
            .with_taskbar(true)
            .with_position([position.x, position.y])
            .with_resizable(true);
        ctx.show_viewport_immediate(viewport_id, builder, |ui, _class| {
            self.draw_settings(ui);
        });
    }

    /// Bottom-right of the pet when there is room, otherwise the closest spot
    /// that still fits on the monitor.
    fn default_settings_position(&self, ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
        let (monitor, pet_rect) =
            ctx.input(|input| (input.viewport().monitor_size, input.viewport().outer_rect));
        let monitor = monitor.unwrap_or(egui::vec2(1280.0, 800.0));
        let pet_rect =
            pet_rect.unwrap_or_else(|| egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        let gap = 12.0;
        let right = pet_rect.right() + gap;
        let left = pet_rect.left() - size.x - gap;
        let x = if right + size.x <= monitor.x {
            right
        } else {
            left.max(0.0)
        };
        let y = (pet_rect.bottom() - size.y).clamp(0.0, (monitor.y - size.y).max(0.0));
        egui::pos2(x.clamp(0.0, (monitor.x - size.x).max(0.0)), y)
    }

    fn draw_settings(&mut self, root_ui: &mut egui::Ui) {
        if root_ui
            .ctx()
            .input(|input| input.viewport().close_requested())
        {
            self.settings_open = false;
            self.settings_pos = None;
            return;
        }
        egui::CentralPanel::default().show(root_ui, |ui| {
            let (header_rect, header_response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::drag());
            if header_response.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.painter().text(
                header_rect.left_center() + egui::vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "BytePet 设置",
                egui::FontId::proportional(16.0),
                ui.visuals().text_color(),
            );
            ui.separator();

            let mut switch_to: Option<String> = None;
            ui.collapsing("宠物", |ui| {
                let pets: Vec<(String, String, &'static str)> = self
                    .pets
                    .iter()
                    .map(|pet| {
                        (
                            pet.id.clone(),
                            pet.display_name.clone(),
                            pet.root.label(),
                        )
                    })
                    .collect();
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "当前：{}（共 {} 个，来自 ~/.codex/pets、~/.unipet/pets 与本地库）",
                        self.active_pet_name(),
                        pets.len()
                    ));
                    if ui.button("重新扫描").clicked() {
                        self.pets = self.library.list();
                        self.pet_preview = None;
                        self.status = format!("发现 {} 个宠物", self.pets.len());
                    }
                });
                if pets.is_empty() {
                    ui.label("没有找到宠物包。把 pet.json + spritesheet 放进 ~/.codex/pets 即可。");
                }
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .id_salt("pet-list")
                    .show(ui, |ui| {
                        for (id, name, root) in &pets {
                            let selected = self.selected_pet.as_deref() == Some(id.as_str());
                            if ui.radio(selected, format!("{name}  ·  {id}  ({root})")).clicked() {
                                self.selected_pet = Some(id.clone());
                            }
                        }
                    });

                if let Some(selected) = self.selected_pet.clone() {
                    let dirty = self.pet_preview.as_ref().map(|(id, _)| id.clone())
                        != Some(selected.clone());
                    if dirty {
                        self.pet_preview = self
                            .pets
                            .iter()
                            .find(|pet| pet.id == selected)
                            .and_then(|pet| {
                                pet_preview_texture(ui.ctx(), pet)
                                    .map(|texture| (pet.id.clone(), texture))
                            });
                    }
                    if let Some((id, texture)) = &self.pet_preview {
                        if id == &selected {
                            ui.horizontal(|ui| {
                                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                    texture.id(),
                                    egui::vec2(96.0, 104.0),
                                )));
                                ui.vertical(|ui| {
                                    let frame = self
                                        .pets
                                        .iter()
                                        .find(|pet| pet.id == selected)
                                        .map(|pet| pet.frame);
                                    if let Some(frame) = frame {
                                        ui.label(format!(
                                            "{} 列 × {} 行，单元格 {}×{}",
                                            frame.columns, frame.rows, frame.width, frame.height
                                        ));
                                    }
                                    if selected == self.active_pet_id() {
                                        ui.label("已经是当前宠物");
                                    } else if ui.button("切换到这个宠物").clicked() {
                                        switch_to = Some(selected.clone());
                                    }
                                });
                            });
                        }
                    }
                }
            });
            if let Some(id) = switch_to {
                self.switch_pet(&id);
            }

            ui.collapsing("人格", |ui| {
                egui::Grid::new("persona-grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("名字");
                        ui.text_edit_singleline(&mut self.persona.name);
                        ui.end_row();

                        ui.label("语气");
                        ui.text_edit_singleline(&mut self.persona.traits.tone);
                        ui.end_row();

                        ui.label("语言");
                        ui.text_edit_singleline(&mut self.persona.traits.language);
                        ui.end_row();

                        ui.label("固定问候");
                        ui.text_edit_singleline(&mut self.greeting_draft);
                        ui.end_row();
                    });
                ui.label("系统提示");
                ui.add(
                    egui::TextEdit::multiline(&mut self.persona.system_prompt)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );
            });

            ui.collapsing("宠物行为", |ui| {
                ui.horizontal(|ui| {
                    ui.label("大小");
                    ui.add(
                        egui::Slider::new(&mut self.config.window.scale, 0.5..=2.0)
                            .fixed_decimals(2),
                    );
                });
                ui.checkbox(&mut self.config.window.auto_walk.enabled, "启用活动提醒");
                egui::Grid::new("auto-walk-grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("提醒间隔（分钟）");
                        ui.add(
                            egui::DragValue::new(
                                &mut self.config.window.auto_walk.interval_minutes,
                            )
                            .range(5..=240),
                        );
                        ui.end_row();

                        ui.label("单次活动时间（秒）");
                        ui.add(
                            egui::DragValue::new(&mut self.config.window.auto_walk.walk_seconds)
                                .range(1.0..=60.0),
                        );
                        ui.end_row();

                        ui.label("移动速度");
                        ui.add(
                            egui::DragValue::new(&mut self.config.window.auto_walk.speed_px_s)
                                .range(5.0..=120.0),
                        );
                        ui.end_row();

                        ui.label("活动范围（像素）");
                        ui.add(
                            egui::DragValue::new(&mut self.config.window.auto_walk.range_px)
                                .range(20.0..=400.0),
                        );
                        ui.end_row();

                        ui.label("交互后静默（秒）");
                        ui.add(
                            egui::DragValue::new(
                                &mut self.config.window.auto_walk.user_grace_seconds,
                            )
                            .range(0.0..=300.0),
                        );
                        ui.end_row();
                    });
                ui.checkbox(&mut self.config.window.click_through, "像素级点击穿透");
            });

            ui.collapsing("状态协议", |ui| {
                ui.checkbox(
                    &mut self.config.state_server.enabled,
                    "允许本地程序驱动宠物（Codex hooks）",
                );
                ui.horizontal(|ui| {
                    ui.label("端口");
                    ui.add(
                        egui::DragValue::new(&mut self.config.state_server.port)
                            .range(1024..=65535),
                    );
                    let running = match &self.state_server {
                        Some(server) => format!("监听中 127.0.0.1:{}", server.port()),
                        None => "未运行".to_string(),
                    };
                    ui.label(running);
                });
                if let Some(server) = &self.state_server {
                    ui.label(format!(
                        "curl -XPOST http://127.0.0.1:{}/state -H \"content-type: application/json\" -d \"{{\\\"source\\\":\\\"codex\\\",\\\"state\\\":\\\"running\\\",\\\"message\\\":\\\"跑测试中\\\"}}\"",
                        server.port()
                    ));
                }
                ui.label(
                    "状态名：idle / running / waiting / failed / review / waving / jumping / running-left / running-right",
                );
                ui.label("改完端口后点「保存」生效。");
            });

            ui.collapsing("DeepSeek", |ui| {
                egui::Grid::new("deepseek-grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Base URL");
                        ui.text_edit_singleline(&mut self.config.deepseek.base_url);
                        ui.end_row();

                        ui.label("模型");
                        ui.text_edit_singleline(&mut self.config.deepseek.model);
                        ui.end_row();

                        ui.label("API Key 环境变量");
                        ui.text_edit_singleline(&mut self.config.deepseek.api_key_env);
                        ui.end_row();

                        ui.label("最大 tokens");
                        ui.add(
                            egui::DragValue::new(&mut self.config.deepseek.max_tokens)
                                .range(16..=400),
                        );
                        ui.end_row();

                        ui.label("temperature");
                        ui.add(egui::Slider::new(
                            &mut self.config.deepseek.temperature,
                            0.0..=2.0,
                        ));
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    ui.label("写入钥匙串");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.api_key_draft)
                            .password(true)
                            .hint_text("sk-..."),
                    );
                    if ui.button("保存 Key").clicked() {
                        match save_api_key(&self.api_key_draft) {
                            Ok(()) => {
                                self.api_key_draft.clear();
                                self.status = "API Key 已保存到系统钥匙串".to_string();
                            }
                            Err(error) => self.status = format!("保存 Key 失败：{error}"),
                        }
                    }
                });
            });

            ui.collapsing("记忆", |ui| {
                let facts = self.memory.list_facts(&self.persona.id);
                if facts.is_empty() {
                    ui.label("还没有记住任何事实。");
                } else {
                    for fact in facts {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}：{}", fact.key, fact.value));
                            if ui.small_button("删除").clicked() {
                                if let Err(error) =
                                    self.memory.forget_fact(&self.persona.id, &fact.id)
                                {
                                    self.status = format!("删除失败：{error}");
                                }
                            }
                        });
                    }
                }
                if ui.button("清空这个人格的记忆").clicked() {
                    match self.memory.clear_persona(&self.persona.id) {
                        Ok(()) => self.status = "记忆已清空".to_string(),
                        Err(error) => self.status = format!("清空失败：{error}"),
                    }
                }
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("保存").clicked() {
                    self.save_all();
                }
                if ui.button("测试问候").clicked() {
                    let _ = self.trigger_greeting("manual", true);
                }
                if ui.button("关闭设置").clicked() {
                    self.settings_open = false;
                }
            });
            if !self.status.is_empty() {
                ui.separator();
                ui.label(&self.status);
            }
        });
    }

    fn save_all(&mut self) {
        self.persona.greeting = if self.greeting_draft.trim().is_empty() {
            None
        } else {
            Some(self.greeting_draft.trim().to_string())
        };
        if let Err(error) = self.personas.save(&self.persona) {
            self.status = format!("保存人格失败：{error}");
            return;
        }
        if let Err(error) = self.config.save(&self.paths.config_file) {
            self.status = format!("保存配置失败：{error}");
            return;
        }
        self.sync_state_server();
        self.publish_health();
        self.status = "已保存".to_string();
    }

    /// Start, stop or restart the local state protocol to match the config.
    fn sync_state_server(&mut self) {
        let wanted = self.config.state_server.enabled;
        let port = self.config.state_server.port;
        let running = self.state_server.is_some();
        if running && (!wanted || port != self.state_server_port) {
            self.state_server = None;
            self.state_events = None;
        }
        if !wanted || self.state_server.is_some() {
            if !wanted {
                self.status = "状态服务已关闭".to_string();
            }
            return;
        }
        let (sender, receiver) = mpsc::channel();
        match StateServer::start(port, sender) {
            Ok(server) => {
                tracing::info!(port = server.port(), "state protocol listening");
                self.state_server_port = server.port();
                self.state_server = Some(server);
                self.state_events = Some(receiver);
                self.status = format!(
                    "状态协议已监听 http://127.0.0.1:{}/state",
                    self.state_server_port
                );
                self.publish_health();
            }
            Err(error) => {
                tracing::warn!(%error, "cannot start the state protocol");
                self.state_server = None;
                self.state_events = None;
                self.status = format!("状态协议启动失败：{error}");
            }
        }
    }

    /// Publish the current pet / persona / state snapshot for `GET /health`.
    fn publish_health(&self) {
        let Some(server) = &self.state_server else {
            return;
        };
        let (pet, pet_path) = self
            .pet
            .as_ref()
            .map(|pet| (pet.entry.display_name.clone(), Some(pet.entry.dir.clone())))
            .unwrap_or_else(|| ("".to_string(), None));
        let state = self
            .pet
            .as_ref()
            .map(|pet| pet.engine.current().name().to_string())
            .unwrap_or_default();
        server.set_health(Health {
            ok: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            pet,
            pet_path,
            persona: self.persona.id.clone(),
            state,
            pets: self.pets.iter().map(|pet| pet.id.clone()).collect(),
            sources: Vec::new(),
        });
    }

    /// Apply state events pushed by hooks.
    fn poll_state_events(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.state_events else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        if events.is_empty() {
            return;
        }
        for event in events {
            let Some(state) = event.pet_state() else {
                continue;
            };
            tracing::info!(source = %event.source, state = state.name(), "state event");
            let message = event.message_clipped();
            if let Some(text) = &message {
                self.show_bubble(text.clone());
            }
            if let Some(pet) = &mut self.pet {
                let source = format!("hook:{}", event.source);
                pet.engine
                    .raise(state, &source, message.clone(), event.ttl(), Instant::now());
                pet.anim_started = Instant::now();
                pet.last_state = pet.engine.current();
            }
            let _ = self.memory.record_event(
                &self.persona.id,
                EventKind::CodexStatus,
                Some(match &message {
                    Some(text) => format!("{}：{}", state.name(), text),
                    None => state.name().to_string(),
                }),
            );
            self.last_user_action = Instant::now();
            ctx.request_repaint();
        }
        self.publish_health();
    }

    fn active_pet_id(&self) -> String {
        self.pet
            .as_ref()
            .map(|pet| pet.entry.id.clone())
            .unwrap_or_default()
    }

    fn active_pet_name(&self) -> String {
        self.pet
            .as_ref()
            .map(|pet| pet.entry.display_name.clone())
            .unwrap_or_else(|| "（无）".to_string())
    }

    /// Load another pet from the library and swap it in without restarting.
    fn switch_pet(&mut self, id: &str) {
        let Some(entry) = self.pets.iter().find(|pet| pet.id == id).cloned() else {
            self.status = format!("找不到宠物 {id}");
            return;
        };
        match PetRuntime::load(entry) {
            Ok(runtime) => {
                tracing::info!(pet = %id, "switching pet");
                self.pet = Some(runtime);
                self.config.active_pet = Some(id.to_string());
                self.selected_pet = Some(id.to_string());
                self.pet_preview = None;
                self.walk_until = None;
                self.walk_origin_x = None;
                self.last_passthrough = None;
                if let Err(error) = self.config.save(&self.paths.config_file) {
                    self.status = format!("已切换，但保存配置失败：{error}");
                } else {
                    self.status = format!("已切换到 {id}");
                }
                let _ = self.memory.record_event(
                    &self.persona.id,
                    EventKind::PetChanged,
                    Some(id.to_string()),
                );
            }
            Err(error) => self.status = format!("切换宠物失败：{error:#}"),
        }
    }

    fn install_tray(&mut self, ctx: egui::Context) {
        use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};

        let menu = Menu::new();
        let open_settings = MenuItem::new("打开设置", true, None);
        let toggle_pet = MenuItem::new("显示/隐藏宠物", true, None);
        let quit = MenuItem::new("退出", true, None);
        if menu
            .append_items(&[
                &open_settings,
                &toggle_pet,
                &PredefinedMenuItem::separator(),
                &quit,
            ])
            .is_err()
        {
            tracing::warn!("cannot build tray menu");
            return;
        }

        let mut builder = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("BytePet");
        tracing::info!("creating tray icon");
        if let Ok(icon) = tray_icon::Icon::from_rgba(tray_icon_rgba(), 32, 32) {
            builder = builder.with_icon(icon);
        }
        match builder.build() {
            Ok(tray) => {
                self.tray_open_settings_id = Some(open_settings.id().clone());
                self.tray_toggle_id = Some(toggle_pet.id().clone());
                self.tray_quit_id = Some(quit.id().clone());
                self.tray = Some(tray);
                tracing::info!("tray icon created");
            }
            Err(error) => tracing::warn!(%error, "cannot create tray icon"),
        }

        // Menu events arrive on the window/message thread, so hand them to the
        // UI through a channel we own instead of polling the shared receiver
        // (which the tray crate only fills when no handler is installed).
        let (sender, receiver) = mpsc::channel();
        tray_icon::menu::MenuEvent::set_event_handler(Some(
            move |event: tray_icon::menu::MenuEvent| {
                let _ = sender.send(event);
                ctx.request_repaint();
            },
        ));
        self.tray_events = Some(receiver);
    }

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.tray_events else {
            return;
        };
        let mut events = Vec::new();
        while let Ok(event) = receiver.try_recv() {
            events.push(event);
        }
        for event in events {
            tracing::info!(id = ?event.id, "tray menu event");
            if Some(&event.id) == self.tray_open_settings_id.as_ref() {
                self.settings_open = true;
                self.settings_pos = None;
                ctx.request_repaint();
            } else if Some(&event.id) == self.tray_toggle_id.as_ref() {
                let visible = !self.pet_visible;
                self.set_pet_visible(ctx, visible);
            } else if Some(&event.id) == self.tray_quit_id.as_ref() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn set_pet_visible(&mut self, ctx: &egui::Context, visible: bool) {
        tracing::info!(visible, "set pet visible");
        self.pet_visible = visible;
        self.last_passthrough = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(!visible));
        ctx.request_repaint();
    }

    fn set_walk_state(&mut self, state: PetState) {
        if let Some(pet) = &mut self.pet {
            if pet.engine.set_base(state) {
                pet.anim_started = Instant::now();
                pet.last_state = pet.engine.current();
            }
        }
    }

    fn update_auto_walk(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.settings_open || !self.pet_visible || !self.config.window.auto_walk.enabled {
            return;
        }
        // A hook state, greeting or click animation owns the pet; the walk
        // reminder waits until the pet is back to its base animation.
        if self
            .pet
            .as_ref()
            .is_some_and(|pet| pet.engine.current() != pet.engine.base())
        {
            return;
        }
        let cfg = self.config.window.auto_walk.clone();
        if self.last_user_action.elapsed()
            < Duration::from_secs_f32(cfg.user_grace_seconds.max(0.0))
        {
            return;
        }
        let Some(window) = frame.winit_window() else {
            return;
        };
        let now = Instant::now();
        let dt = (now - self.last_walk_tick).as_secs_f32().clamp(0.0, 0.1);
        self.last_walk_tick = now;

        let Ok(position) = window.outer_position() else {
            return;
        };
        let scale = window.scale_factor() as f32;
        let current_x = position.x as f32 / scale;

        if self.walk_until.is_none() {
            if now < self.next_walk_at {
                self.set_walk_state(PetState::Idle);
                return;
            }
            self.walk_until = Some(now + Duration::from_secs_f32(cfg.walk_seconds.max(1.0)));
            self.walk_origin_x = Some(current_x);
            self.show_bubble("坐久了，起来活动一下吧。".to_string());
        }

        let Some(until) = self.walk_until else {
            return;
        };
        if now >= until {
            self.walk_until = None;
            self.walk_origin_x = None;
            self.next_walk_at = now + Duration::from_secs(cfg.interval_minutes.max(1) as u64 * 60);
            self.set_walk_state(PetState::Idle);
            return;
        }

        let origin = self.walk_origin_x.unwrap_or(current_x);
        let half_range = cfg.range_px.max(20.0) * 0.5;
        let min_x = origin - half_range;
        let max_x = origin + half_range;
        let step = cfg.speed_px_s.max(1.0) * dt * self.walk_direction;
        let mut next_x = current_x + step;
        if next_x <= min_x {
            next_x = min_x;
            self.walk_direction = 1.0;
        } else if next_x >= max_x {
            next_x = max_x;
            self.walk_direction = -1.0;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            next_x,
            position.y as f32 / scale,
        )));
        self.set_walk_state(if self.walk_direction >= 0.0 {
            PetState::RunningRight
        } else {
            PetState::RunningLeft
        });
    }

    /// Expire timed states (click wave, greeting, hook TTLs) so the pet always
    /// returns to its base animation.
    fn update_pet_timers(&mut self) {
        let now = Instant::now();
        let Some(pet) = &mut self.pet else {
            return;
        };
        if pet.engine.tick(now).is_some() {
            pet.anim_started = now;
            pet.last_state = pet.engine.current();
        }
    }

    /// Let the V2 look rows follow the cursor: crossing to one side of the pet
    /// plays that side's turn cycle exactly once.
    fn update_glance(&mut self, frame: &eframe::Frame) {
        if self.settings_open || !self.pet_visible {
            self.glance_side = 0;
            return;
        }
        let Some(pet) = &self.pet else {
            return;
        };
        if pet.engine.base().is_locomotion() || pet.engine.animation(PetState::LookRow9).is_none() {
            self.glance_side = 0;
            return;
        }
        let Some(window) = frame.winit_window() else {
            return;
        };
        let (Some((cursor_x, cursor_y)), Ok(position), size) = (
            crate::platform::global_cursor_position(),
            window.outer_position(),
            window.outer_size(),
        ) else {
            return;
        };
        let scale = window.scale_factor().max(0.1);
        let pet_size = self.pet_size();
        let window_width = size.width as f64 / scale;
        let window_height = size.height as f64 / scale;
        let pet_left =
            position.x as f64 / scale + (window_width - pet_size.x as f64).max(0.0) * 0.5;
        let pet_top = position.y as f64 / scale + (window_height - pet_size.y as f64).max(0.0);
        let dx = cursor_x - (pet_left + pet_size.x as f64 * 0.5);
        let dy = cursor_y - (pet_top + pet_size.y as f64 * 0.5);
        // Only glance when the cursor is actually near the pet.
        let reach = pet_size.x.max(pet_size.y) as f64 * 2.5;
        if dx.abs() > reach || dy.abs() > reach {
            self.glance_side = 0;
            return;
        }
        let dead_zone = pet_size.x as f64 * 0.35;
        let side = if dx > dead_zone {
            1
        } else if dx < -dead_zone {
            -1
        } else {
            0
        };
        if side == self.glance_side {
            return;
        }
        self.glance_side = side;
        if side == 0 {
            return;
        }
        if self
            .last_glance_at
            .is_some_and(|last| last.elapsed() < Duration::from_millis(900))
        {
            return;
        }
        let now = Instant::now();
        let raised = self
            .pet
            .as_mut()
            .is_some_and(|pet| pet.engine.glance(dx as f32, now).is_some());
        if raised {
            if let Some(pet) = &mut self.pet {
                pet.anim_started = now;
                pet.last_state = pet.engine.current();
            }
            self.last_glance_at = Some(now);
        }
    }

    fn update_passthrough(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // The settings window is a separate native window, so the pet keeps its
        // per-pixel click-through while the settings are open.
        if !self.pet_visible || !self.config.window.click_through {
            let ignore = !self.pet_visible;
            if self.last_passthrough != Some(ignore) {
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(ignore));
                self.last_passthrough = Some(ignore);
            }
            return;
        }
        let Some(window) = frame.winit_window() else {
            return;
        };
        let scale = window.scale_factor() as f32;
        let (cursor, window_width, window_height) =
            if let Some((global_x, global_y)) = crate::platform::global_cursor_position() {
                let Ok(position) = window.outer_position() else {
                    return;
                };
                let size = window.outer_size();
                (
                    egui::vec2(
                        (global_x - position.x as f64) as f32 / scale,
                        (global_y - position.y as f64) as f32 / scale,
                    ),
                    size.width as f32 / scale,
                    size.height as f32 / scale,
                )
            } else {
                let Some(cursor) = ctx.input(|input| input.pointer.hover_pos()) else {
                    return;
                };
                let rect = ctx.input(|input| input.viewport_rect());
                (cursor.to_vec2(), rect.width(), rect.height())
            };
        let pet_size = self.pet_size();
        let window_width = window_width.max(pet_size.x);
        let pet_x = (window_width - pet_size.x).max(0.0) * 0.5;
        let pet_y = window_height - pet_size.y;
        let inside = cursor.x >= pet_x
            && cursor.y >= pet_y
            && cursor.x < pet_x + pet_size.x
            && cursor.y < pet_y + pet_size.y;
        let ignore = if inside {
            self.pet.as_ref().is_none_or(|pet| {
                !pet.atlas.mask.opaque_at_cell_dilated(
                    pet.last_sprite,
                    cursor.x - pet_x,
                    cursor.y - pet_y,
                    1,
                )
            })
        } else {
            true
        };
        if self.last_passthrough != Some(ignore) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(ignore));
            self.last_passthrough = Some(ignore);
        }
    }

    fn on_pet_click(&mut self) {
        self.last_user_action = Instant::now();
        let _ = self
            .memory
            .record_event(&self.persona.id, EventKind::UserClick, None);
        if let Some(pet) = &mut self.pet {
            pet.engine.raise(
                PetState::Waving,
                "click",
                None,
                Some(Duration::from_secs(2)),
                Instant::now(),
            );
            pet.anim_started = Instant::now();
            pet.last_state = pet.engine.current();
        }
        if !self.trigger_greeting("click", true) {
            self.show_bubble(greeting::fallback_greeting(&self.persona));
        }
    }

    fn on_double_click(&mut self) {
        self.last_user_action = Instant::now();
        let _ =
            self.memory
                .record_event(&self.persona.id, EventKind::UserClick, Some("双击".into()));
        self.show_bubble("嘿！".to_string());
        if let Some(pet) = &mut self.pet {
            pet.engine.raise(
                PetState::Jumping,
                "double-click",
                None,
                Some(Duration::from_secs(2)),
                Instant::now(),
            );
            pet.anim_started = Instant::now();
            pet.last_state = pet.engine.current();
        }
    }

    fn trigger_greeting(&mut self, trigger: &str, force: bool) -> bool {
        if !force && !self.config.greeting.enabled {
            return false;
        }
        if !force {
            if let Some(last) = self.last_greeting_at {
                let cooldown =
                    Duration::from_secs(self.config.greeting.cooldown_minutes as u64 * 60);
                if last.elapsed() < cooldown {
                    return false;
                }
            }
        }
        if self.greeting_inflight {
            return false;
        }

        let context = self.memory.build_greeting_context(
            &self.persona.id,
            self.config.memory.recent_events,
            self.config.memory.fact_limit,
        );
        let persona = self.persona.clone();
        let config = self.config.deepseek.clone();
        let trigger = trigger.to_string();
        let now_text = greeting::local_now_text();
        let pet_name = self.pet.as_ref().map(|pet| pet.entry.display_name.clone());
        let pet_state = self
            .pet
            .as_ref()
            .map(|pet| pet.engine.current().name().to_string())
            .unwrap_or_else(|| PetState::Idle.name().to_string());

        let (sender, receiver) = mpsc::channel();
        self.greeting_rx = Some(receiver);
        self.greeting_inflight = true;
        thread::spawn(move || {
            let result = DeepSeekClient::new(config)
                .and_then(|client| {
                    client.generate_greeting(
                        &persona,
                        &context,
                        &trigger,
                        &now_text,
                        pet_name.as_deref(),
                        &pet_state,
                    )
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        true
    }

    fn poll_greeting(&mut self) {
        let received = self
            .greeting_rx
            .as_ref()
            .map(|receiver| receiver.try_recv());
        match received {
            Some(Ok(result)) => {
                self.greeting_rx = None;
                self.greeting_inflight = false;
                match result {
                    Ok(text) => {
                        self.show_bubble(text.clone());
                        if let Some(pet) = &mut self.pet {
                            pet.engine.raise(
                                PetState::Waving,
                                "greeting",
                                Some(text.clone()),
                                Some(Duration::from_secs(5)),
                                Instant::now(),
                            );
                            pet.anim_started = Instant::now();
                            pet.last_state = pet.engine.current();
                        }
                        let _ = self.memory.mark_greeted(&self.persona.id, "api", &text);
                        self.last_greeting_at = Some(Instant::now());
                    }
                    Err(error) => {
                        self.show_bubble(greeting::fallback_greeting(&self.persona));
                        self.status = error;
                    }
                }
            }
            Some(Err(TryRecvError::Empty)) => {}
            Some(Err(TryRecvError::Disconnected)) => {
                self.greeting_rx = None;
                self.greeting_inflight = false;
            }
            None => {}
        }
    }

    fn show_bubble(&mut self, text: String) {
        self.bubble = Some(Bubble {
            text,
            until: Instant::now() + Duration::from_secs(8),
        });
    }
}

impl PetRuntime {
    fn load(entry: PetEntry) -> Result<Self> {
        let (atlas, warnings) = PetAtlas::open(&entry.dir, &entry.manifest)
            .with_context(|| format!("cannot open pet '{}'", entry.id))?;
        for warning in warnings {
            tracing::warn!(pet = %entry.id, %warning, "pet atlas warning");
        }
        let frame = atlas.frame;
        // `from_atlas` follows the frames this pet actually drew instead of the
        // frame count of the reference sheet.
        let engine = PetEngine::from_atlas(&atlas, &entry.manifest);
        Ok(Self {
            entry,
            atlas,
            engine,
            textures: Vec::new(),
            cell_size: egui::vec2(frame.width as f32, frame.height as f32),
            anim_started: Instant::now(),
            last_state: PetState::Idle,
            last_sprite: 0,
        })
    }

    fn texture_for(&mut self, ctx: &egui::Context, sprite_index: u32) -> Option<egui::TextureId> {
        let index = sprite_index as usize;
        if index >= self.textures.len() {
            self.textures.resize_with(index + 1, || None);
        }
        if self.textures[index].is_none() {
            let frame = self.atlas.frame;
            let row = sprite_index / frame.columns.max(1);
            if row >= frame.rows {
                return None;
            }
            let col = sprite_index % frame.columns.max(1);
            let cell_width = frame.width as usize;
            let cell_height = frame.height as usize;
            let source_x = col * frame.width;
            let source_y = row * frame.height;
            let image = &self.atlas.image;
            let mut pixels = Vec::with_capacity(cell_width * cell_height * 4);
            for y in 0..frame.height {
                for x in 0..frame.width {
                    let pixel = image.get_pixel(source_x + x, source_y + y);
                    pixels.extend_from_slice(&pixel.0);
                }
            }
            let color =
                egui::ColorImage::from_rgba_unmultiplied([cell_width, cell_height], &pixels);
            let texture = ctx.load_texture(
                format!("pet-{}-cell-{}", self.entry.id, sprite_index),
                color,
                egui::TextureOptions::NEAREST,
            );
            self.textures[index] = Some(texture);
        }
        self.textures[index].as_ref().map(|texture| texture.id())
    }

    fn current_sprite(&mut self, elapsed_ms: f32) -> u32 {
        let state = self.engine.current();
        if state != self.last_state {
            self.last_state = state;
            self.anim_started = Instant::now();
        }

        let (sprite, finished) = {
            let Some(animation) = self.engine.current_animation() else {
                return 0;
            };
            (
                animation.sprite_at(elapsed_ms).unwrap_or(0),
                elapsed_ms >= animation.total_ms && animation.total_ms > 0.0,
            )
        };
        if finished && self.engine.current().is_one_shot() {
            self.engine.on_one_shot_finished();
            self.anim_started = Instant::now();
            self.last_state = self.engine.current();
        }
        self.last_sprite = sprite;
        sprite
    }
}

impl eframe::App for BytePetApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_tray(ctx);
        self.poll_state_events(ctx);
        self.update_pet_timers();
        self.update_auto_walk(ctx, frame);
        self.update_glance(frame);
        self.update_passthrough(ctx, frame);
        // `logic` also runs while the pet window is hidden or occluded, so the
        // `GET /health` snapshot stays fresh even when nothing is painted.
        if self.last_health_at.elapsed() >= Duration::from_secs(1) {
            self.publish_health();
            self.last_health_at = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::{CornerPreference, WindowExtWindows as _};
            if let Some(window) = frame.winit_window() {
                window.set_undecorated_shadow(false);
                window.set_border_color(None);
                window.set_corner_preference(CornerPreference::DoNotRound);
                crate::platform::clear_dwm_frame(window);
            }
        }

        if !self.fonts_installed {
            crate::fonts::install_cjk_font(ui.ctx());
            self.fonts_installed = true;
            ui.ctx().request_repaint();
        }
        if self.pending_single_click {
            if let Some(last_click) = self.last_click_at {
                if last_click.elapsed() >= Duration::from_millis(320) {
                    self.pending_single_click = false;
                    self.last_click_at = None;
                    self.on_pet_click();
                }
            } else {
                self.pending_single_click = false;
            }
        }

        let window_size = self.pet_window_size();
        self.apply_viewport(ui.ctx(), window_size);
        if self.pet_visible {
            self.draw_pet(ui, window_size);
        }

        if self.settings_open {
            self.show_settings_viewport(ui.ctx());
        }
        self.poll_greeting();

        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

/// Decode the idle frame of a pet into a texture for the settings preview.
fn pet_preview_texture(ctx: &egui::Context, entry: &PetEntry) -> Option<egui::TextureHandle> {
    let (atlas, warnings) = PetAtlas::open(&entry.dir, &entry.manifest).ok()?;
    for warning in warnings {
        tracing::debug!(pet = %entry.id, %warning, "preview atlas warning");
    }
    let frame = atlas.frame;
    let mut pixels = Vec::with_capacity((frame.width * frame.height * 4) as usize);
    for y in 0..frame.height {
        for x in 0..frame.width {
            pixels.extend_from_slice(&atlas.image.get_pixel(x, y).0);
        }
    }
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width as usize, frame.height as usize],
        &pixels,
    );
    Some(ctx.load_texture(
        format!("pet-preview-{}", entry.id),
        image,
        egui::TextureOptions::NEAREST,
    ))
}

fn tray_icon_rgba() -> Vec<u8> {
    const SIZE: i32 = 32;
    let mut pixels = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x - SIZE / 2;
            let dy = y - SIZE / 2;
            let inside = dx * dx + dy * dy <= (SIZE / 2 - 2) * (SIZE / 2 - 2);
            let eye = (11..=13).contains(&x) && (11..=13).contains(&y)
                || (18..=20).contains(&x) && (11..=13).contains(&y);
            let color = if eye {
                [24, 24, 28, 255]
            } else if inside {
                [255, 170, 64, 255]
            } else {
                [0, 0, 0, 0]
            };
            let index = ((y * SIZE + x) * 4) as usize;
            pixels[index..index + 4].copy_from_slice(&color);
        }
    }
    pixels
}
