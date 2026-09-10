use std::path::{Path, PathBuf};
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
const MENU_TITLE: &str = "BytePet 菜单";
const MENU_WIDTH: f32 = 176.0;
const MENU_ROW: f32 = 30.0;
const MENU_PAD: f32 = 6.0;
const MENU_ROWS: f32 = 4.0;
const MENU_SIZE: egui::Vec2 = egui::vec2(MENU_WIDTH, MENU_ROWS * MENU_ROW + MENU_PAD * 2.0);
/// How far the cursor may travel before a press becomes a drag.
const CLICK_MOVE_TOLERANCE: f32 = 4.0;
/// How long a press may last and still count as a click.
const CLICK_MAX_HOLD: Duration = Duration::from_millis(700);

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
    /// Global (monitor space) position of the open context menu.
    menu_anchor: Option<egui::Pos2>,
    /// Where the menu window actually ended up (clamped to the monitor).
    menu_window_pos: Option<egui::Pos2>,
    /// The menu window exists after the first frame it is requested.
    menu_created: bool,
    menu_styled: bool,
    menu_button_was_down: bool,
    menu_right_button_was_down: bool,
    /// Pet library (Codex / UniPet / local roots) and its current contents.
    library: PetLibrary,
    pets: Vec<PetEntry>,
    selected_pet: Option<String>,
    pet_preview: Option<(String, egui::TextureHandle)>,
    /// Which side the cursor was on when the pet last glanced (-1/0/1).
    glance_side: i8,
    last_glance_at: Option<Instant>,
    /// The pet is being moved by the user right now.
    pet_dragged: bool,
    pointer_left_down: bool,
    pointer_right_down: bool,
    /// Where the current press started (window-local) and when.
    press_origin: Option<egui::Pos2>,
    press_started_at: Option<Instant>,
    press_moved: bool,
    /// Offset from the window origin to the cursor when the drag started.
    drag_grab: Option<egui::Vec2>,
    last_window_pos: Option<egui::Vec2>,
    tray: Option<tray_icon::TrayIcon>,
    tray_events: Option<Receiver<tray_icon::TrayIconEvent>>,
    settings_pos: Option<egui::Pos2>,
    /// Local state protocol (Codex hooks -> pet).
    state_server: Option<StateServer>,
    state_events: Option<Receiver<StateEvent>>,
    state_server_port: u16,
    last_health_at: Instant,
    /// Pet import / export state for the settings window.
    import_draft: String,
    pending_overwrite: Option<PathBuf>,
    pending_delete: Option<String>,
    /// Cached viewport commands. Re-sending them every frame made Windows
    /// redraw the non-client frame, which showed up as a flashing border.
    applied_window_size: Option<egui::Vec2>,
    applied_always_on_top: Option<bool>,
    window_chrome_ready: bool,
    /// When the window was last resized; the Windows chrome is re-applied once
    /// the resize settles instead of on every step of a drag.
    resize_settled_at: Option<Instant>,
    /// Icon built from the active pet, cached by pet id.
    pet_icon: Option<(String, std::sync::Arc<egui::IconData>)>,
    applied_window_icon: Option<String>,
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

/// What the context menu should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Dismiss,
    OpenSettings,
    TogglePet,
    Quit,
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
        // The bundled pet lives in the local library next to the user's own
        // pets. Deleting it in the settings opts out for good.
        if let Some(installed) =
            bytepet_core::pet::default_pet::ensure_installed(&library, config.bundled_pet_removed)?
        {
            tracing::info!(pet = %installed.id, "installed the bundled pet");
            if config.active_pet.is_none() {
                config.active_pet = Some(installed.id);
            }
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
            menu_anchor: None,
            menu_window_pos: None,
            menu_created: false,
            menu_styled: false,
            menu_button_was_down: false,
            menu_right_button_was_down: false,
            selected_pet,
            pet_preview: None,
            glance_side: 0,
            last_glance_at: None,
            pet_dragged: false,
            pointer_left_down: false,
            pointer_right_down: false,
            press_origin: None,
            press_started_at: None,
            press_moved: false,
            drag_grab: None,
            last_window_pos: None,
            library,
            pets,
            tray: None,
            tray_events: None,
            settings_pos: None,
            state_server: None,
            state_events: None,
            state_server_port: state_port,
            last_health_at: Instant::now(),
            import_draft: String::new(),
            pending_overwrite: None,
            pending_delete: None,
            applied_window_size: None,
            applied_always_on_top: None,
            window_chrome_ready: false,
            resize_settled_at: None,
            pet_icon: None,
            applied_window_icon: None,
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
        let cell = self.pet_cell_size();
        // The window is sized in quarter steps of the scale slider. The sprite
        // itself still follows the scale continuously, but the native window
        // only resizes when a quarter step is crossed - resizing it on every
        // frame of a drag is what made the pet flicker.
        let stage = self.stage_scale();
        let stage_size = cell * stage;
        egui::vec2(
            stage_size.x.max(PET_WINDOW_MIN_WIDTH),
            stage_size.y + BUBBLE_AREA_HEIGHT,
        )
    }

    /// Unscaled atlas cell size of the active pet.
    fn pet_cell_size(&self) -> egui::Vec2 {
        self.pet
            .as_ref()
            .map(|pet| pet.cell_size)
            .unwrap_or_else(|| egui::vec2(192.0, 208.0))
    }

    /// Window scale rounded up to the next quarter step.
    fn stage_scale(&self) -> f32 {
        let scale = self.config.window.scale.clamp(0.5, 3.0);
        ((scale * 4.0).ceil() / 4.0).max(0.5)
    }

    fn apply_viewport(&mut self, ctx: &egui::Context, window_size: egui::Vec2) {
        let level = self.config.window.always_on_top;
        if self.applied_always_on_top != Some(level) {
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if level {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }));
            self.applied_always_on_top = Some(level);
        }
        let target = egui::vec2(window_size.x.round(), window_size.y.round());
        if self.applied_window_size != Some(target) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
            self.applied_window_size = Some(target);
            self.resize_settled_at = Some(Instant::now());
        }
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
        // Compute the sprite rectangle before borrowing the pet mutably.
        let pet_rect = self.pet_rect(window_size);
        let scale = self.config.window.scale.clamp(0.5, 3.0);
        let total = window_size;
        let pet = self.pet.as_mut().expect("checked above");
        let cell = pet.cell_size * scale;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::TRANSPARENT))
            .show(root_ui, |ui| {
                // Input is handled from the system cursor and button state (the
                // window is permanently click-through), so the pet itself only
                // needs a passive area to draw into.
                let (rect, _response) = ui.allocate_exact_size(total, egui::Sense::hover());
                let pet_rect = pet_rect.translate(rect.min.to_vec2());

                let elapsed_ms = pet.anim_started.elapsed().as_secs_f32() * 1000.0;
                let sprite = pet.current_sprite(elapsed_ms);
                if let Some(texture_id) = pet.texture_for(ui.ctx(), sprite) {
                    // SizedTexture uses `ImageFit::Exact`, so hand it the scaled
                    // size or the 大小 setting would be ignored.
                    let image = egui::Image::new(egui::load::SizedTexture::new(texture_id, cell));
                    ui.put(pet_rect, image);
                }

                if let Some(text) = &bubble_text {
                    draw_bubble(ui.painter(), rect, pet_rect, text);
                }
            });
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

    /// Real popup menu: its own native window, sized to the items, so it can
    /// open at the cursor and is never clipped by the pet window.
    fn show_context_menu(&mut self, ctx: &egui::Context) {
        let size = MENU_SIZE;
        let anchor = self.menu_anchor.unwrap_or(egui::Pos2::ZERO);
        let target = clamp_to_monitor(ctx, anchor, size);
        // The window is created one frame early as a tiny transparent square
        // under the cursor, then resized into place. It is on screen (so the
        // backend really paints it) but invisible, which avoids the flash of a
        // window being shown before its first paint.
        let first_frame = !self.menu_created;
        let (position, size) = if first_frame {
            (anchor, egui::vec2(8.0, 8.0))
        } else {
            (target, MENU_SIZE)
        };
        let builder = egui::ViewportBuilder::default()
            .with_title(MENU_TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_resizable(false)
            .with_active(false)
            .with_inner_size([size.x, size.y])
            .with_position([position.x, position.y]);
        let builder = match self.pet_icon() {
            Some(icon) => builder.with_icon(icon),
            None => builder,
        };

        let mut action: Option<MenuAction> = None;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("bytepet-menu"),
            builder,
            |ui, _class| {
                if first_frame {
                    // Nothing may be drawn yet: this frame only exists to create
                    // and paint the window. Drawing here is what showed up as a
                    // small dark dot before the menu appeared.
                    return;
                }
                egui::Frame::popup(ui.style())
                    .inner_margin(egui::Margin::same(MENU_PAD as i8))
                    .show(ui, |ui| {
                        ui.set_min_width(MENU_WIDTH - MENU_PAD * 2.0);
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                        let toggle = if self.pet_visible {
                            "隐藏宠物"
                        } else {
                            "显示宠物"
                        };
                        let entries = [
                            ("打开设置", MenuAction::OpenSettings),
                            ("更换宠物", MenuAction::OpenSettings),
                            (toggle, MenuAction::TogglePet),
                            ("退出", MenuAction::Quit),
                        ];
                        for (label, candidate) in entries {
                            let button = egui::Button::new(label)
                                .min_size(egui::vec2(MENU_WIDTH - MENU_PAD * 2.0, MENU_ROW - 2.0));
                            if ui.add(button).clicked() {
                                action = Some(candidate);
                            }
                        }
                    });
                if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
                    action = Some(MenuAction::Dismiss);
                }
            },
        );
        self.menu_created = true;
        self.menu_window_pos = Some(target);
        if !self.menu_styled {
            // Menus never activate, so they cannot steal focus (or repaint a
            // frame) either. Retry until the window really exists.
            self.menu_styled = crate::platform::set_no_activate_for_title(MENU_TITLE) > 0;
        }
        let Some(action) = action else {
            return;
        };
        self.dismiss_menu();
        match action {
            MenuAction::Dismiss => {}
            MenuAction::OpenSettings => {
                self.settings_open = true;
                self.settings_pos = None;
            }
            MenuAction::TogglePet => {
                let visible = !self.pet_visible;
                self.set_pet_visible(ctx, visible);
            }
            MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn dismiss_menu(&mut self) {
        self.menu_open = false;
        self.menu_anchor = None;
        self.menu_window_pos = None;
        self.menu_created = false;
        self.menu_styled = false;
        self.menu_button_was_down = false;
        self.menu_right_button_was_down = false;
    }

    /// Menus do not take focus, so "clicked somewhere else" and Escape are read
    /// from the system instead of from focus events.
    fn poll_menu(&mut self, ctx: &egui::Context) {
        if !self.menu_open {
            self.menu_button_was_down = false;
            self.menu_right_button_was_down = false;
            return;
        }
        if crate::platform::escape_pressed() {
            self.dismiss_menu();
            return;
        }
        let button_down = crate::platform::primary_button_down()
            .unwrap_or_else(|| ctx.input(|input| input.pointer.any_down()));
        let right_down = crate::platform::secondary_button_down()
            .unwrap_or_else(|| ctx.input(|input| input.pointer.secondary_down()));
        let pressed_left = button_down && !self.menu_button_was_down;
        let pressed_right = right_down && !self.menu_right_button_was_down;
        self.menu_button_was_down = button_down;
        self.menu_right_button_was_down = right_down;
        if !(pressed_left || pressed_right) || !self.menu_created {
            return;
        }
        let (Some(menu_pos), Some((cursor_x, cursor_y))) = (
            self.menu_window_pos,
            crate::platform::global_cursor_position(),
        ) else {
            return;
        };
        let scale = ctx
            .input(|input| input.viewport().native_pixels_per_point)
            .unwrap_or(1.0) as f64;
        let menu_rect = egui::Rect::from_min_size(menu_pos, MENU_SIZE);
        let cursor = egui::pos2(
            cursor_x as f32 / scale as f32,
            cursor_y as f32 / scale as f32,
        );
        if !menu_rect.contains(cursor) {
            self.dismiss_menu();
        }
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
        let builder = match self.pet_icon() {
            Some(icon) => builder.with_icon(icon),
            None => builder,
        };
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
        self.handle_dropped_files(root_ui.ctx());
        if root_ui
            .ctx()
            .input(|input| input.viewport().close_requested())
        {
            self.settings_open = false;
            self.settings_pos = None;
            return;
        }
        egui::CentralPanel::default().show(root_ui, |ui| {
            // The window has native decorations now, so it scrolls as a whole
            // instead of pretending to be a title bar.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_settings_body(ui);
                });
        });
    }

    fn draw_settings_body(&mut self, ui: &mut egui::Ui) {
        let mut switch_to: Option<String> = None;
        let mut import_request: Option<(PathBuf, bool)> = None;
        let mut export_id: Option<String> = None;
        let mut delete_id: Option<String> = None;
        ui.collapsing("宠物", |ui| {
            let pets: Vec<(String, String, bool)> = self
                .pets
                .iter()
                .map(|pet| {
                    let local = pet.root == bytepet_core::pet::RootKind::AppData;
                    let mut label = format!(
                        "{}  ·  {}  ({})",
                        pet.display_name,
                        pet.id,
                        pet.root.label()
                    );
                    if local && pet.id == bytepet_core::pet::DEFAULT_PET_ID {
                        label.push_str("  ·  内置");
                    }
                    (pet.id.clone(), label, local)
                })
                .collect();
            ui.horizontal(|ui| {
                ui.label(format!("当前：{}", self.active_pet_name()));
                ui.label(format!(
                    "共 {} 个（本地库 {} 个，其余来自 ~/.codex/pets、~/.unipet/pets）",
                    pets.len(),
                    pets.iter().filter(|(_, _, local)| *local).count()
                ));
            });
            ui.horizontal(|ui| {
                ui.label("导入");
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_draft)
                        .hint_text("宠物文件夹或 .zip 的路径")
                        .desired_width(240.0),
                );
                if ui
                    .button("导入")
                    .on_hover_text("把文件夹或 .zip 拖到窗口里也可以导入")
                    .clicked()
                {
                    let draft = self.import_draft.trim().to_string();
                    if !draft.is_empty() {
                        import_request = Some((PathBuf::from(draft), false));
                    }
                }
                if ui.button("打开宠物库目录").clicked() {
                    if crate::platform::open_in_file_manager(&self.paths.pets_dir) {
                        self.status = format!("宠物库：{}", self.paths.pets_dir.display());
                    } else {
                        self.status = format!("宠物库目录：{}", self.paths.pets_dir.display());
                    }
                }
                if ui.button("重新扫描").clicked() {
                    self.refresh_pets();
                    self.status = format!("发现 {} 个宠物", self.pets.len());
                }
            });
            if let Some(path) = self.pending_overwrite.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("{} 与本地库里的宠物同 id。", path.display()));
                    if ui.button("覆盖导入").clicked() {
                        import_request = Some((path.clone(), true));
                    }
                    if ui.button("取消").clicked() {
                        self.pending_overwrite = None;
                    }
                });
            }
            if pets.is_empty() {
                ui.label("没有找到宠物包。用上面的「导入」或直接拖进窗口即可。");
            }
            egui::ScrollArea::vertical()
                .max_height(150.0)
                .id_salt("pet-list")
                .show(ui, |ui| {
                    for (id, label, _local) in &pets {
                        let selected = self.selected_pet.as_deref() == Some(id.as_str());
                        if ui.radio(selected, label).clicked() {
                            self.selected_pet = Some(id.clone());
                        }
                    }
                });

            if let Some(selected) = self.selected_pet.clone() {
                let dirty =
                    self.pet_preview.as_ref().map(|(id, _)| id.clone()) != Some(selected.clone());
                if dirty {
                    self.pet_preview =
                        self.pets
                            .iter()
                            .find(|pet| pet.id == selected)
                            .and_then(|pet| {
                                pet_preview_texture(ui.ctx(), pet)
                                    .map(|texture| (pet.id.clone(), texture))
                            });
                }
                // Read everything the row needs before opening the nested
                // closures, so they never borrow `self` while it is used.
                let texture_id = self
                    .pet_preview
                    .as_ref()
                    .filter(|(id, _)| id == &selected)
                    .map(|(_, texture)| texture.id());
                let frame = self
                    .pets
                    .iter()
                    .find(|pet| pet.id == selected)
                    .map(|pet| pet.frame);
                let local = self
                    .pets
                    .iter()
                    .find(|pet| pet.id == selected)
                    .is_some_and(|pet| pet.root == bytepet_core::pet::RootKind::AppData);
                let is_active = selected == self.active_pet_id();
                let confirm_delete = self.pending_delete.as_deref() == Some(selected.as_str());
                if let Some(texture_id) = texture_id {
                    ui.horizontal(|ui| {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            texture_id,
                            egui::vec2(96.0, 104.0),
                        )));
                        ui.vertical(|ui| {
                            if let Some(frame) = frame {
                                ui.label(format!(
                                    "{} 列 × {} 行，单元格 {}×{}",
                                    frame.columns, frame.rows, frame.width, frame.height
                                ));
                            }
                            if is_active {
                                ui.label("已经是当前宠物");
                            } else if ui.button("切换到这个宠物").clicked() {
                                switch_to = Some(selected.clone());
                            }
                            ui.horizontal(|ui| {
                                if ui.button("导出为 zip").clicked() {
                                    export_id = Some(selected.clone());
                                }
                                if !local {
                                    ui.label("（来自 Codex/UniPet，只读引用）");
                                } else if confirm_delete {
                                    if ui.button("确认删除").clicked() {
                                        delete_id = Some(selected.clone());
                                    }
                                    if ui.button("取消").clicked() {
                                        self.pending_delete = None;
                                    }
                                } else if ui.button("从本地库删除").clicked() {
                                    self.pending_delete = Some(selected.clone());
                                }
                            });
                        });
                    });
                }
            }
            if Self::files_are_hovering(ui.ctx()) {
                ui.label("松手即可把宠物导入本地库");
            }
        });
        if let Some(id) = switch_to {
            self.switch_pet(&id);
        }
        if let Some((path, overwrite)) = import_request {
            self.import_path(&path, overwrite);
        }
        if let Some(id) = export_id {
            self.export_pet_zip(&id);
        }
        if let Some(id) = delete_id {
            self.remove_local_pet(&id);
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
                    egui::Slider::new(&mut self.config.window.scale, 0.5..=2.0).fixed_decimals(2),
                );
            });
            ui.checkbox(&mut self.config.window.auto_walk.enabled, "启用活动提醒");
            egui::Grid::new("auto-walk-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("提醒间隔（分钟）");
                    ui.add(
                        egui::DragValue::new(&mut self.config.window.auto_walk.interval_minutes)
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
                        egui::DragValue::new(&mut self.config.window.auto_walk.user_grace_seconds)
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
                        egui::DragValue::new(&mut self.config.deepseek.max_tokens).range(16..=400),
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
                            if let Err(error) = self.memory.forget_fact(&self.persona.id, &fact.id)
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

    /// Re-scan the local library and the linked Codex / UniPet roots.
    fn refresh_pets(&mut self) {
        self.pets = self.library.list();
        self.pet_preview = None;
        self.publish_health();
    }

    /// Icon of the active pet (its idle pose), built once per pet and reused by
    /// the settings window, the menu and the tray.
    fn pet_icon(&mut self) -> Option<std::sync::Arc<egui::IconData>> {
        const SIZE: u32 = 128;
        let id = self.pet.as_ref()?.entry.id.clone();
        if let Some((cached, icon)) = &self.pet_icon {
            if cached == &id {
                return Some(std::sync::Arc::clone(icon));
            }
        }
        let rgba = {
            let pet = self.pet.as_ref()?;
            let sprite = pet.current_sprite_index();
            pet.atlas.icon_rgba(sprite, SIZE)?
        };
        let icon = std::sync::Arc::new(egui::IconData {
            rgba,
            width: SIZE,
            height: SIZE,
        });
        self.pet_icon = Some((id, std::sync::Arc::clone(&icon)));
        Some(icon)
    }

    /// Redraw the tray icon from the active pet.
    fn refresh_tray_icon(&mut self) {
        const SIZE: u32 = 64;
        let Some(tray) = &self.tray else {
            return;
        };
        let Some(pet) = self.pet.as_ref() else {
            return;
        };
        let sprite = pet.current_sprite_index();
        let Some(rgba) = pet.atlas.icon_rgba(sprite, SIZE) else {
            return;
        };
        if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, SIZE, SIZE) {
            let _ = tray.set_icon(Some(icon));
        }
    }

    /// Import a pet folder or `.zip` into the app-local library.
    ///
    /// Refuses invalid packages (the library validates manifest, geometry,
    /// decode and path safety first) and asks for confirmation before
    /// overwriting an existing id.
    fn import_path(&mut self, path: &Path, overwrite: bool) {
        if !path.exists() {
            self.status = format!(
                "找不到 {}：可以把宠物文件夹或 .zip 拖到窗口里",
                path.display()
            );
            return;
        }
        let is_zip = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if !path.is_dir() && !is_zip {
            self.status = "只支持宠物文件夹或 .zip 压缩包".to_string();
            return;
        }
        let result = if path.is_dir() {
            self.library.import_dir(path, overwrite)
        } else {
            self.library.import_zip(path, overwrite)
        };
        match result {
            Ok(entry) => {
                tracing::info!(pet = %entry.id, "imported pet");
                self.pending_overwrite = None;
                self.selected_pet = Some(entry.id.clone());
                self.refresh_pets();
                self.switch_pet(&entry.id);
                self.status = format!("已导入并切换到 {}（本地库）", entry.display_name);
            }
            Err(error) => {
                let text = format!("{error:#}");
                if text.contains("already exists") {
                    self.pending_overwrite = Some(path.to_path_buf());
                    self.status = format!("本地库已有同名宠物，点「覆盖导入」确认覆盖：{text}");
                } else {
                    self.pending_overwrite = None;
                    self.status = format!("导入失败：{text}");
                }
            }
        }
    }

    /// Take pet packages dropped onto a window (folder or `.zip`).
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            // A single drop of several files keeps the first pet's id; the
            // remaining ones import on their own because the ids differ.
            let overwrite = self.pending_overwrite.as_deref() == Some(path.as_path());
            self.import_path(&path, overwrite);
        }
    }

    /// True while the user is dragging files over the window.
    fn files_are_hovering(ctx: &egui::Context) -> bool {
        ctx.input(|input| !input.raw.hovered_files.is_empty())
    }

    /// Write the Codex upload format next to the config directory.
    fn export_pet_zip(&mut self, id: &str) {
        let exports = self.paths.config_dir.join("exports");
        let out = exports.join(format!("{id}.zip"));
        match self.library.export_zip(id, &out) {
            Ok(()) => {
                self.status = format!("已导出 {}", out.display());
                if crate::platform::open_in_file_manager(&exports) {
                    self.status.push_str("（已打开导出目录）");
                }
            }
            Err(error) => self.status = format!("导出失败：{error:#}"),
        }
    }

    /// Delete a pet from the app-local library (linked Codex pets are never
    /// touched). Requires the confirmation stored in `pending_delete`.
    fn remove_local_pet(&mut self, id: &str) {
        match self.library.remove_local(id) {
            Ok(()) => {
                self.pending_delete = None;
                if id == bytepet_core::pet::DEFAULT_PET_ID {
                    // Do not resurrect a pet the user deleted on purpose.
                    self.config.bundled_pet_removed = true;
                    let _ = self.config.save(&self.paths.config_file);
                }
                if self.active_pet_id() == id {
                    self.refresh_pets();
                    if let Some(next) = self.pets.first().map(|pet| pet.id.clone()) {
                        self.switch_pet(&next);
                    } else {
                        self.pet = None;
                    }
                } else {
                    self.refresh_pets();
                }
                self.status = format!("已从本地库删除 {id}");
            }
            Err(error) => self.status = format!("删除失败：{error:#}"),
        }
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
                self.pet_icon = None;
                self.refresh_tray_icon();
                self.walk_until = None;
                self.walk_origin_x = None;
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
        // The tray has no native menu on purpose: a native menu runs a modal
        // Win32 menu loop on the event-loop thread, which froze the whole app
        // (the pet stopped responding and the menu items did nothing).
        // Clicking the icon opens our own popup menu instead - the same window
        // the pet's right-click menu uses.
        let mut builder = tray_icon::TrayIconBuilder::new()
            .with_menu_on_left_click(false)
            .with_menu_on_right_click(false)
            .with_tooltip("BytePet");
        tracing::info!("creating tray icon");
        if let Ok(icon) = tray_icon::Icon::from_rgba(tray_icon_rgba(), 32, 32) {
            builder = builder.with_icon(icon);
        }
        match builder.build() {
            Ok(tray) => {
                self.tray = Some(tray);
                tracing::info!("tray icon created");
                self.refresh_tray_icon();
            }
            Err(error) => tracing::warn!(%error, "cannot create tray icon"),
        }

        // Click events arrive on the message thread, so hand them to the UI
        // through a channel we own and wake the event loop.
        let (sender, receiver) = mpsc::channel();
        tray_icon::TrayIconEvent::set_event_handler(Some(
            move |event: tray_icon::TrayIconEvent| {
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
        let scale = ctx
            .input(|input| input.viewport().native_pixels_per_point)
            .unwrap_or(1.0);
        for event in events {
            let tray_icon::TrayIconEvent::Click {
                button_state,
                position,
                ..
            } = event
            else {
                continue;
            };
            if button_state != tray_icon::MouseButtonState::Down {
                continue;
            }
            tracing::info!(?position, "tray click");
            self.open_menu_at(egui::pos2(
                position.x as f32 / scale,
                position.y as f32 / scale,
            ));
            ctx.request_repaint();
        }
    }

    /// Open the shared popup menu with its top-left at `anchor`.
    fn open_menu_at(&mut self, anchor: egui::Pos2) {
        self.menu_open = true;
        self.menu_created = false;
        self.menu_styled = false;
        self.menu_button_was_down = false;
        self.menu_right_button_was_down = true;
        self.menu_anchor = Some(anchor);
    }

    fn set_pet_visible(&mut self, ctx: &egui::Context, visible: bool) {
        tracing::info!(visible, "set pet visible");
        self.pet_visible = visible;
        self.last_passthrough = None;
        self.press_origin = None;
        self.pet_dragged = false;
        self.drag_grab = None;
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
        // The user is holding the pet; the reminder can wait.
        if self.pet_dragged {
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

    /// Play the locomotion row that matches how the pet is being carried.
    ///
    /// The state is raised with a short TTL and refreshed while the window
    /// keeps moving, so it always falls back to the base animation on its own -
    /// no "drag ended" bookkeeping can get stuck.
    fn raise_motion(&mut self, state: PetState) {
        const MOTION_TTL: Duration = Duration::from_millis(300);
        let now = Instant::now();
        let Some(pet) = &mut self.pet else {
            return;
        };
        let already_running =
            pet.engine.current() == state && pet.engine.source() == Some("motion");
        let raised = pet
            .engine
            .raise(state, "motion", None, Some(MOTION_TTL), now)
            .is_some();
        if raised && !already_running {
            pet.anim_started = now;
            pet.last_state = state;
        }
    }

    /// Move the pet with the cursor ourselves.
    ///
    /// Handing the drag to the OS (`ViewportCommand::StartDrag`) enters a modal
    /// move loop: it blocks the event loop for the whole gesture, so the pet
    /// cannot animate, and asking for it again on every frame re-entered that
    /// loop and flashed the window frame. Instead the window is positioned from
    /// the cursor each frame, which also lets the locomotion row play.
    fn drag_pet(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            return;
        };
        let button_down = crate::platform::primary_button_down()
            .unwrap_or_else(|| ctx.input(|input| input.pointer.any_down()));
        if self.pet_dragged && !button_down {
            self.pet_dragged = false;
            self.drag_grab = None;
        }

        let Ok(position) = window.outer_position() else {
            return;
        };
        let scale = window.scale_factor().max(0.1);
        let current = egui::vec2(
            position.x as f32 / scale as f32,
            position.y as f32 / scale as f32,
        );
        let previous = self.last_window_pos.replace(current);

        if self.pet_dragged {
            let cursor = crate::platform::global_cursor_position();
            if self.drag_grab.is_none() {
                if let Some((cursor_x, cursor_y)) = cursor {
                    self.drag_grab = Some(egui::vec2(
                        cursor_x as f32 / scale as f32 - current.x,
                        cursor_y as f32 / scale as f32 - current.y,
                    ));
                }
            }
            if let (Some(grab), Some((cursor_x, cursor_y))) = (self.drag_grab, cursor) {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                    cursor_x as f32 / scale as f32 - grab.x,
                    cursor_y as f32 / scale as f32 - grab.y,
                )));
            }
        }

        let dx = previous.map_or(0.0, |previous| current.x - previous.x);
        if dx.abs() >= 0.5 {
            let state = if dx > 0.0 {
                PetState::RunningRight
            } else {
                PetState::RunningLeft
            };
            self.raise_motion(state);
        }
    }

    /// Let the V2 look rows follow the cursor: crossing to one side of the pet
    /// plays that side's turn cycle exactly once.
    fn update_glance(&mut self, frame: &eframe::Frame) {
        if self.settings_open || !self.pet_visible || self.pet_dragged {
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

    /// Rectangle of the pet sprite inside the window (unscaled window space).
    fn pet_rect(&self, window_size: egui::Vec2) -> egui::Rect {
        let size = self.pet_size();
        egui::Rect::from_min_size(
            egui::pos2(
                (window_size.x - size.x).max(0.0) * 0.5,
                (window_size.y - size.y).max(0.0),
            ),
            size,
        )
    }

    /// Is the cursor on a drawn pixel of the pet?
    ///
    /// The window is permanently click-through, so this - plus the Win32 button
    /// state - is what decides whether a press belongs to the pet.
    fn cursor_over_pet(&self, window_size: egui::Vec2, local: egui::Pos2) -> bool {
        let Some(pet) = &self.pet else {
            return false;
        };
        let rect = self.pet_rect(window_size);
        if !rect.contains(local) {
            return false;
        }
        // With the "pixel-level click-through" option off, the whole sprite
        // rectangle reacts; with it on, only drawn pixels do.
        if !self.config.window.click_through {
            return true;
        }
        pet.atlas.mask.opaque_at_cell_dilated(
            pet.last_sprite,
            local.x - rect.min.x,
            local.y - rect.min.y,
            1,
        )
    }

    /// Keep the window click-through on exactly the pixels the pet does not
    /// draw, so the desktop below stays usable while the sprite still reacts.
    /// The window carries no frame styles, so changing this style can no longer
    /// make Windows paint a border.
    fn update_passthrough(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.pet_dragged {
            return;
        }
        let ignore = if !self.pet_visible {
            true
        } else if !self.config.window.click_through {
            false
        } else {
            let Some(window) = frame.winit_window() else {
                return;
            };
            let (cursor, position) = (
                crate::platform::global_cursor_position(),
                window.outer_position(),
            );
            let (Some((cursor_x, cursor_y)), Ok(position)) = (cursor, position) else {
                return;
            };
            let scale = window.scale_factor().max(0.1) as f32;
            let local = egui::pos2(
                (cursor_x - position.x as f64) as f32 / scale,
                (cursor_y - position.y as f64) as f32 / scale,
            );
            !self.cursor_over_pet(self.pet_window_size(), local)
        };
        if self.last_passthrough != Some(ignore) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(ignore));
            self.last_passthrough = Some(ignore);
        }
    }

    /// Read the cursor and the mouse buttons and turn them into pet input.
    ///
    /// Using the system state instead of window events keeps the per-pixel
    /// click-through exact and never changes a window style at runtime, which
    /// is what used to make Windows flash the frame.
    fn update_pointer(&mut self, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            return;
        };
        let left_down = crate::platform::primary_button_down().unwrap_or(false);
        let right_down = crate::platform::secondary_button_down().unwrap_or(false);
        let left_pressed = left_down && !self.pointer_left_down;
        let left_released = !left_down && self.pointer_left_down;
        let right_pressed = right_down && !self.pointer_right_down;
        self.pointer_left_down = left_down;
        self.pointer_right_down = right_down;

        if !self.pet_visible {
            self.press_origin = None;
            return;
        }
        let Some((cursor_x, cursor_y)) = crate::platform::global_cursor_position() else {
            return;
        };
        let Ok(position) = window.outer_position() else {
            return;
        };
        let scale = window.scale_factor().max(0.1) as f32;
        let local = egui::pos2(
            (cursor_x - position.x as f64) as f32 / scale,
            (cursor_y - position.y as f64) as f32 / scale,
        );
        let window_size = self.pet_window_size();
        let over_pet = self.cursor_over_pet(window_size, local);
        let now = Instant::now();

        if left_pressed && over_pet {
            self.press_origin = Some(local);
            self.press_started_at = Some(now);
            self.press_moved = false;
        }
        if left_down {
            if let Some(origin) = self.press_origin {
                if (local - origin).length() > CLICK_MOVE_TOLERANCE {
                    self.press_moved = true;
                    self.pet_dragged = true;
                }
            }
        }
        if left_released {
            let clicked = self.press_origin.is_some()
                && !self.press_moved
                && self
                    .press_started_at
                    .is_some_and(|at| at.elapsed() <= CLICK_MAX_HOLD);
            self.press_origin = None;
            self.press_started_at = None;
            self.press_moved = false;
            self.pet_dragged = false;
            self.drag_grab = None;
            if clicked {
                self.register_click();
            }
        }
        if right_pressed && over_pet {
            self.open_menu_at(egui::pos2(cursor_x as f32 / scale, cursor_y as f32 / scale));
        }
    }

    /// A press that neither dragged nor out-lasted a click.
    fn register_click(&mut self) {
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
    /// Sprite that was drawn last, used for the window / tray icon.
    fn current_sprite_index(&self) -> u32 {
        self.last_sprite
    }

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
        self.poll_menu(ctx);
        self.poll_state_events(ctx);
        self.update_pet_timers();
        self.update_auto_walk(ctx, frame);
        self.drag_pet(ctx, frame);
        self.update_glance(frame);
        self.update_passthrough(ctx, frame);
        self.update_pointer(frame);
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
                // Apply the chrome exactly once (and again after a resize):
                // touching these attributes every frame made Windows repaint
                // the non-client frame, which is the border that flashed.
                let chrome_due = self
                    .resize_settled_at
                    .is_some_and(|at| at.elapsed() >= Duration::from_millis(200));
                if !self.window_chrome_ready || chrome_due {
                    window.set_undecorated_shadow(false);
                    window.set_border_color(None);
                    window.set_corner_preference(CornerPreference::DoNotRound);
                    crate::platform::strip_frame_styles(window);
                    crate::platform::clear_dwm_frame(window);
                    crate::platform::enable_transparency(window);
                    // Never activate: an activation repaints the frame state and
                    // would also steal focus from the user's editor.
                    crate::platform::set_no_activate(window);
                    self.window_chrome_ready = true;
                    self.resize_settled_at = None;
                }
            }
        }

        if !self.fonts_installed {
            crate::fonts::install_cjk_font(ui.ctx());
            self.fonts_installed = true;
            ui.ctx().request_repaint();
        }
        // Dropping a pet folder or .zip on the pet itself imports it too.
        self.handle_dropped_files(ui.ctx());
        // Keep the window icon in step with the active pet.
        if let Some(pet_id) = self.pet.as_ref().map(|pet| pet.entry.id.clone()) {
            if self.applied_window_icon.as_deref() != Some(pet_id.as_str()) {
                if let Some(icon) = self.pet_icon() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Icon(Some(icon)));
                    self.applied_window_icon = Some(pet_id);
                }
            }
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
        if self.menu_open {
            self.show_context_menu(ui.ctx());
        }
        self.poll_greeting();

        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

/// Keep a popup inside the monitor it was opened on.
fn clamp_to_monitor(ctx: &egui::Context, anchor: egui::Pos2, size: egui::Vec2) -> egui::Pos2 {
    let monitor = ctx
        .input(|input| input.viewport().monitor_size)
        .unwrap_or(egui::vec2(1280.0, 800.0));
    egui::pos2(
        anchor.x.clamp(0.0, (monitor.x - size.x).max(0.0)),
        anchor.y.clamp(0.0, (monitor.y - size.y).max(0.0)),
    )
}

/// Speech bubble sized to its text and anchored just above the pet, with a
/// tail pointing at it. The pet window reserves room for the largest bubble,
/// so showing one never moves the pet.
fn draw_bubble(painter: &egui::Painter, window: egui::Rect, pet: egui::Rect, text: &str) {
    let galley = painter.layout(
        text.to_owned(),
        egui::FontId::proportional(14.0),
        egui::Color32::WHITE,
        (window.width() - 32.0).max(96.0),
    );
    let padding = egui::vec2(10.0, 8.0);
    let size = galley.size() + padding * 2.0;
    let tail = 7.0;
    let max_x = (window.right() - size.x - 6.0).max(window.left() + 6.0);
    let x = (pet.center().x - size.x * 0.5).clamp(window.left() + 6.0, max_x);
    let y = (pet.top() - tail - 8.0 - size.y).max(window.top() + 6.0);
    let rect = egui::Rect::from_min_size(egui::pos2(x, y), size);
    let fill = egui::Color32::from_rgba_unmultiplied(24, 24, 28, 235);
    painter.rect_filled(rect, egui::CornerRadius::same(10), fill);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(10),
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26),
        ),
        egui::StrokeKind::Inside,
    );
    let tip_x = pet
        .center()
        .x
        .clamp(rect.left() + 14.0, rect.right() - 14.0);
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(tip_x - tail * 0.8, rect.bottom() - 1.0),
            egui::pos2(tip_x + tail * 0.8, rect.bottom() - 1.0),
            egui::pos2(tip_x, rect.bottom() + tail),
        ],
        fill,
        egui::Stroke::NONE,
    ));
    painter.galley(rect.min + padding, galley, egui::Color32::WHITE);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay out a bubble in a realistically sized pet window.
    fn bubble_geometry(text: &str, width: f32, height: f32) -> egui::Rect {
        let ctx = egui::Context::default();
        let mut painted = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            ui.set_clip_rect(window);
            let pet = egui::Rect::from_min_size(
                egui::pos2(width * 0.5 - 96.0, height - 208.0),
                egui::vec2(192.0, 208.0),
            );
            draw_bubble(ui.painter(), window, pet, text);
            painted = window;
        });
        // Nothing uploads the font atlas in a headless pass, so drop it.
        output.textures_delta.clear();
        painted
    }

    #[test]
    fn bubbles_render_for_short_and_long_text() {
        for text in [
            "嗨！",
            "早上好呀，今天也一起加油吧。",
            "这句特别长的问候会被自动换行，并且必须完全待在宠物窗口内部，不允许溢出。",
        ] {
            assert!(bubble_geometry(text, 220.0, 318.0).width() > 0.0);
        }
    }

    #[test]
    fn menu_position_is_clamped_to_the_monitor() {
        let ctx = egui::Context::default();
        // No monitor info yet: the helper falls back to a 1280x800 desktop.
        let position = clamp_to_monitor(&ctx, egui::pos2(5000.0, 5000.0), egui::vec2(176.0, 126.0));
        assert!(position.x <= 1280.0 - 176.0);
        assert!(position.y <= 800.0 - 126.0);
        let position = clamp_to_monitor(&ctx, egui::pos2(-40.0, -40.0), egui::vec2(176.0, 126.0));
        assert_eq!(position, egui::Pos2::ZERO);
    }

    fn test_app(name: &str) -> BytePetApp {
        let paths = AppPaths::resolve(std::env::temp_dir().join(format!("bytepet-test-{name}")));
        let _ = std::fs::remove_dir_all(&paths.config_dir);
        BytePetApp::new(paths, AppConfig::default()).expect("app starts")
    }

    #[test]
    fn clicks_land_on_drawn_pixels_only() {
        let app = test_app("pointer");
        let window = app.pet_window_size();
        let rect = app.pet_rect(window);
        // The sprite is centered horizontally and pinned to the window bottom.
        assert!((rect.center().x - window.x * 0.5).abs() < 0.5);
        assert!((rect.bottom() - window.y).abs() < 0.5);
        // The corner of a cell is transparent, so a click there is not the pet.
        assert!(!app.cursor_over_pet(window, rect.min + egui::vec2(2.0, 2.0)));

        // ...but any drawn pixel is.
        let pet = app.pet.as_ref().expect("bundled pet loads");
        let mut hit = None;
        'search: for y in 0..rect.height() as u32 {
            for x in 0..rect.width() as u32 {
                if pet
                    .atlas
                    .mask
                    .opaque_at_cell(pet.last_sprite, x as f32, y as f32)
                {
                    hit = Some(rect.min + egui::vec2(x as f32, y as f32));
                    break 'search;
                }
            }
        }
        let hit = hit.expect("the bundled pet draws something");
        assert!(app.cursor_over_pet(window, hit));
    }

    #[test]
    fn double_click_replaces_the_single_click() {
        let mut app = test_app("click");
        app.register_click();
        assert!(app.pending_single_click, "first click waits for a second");
        assert!(app.last_click_at.is_some());
        app.register_click();
        assert!(!app.pending_single_click, "the pair is a double click");
        assert!(app.last_click_at.is_none());
    }

    #[test]
    fn stage_scale_rounds_up_to_quarter_steps() {
        let mut app = test_app("stage");
        for (scale, expected) in [
            (0.5, 0.5),
            (0.51, 0.75),
            (1.0, 1.0),
            (1.26, 1.5),
            (2.0, 2.0),
        ] {
            app.config.window.scale = scale;
            assert!(
                (app.stage_scale() - expected).abs() < f32::EPSILON,
                "scale {scale} -> {} (expected {expected})",
                app.stage_scale()
            );
        }
        // The sprite keeps following the raw scale while the stage snaps.
        app.config.window.scale = 1.1;
        assert!((app.pet_size().x - app.pet_cell_size().x * 1.1).abs() < 0.01);
        assert!(app.pet_window_size().x >= app.pet_size().x);
    }
}
