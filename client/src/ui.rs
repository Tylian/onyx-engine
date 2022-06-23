use std::collections::HashMap;

use egui::*;
use glam::ivec2;
use mint::Point2;
use onyx_common::{
    network::{AreaData, MapId, MapLayer, MapSettings, TileAnimation},
    SPRITE_SIZE, TILE_SIZE,
};
use strum::IntoEnumIterator;

use crate::{assets::Assets, map::Tile, utils::ping_pong};

pub fn area_radio(ui: &mut Ui, selected: bool, title: &str, description: &str) -> Response {
    ui.radio(selected, title).on_hover_ui(|ui| {
        ui.heading(title);
        ui.label(description);
    })
}

// TODO multiple tile selections
pub fn tile_selector(ui: &mut Ui, texture: &TextureHandle, selected: &mut Pos2, snap: Vec2) {
    ScrollArea::both().show_viewport(ui, |ui, viewport| {
        let clip_rect = ui.clip_rect();

        let margin = ui.visuals().clip_rect_margin;
        let offset = (clip_rect.left_top() - viewport.left_top()) + vec2(margin, margin);
        let texture_size = texture.size_vec2();

        let response = ui.add(Image::new(texture, texture_size).sense(Sense::click()));
        if response.clicked() {
            let pointer = response.interact_pointer_pos().unwrap();
            let position = pointer - offset;
            if position.x >= 0.0 && position.y >= 0.0 && position.x < texture_size.x && position.y < texture_size.y {
                *selected = (snap * (position.to_vec2() / snap).floor()).to_pos2();
            }
        }

        let painter = ui.painter();
        let rect = Rect::from_min_size(*selected + offset, snap);
        painter.rect_stroke(rect, 0., ui.visuals().window_stroke());

        response
    });
}

pub fn sprite_preview(ui: &mut Ui, texture: &TextureHandle, time: f64, sprite: u32) -> Response {
    let sprite_x = (sprite as f64 % 4.0) * 3.0;
    let sprite_y = (sprite as f64 / 4.0).floor() * 4.0;

    // walk left and right
    let speed = 2.5; // tiles per second
    let loops = 8.0; // how many tiles to walk before rotating

    let animation_speed = 2.0 / speed; // time to complete 1 walk cycle

    let offset_x = ping_pong(time / animation_speed % 1.0, 3) as f64;
    let offset_y = ((time / (animation_speed * loops)) % 4.0).floor();

    let p = vec2(
        (sprite_x + offset_x) as f32 * SPRITE_SIZE as f32,
        (sprite_y + offset_y) as f32 * SPRITE_SIZE as f32,
    ) / texture.size_vec2();
    let size = vec2(SPRITE_SIZE as f32, SPRITE_SIZE as f32) / texture.size_vec2();
    let sprite =
        Image::new(texture, (SPRITE_SIZE as f32, SPRITE_SIZE as f32)).uv(Rect::from_min_size(p.to_pos2(), size));

    ui.add(sprite)
}

fn map_selector(ui: &mut Ui, id: &str, value: &mut Option<MapId>, maps: &HashMap<MapId, String>) {
    let selected_label = if let Some(id) = value {
        if let Some(map) = maps.get(id) {
            format!("{}. {}", id.0, map)
        } else {
            format!("{}.", id.0)
        }
    } else {
        String::from("Disabled")
    };

    egui::ComboBox::from_id_source(id)
        .selected_text(selected_label)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, "Disabled");
            ui.separator();

            let mut items = maps.iter().collect::<Vec<_>>();
            items.sort_unstable_by(|(a, _), (b, _)| a.0.cmp(&b.0));

            for (id, name) in items {
                ui.selectable_value(value, Some(*id), format!("{}. {}", id.0, name));
            }
        });
}

#[derive(Clone, Copy, PartialEq)]
pub enum MapEditorTab {
    Tileset,
    Areas,
    Settings,
    Tools,
}

#[derive(Clone, PartialEq)]
pub enum MapEditorWants {
    Nothing,
    Save,
    Close,
    /// Map editor wishes to be closed and to teleport the user to the id supplied
    Warp(MapId),
    /// Map editor wishes to resize the map
    ResizeMap(u32, u32),
}

#[derive(Clone, PartialEq)]
pub struct MapEditorResponse {
    tab: MapEditorTab,
    wants: MapEditorWants,
}

impl MapEditorResponse {
    pub fn wants(&self) -> &MapEditorWants {
        &self.wants
    }
}

pub struct MapEditor {
    tab: MapEditorTab,

    // map editor
    layer: MapLayer,
    tile_picker: Pos2,
    is_autotile: bool,
    is_tile_animated: bool,
    tile_animation: TileAnimation,

    // areas
    area_data: AreaData,

    // settings
    settings: MapSettings,
    id: MapId,
    increment_revision: bool,

    // tools
    maps: HashMap<MapId, String>,
    new_width: u32,
    new_height: u32,
    selected_id: MapId,
}

fn auto_complete<T: AsRef<str>>(ui: &mut Ui, popup_id: Id, suggestions: &[T], current: &mut String) {
    let filtered = suggestions
        .iter()
        .filter(|item| item.as_ref().contains(&*current))
        .collect::<Vec<_>>();

    let text_edit = ui.text_edit_singleline(current);
    if text_edit.gained_focus() {
        ui.memory().open_popup(popup_id);
    }

    popup_below_widget(ui, popup_id, &text_edit, |ui| {
        ScrollArea::vertical()
            .max_height(ui.spacing().combo_height)
            .show(ui, |ui| {
                for item in filtered {
                    let item = item.as_ref();
                    if ui.selectable_label(current == item, item).clicked() {
                        *current = String::from(item);
                    }
                }
            })
            .inner
    });

    // crappy attempt at fixing a bug lmao
    if text_edit.lost_focus() {
        ui.memory().close_popup();
    }
}

fn option_combo<H, T, F>(ui: &mut Ui, id: H, selected: &mut Option<T>, render: F, list: &[T])
where
    H: std::hash::Hash,
    T: PartialEq + Clone,
    F: Fn(Option<&T>) -> String
{
    egui::ComboBox::from_id_source(id)
        .selected_text(render(selected.as_ref()))
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, render(None));
            ui.separator();

            for item in list.iter() {
                if ui.selectable_label(selected.as_ref() == Some(item), render(Some(item))).clicked() {
                    *selected = Some(item.clone())
                }
            }
        });
}

impl MapEditor {
    pub fn new() -> Self {
        Self {
            tab: MapEditorTab::Tileset,

            // map editor
            layer: MapLayer::Ground,
            tile_picker: pos2(0.0, 0.0),
            is_autotile: false,
            is_tile_animated: false,
            tile_animation: TileAnimation {
                frames: 2,
                duration: 1.0,
                bouncy: false,
            },

            // area
            area_data: AreaData::Blocked,

            // settings
            id: MapId::start(),
            settings: MapSettings::default(),
            increment_revision: true,

            // tools
            maps: HashMap::new(),
            new_width: 0,
            new_height: 0,

            selected_id: MapId::start(),
        }
    }

    pub fn show(&mut self, ui: &mut Ui, assets: &Assets) -> MapEditorResponse {
        let mut wants = MapEditorWants::Nothing;

        menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save").clicked() {
                    wants = MapEditorWants::Save;
                    if self.increment_revision {
                        self.settings.revision += 1;
                    }
                    ui.close_menu();
                }
                if ui.button("Exit").clicked() {
                    wants = MapEditorWants::Close;
                    ui.close_menu();
                }
            });
            ui.add_space(6.0);
            ui.separator();

            ui.selectable_value(&mut self.tab, MapEditorTab::Tileset, "Tileset");
            ui.selectable_value(&mut self.tab, MapEditorTab::Areas, "Areas");
            ui.selectable_value(&mut self.tab, MapEditorTab::Settings, "Settings");
            ui.selectable_value(&mut self.tab, MapEditorTab::Tools, "Tools");
        });

        ui.separator();

        let tab_wants = match self.tab {
            MapEditorTab::Tileset => self.show_tileset_tab(ui, assets),
            MapEditorTab::Areas => self.show_area_tab(ui),
            MapEditorTab::Settings => self.show_settings_tab(ui, assets),
            MapEditorTab::Tools => self.show_tools_tab(ui),
        };

        if tab_wants != MapEditorWants::Nothing {
            wants = tab_wants;
        }

        MapEditorResponse { tab: self.tab, wants }
    }

    fn show_tileset_tab(&mut self, ui: &mut Ui, assets: &Assets) -> MapEditorWants {
        let id = ui.make_persistent_id("mapeditor_settings");
        collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Layer: ");
                    egui::ComboBox::from_id_source("layer")
                        .selected_text(self.layer.to_string())
                        .show_ui(ui, |ui| {
                            for layer in MapLayer::iter() {
                                if layer == MapLayer::Fringe {
                                    ui.separator();
                                }
                                ui.selectable_value(&mut self.layer, layer, layer.to_string());
                            }
                        });
                    ui.weak("(Press the arrow for more options)");
                });
            })
            .body(|ui| {
                ui.checkbox(&mut self.is_autotile, "Autotile");
                ui.checkbox(&mut self.is_tile_animated, "Animated");
                ui.add_enabled_ui(self.is_tile_animated, |ui| {
                    Grid::new("animation settings").num_columns(2).show(ui, |ui| {
                        ui.label("Duration:");
                        ui.add(
                            DragValue::new(&mut self.tile_animation.duration)
                                .speed(0.01f64)
                                .clamp_range(0f64..=f64::MAX)
                                .suffix("s"),
                        );
                        ui.end_row();

                        ui.label("Frames:");
                        ui.add(
                            DragValue::new(&mut self.tile_animation.frames)
                                .speed(0.1f64)
                                .clamp_range(0f64..=f64::MAX),
                        );
                        ui.end_row();
                    });
                    ui.checkbox(&mut self.tile_animation.bouncy, "Bouncy animation (e.g 1-2-3-2)");
                });
            });

        ui.add_space(3.0);
        tile_selector(
            ui,
            &assets.tileset().egui,
            &mut self.tile_picker,
            Vec2::new(TILE_SIZE as f32, TILE_SIZE as f32),
        );

        MapEditorWants::Nothing
    }

    fn show_area_tab(&mut self, ui: &mut Ui) -> MapEditorWants {
        ui.horizontal(|ui| {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Area type");
                    let response = area_radio(
                        ui,
                        matches!(self.area_data, AreaData::Blocked),
                        "Blocked",
                        "Entities are blocked from entering this area.",
                    );
                    if response.clicked() {
                        self.area_data = AreaData::Blocked;
                    }

                    let response = area_radio(
                        ui,
                        matches!(self.area_data, AreaData::Warp(_, _, _)),
                        "Warp",
                        "Teleports a player somewhere else",
                    );
                    if response.clicked() {
                        self.area_data = AreaData::Warp(
                            MapId(0),
                            Point2 { x: 0.0, y: 0.0 },
                            onyx_common::network::Direction::South
                        );
                    }
                });
            });

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Area data");
                    Grid::new("area data")
                        .num_columns(2)
                        .show(ui, |ui| match &mut self.area_data {
                            AreaData::Blocked => {
                                ui.label("Blocked has no values");
                            }
                            AreaData::Warp(map_id, position, direction) => {
                                ui.label("Map:");
                                egui::ComboBox::from_id_source("warp map")
                                    .selected_text(format!("{}. {}", map_id.0, self.maps.get(map_id).unwrap()))
                                    .show_ui(ui, |ui| {
                                        let mut items = self.maps.iter().collect::<Vec<_>>();
                                        items.sort_unstable_by(|(a, _), (b, _)| a.0.cmp(&b.0));
                            
                                        for (id, name) in items {
                                            ui.selectable_value(map_id, *id, format!("{}. {}", id.0, name));
                                        }
                                    });
                                ui.end_row();

                                ui.label("Position:");
                                ui.horizontal(|ui| {
                                    ui.label("X: ");
                                    ui.add(DragValue::new(&mut position.x).clamp_range(0.0..=f32::INFINITY));
                                    ui.label("Y: ");
                                    ui.add(DragValue::new(&mut position.y).clamp_range(0.0..=f32::INFINITY));
                                });
                                ui.end_row();

                                ui.label("Direction:");
                                egui::ComboBox::from_id_source("warp direction")
                                    .selected_text(direction.to_string())
                                    .show_ui(ui, |ui| {
                                        use onyx_common::network::Direction;
                                        ui.selectable_value(direction, Direction::North, "North");
                                        ui.selectable_value(direction, Direction::East, "East");
                                        ui.selectable_value(direction, Direction::South, "South");
                                        ui.selectable_value(direction, Direction::West, "West");
                                    });
                            }
                        });
                });
            });
        });

        MapEditorWants::Nothing
    }

    pub fn show_settings_tab(&mut self, ui: &mut Ui, assets: &Assets) -> MapEditorWants {
        let mut wants = MapEditorWants::Nothing;
        let shift = ui.ctx().input().modifiers.shift;

        ui.heading("Map properties");
        Grid::new("properties").num_columns(2).show(ui, |ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.settings.name);
            ui.end_row();

            ui.label("Internal id:");
            ui.label(self.id.0.to_string());
            ui.end_row();

            ui.label("Tileset:");
            egui::ComboBox::from_id_source("tileset")
                .selected_text(&self.settings.tileset)
                .show_ui(ui, |ui| {
                    for tileset in assets.tilesets() {
                        if ui.selectable_label(self.settings.tileset == tileset, tileset).clicked() {
                            self.settings.tileset = tileset.to_owned();
                            assets.set_tileset(tileset).unwrap();
                            self.tile_picker = Pos2::ZERO;
                            ui.close_menu();
                        }
                    }
                });
            ui.end_row();

            ui.label("Music:");
            egui::ComboBox::from_id_source("music")
                .selected_text(if let Some(music) = &self.settings.music { music } else { "None" })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.settings.music.is_none(), "None").clicked() {
                        self.settings.music = None;
                        assets.stop_music();
                    }
                    ui.separator();

                    for item in assets.get_music() {
                        if ui.selectable_label(self.settings.music.as_ref() == Some(&item), &item).clicked() {
                            self.settings.music = Some(item.clone());
                            assets.play_music(&item);
                        }
                    }
                });
            ui.end_row();

            ui.label("Revision:");
            ui.add_enabled(
                shift,
                DragValue::new(&mut self.settings.revision)
                    .clamp_range(0..=u32::MAX)
                    .speed(0.1),
            )
            .on_disabled_hover_ui(|ui| {
                ui.colored_label(Color32::RED, "Manually changing this value may break a lot of things");
                ui.label("Hold shift to enable editing");
            });
            ui.end_row();
        });
        ui.checkbox(&mut self.increment_revision, "Increment revision on save");

        ui.add_space(6.0);

        ui.heading("Edge warps");
        Grid::new("warps").num_columns(3).show(ui, |ui| {
            ui.label("North:");
            map_selector(ui, "north", &mut self.settings.warps.north, &self.maps);
            ui.end_row();

            ui.label("East:");
            map_selector(ui, "east", &mut self.settings.warps.east, &self.maps);
            ui.end_row();

            ui.label("South:");
            map_selector(ui, "south", &mut self.settings.warps.south, &self.maps);
            ui.end_row();

            ui.label("West:");
            map_selector(ui, "west", &mut self.settings.warps.west, &self.maps);
            ui.end_row();
        });

        ui.add_space(3.0);

        wants
    }

    pub fn show_tools_tab(&mut self, ui: &mut Ui) -> MapEditorWants {
        let shift = ui.ctx().input().modifiers.shift;
        let mut wants = MapEditorWants::Nothing;

        ui.heading("Teleport");
        ui.label("Select a map and hit ▶, the map editor will close and you will be teleported to it.");
        ui.label("The list contains maps that haven't been created yet, as well as the option to create a new map at the bottom.");
        ui.horizontal(|ui| {
            fn label_text(id: MapId, name: Option<impl AsRef<str>>) -> impl Into<WidgetText> {
                if let Some(name) = name {
                    RichText::new(format!("{}. {}", id.0, name.as_ref()))
                } else {
                    RichText::new(format!("{}. new map", id.0)).italics()
                }
            }
            let selected_text = label_text(self.selected_id, self.maps.get(&self.selected_id));
            egui::ComboBox::from_id_source("map selecter")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    ui.set_min_width(200.0);
                    let max_id = self.maps.keys().fold(0, |acc, k| k.0.max(acc)) + 1;
                    for id in 0..=max_id {
                        let key = MapId(id);
                        let label = label_text(key, self.maps.get(&key));
                        if ui.selectable_label(self.selected_id == key, label).clicked() {
                            self.selected_id = key;
                        }
                    }
                }
            );

            if ui.button("▶").clicked() {
                wants = MapEditorWants::Warp(self.selected_id);
            }
        });

        ui.add_space(6.0);

        ui.heading("Map size");
        Grid::new("resize").num_columns(2).show(ui, |ui| {
            ui.label("Width:");
            ui.add(
                DragValue::new(&mut self.new_width)
                    .clamp_range(0..=u32::MAX)
                    .speed(0.05)
                    .suffix(" tiles"),
            );
            ui.end_row();

            ui.label("Height:");
            ui.add(
                DragValue::new(&mut self.new_height)
                    .clamp_range(0..=u32::MAX)
                    .speed(0.05)
                    .suffix(" tiles"),
            );
            ui.end_row();

            ui.add_enabled_ui(shift, |ui| {
                let button = ui.button("Save").on_disabled_hover_ui(|ui| {
                    ui.colored_label(
                        Color32::RED,
                        "This will destroy tiles outside of the map and isn't reversable.",
                    );
                    ui.label("Hold shift to enable the save button.");
                });
                if button.clicked() {
                    wants = MapEditorWants::ResizeMap(self.new_width, self.new_height);
                }
            });
        });

        ui.add_space(6.0);
        wants
    }

    pub fn tab(&self) -> MapEditorTab {
        self.tab
    }

    pub fn layer(&self) -> MapLayer {
        self.layer
    }

    pub fn set_maps(&mut self, maps: HashMap<MapId, String>) {
        self.maps = maps;
    }

    pub fn set_map_size(&mut self, width: u32, height: u32) {
        self.new_width = width;
        self.new_height = height;
    }

    pub fn set_settings(&mut self, id: MapId, settings: MapSettings) {
        self.id = id;
        self.selected_id = id;
        self.settings = settings;
    }

    pub fn tile(&self) -> Tile {
        Tile {
            texture: ivec2(
                self.tile_picker.x as i32 / TILE_SIZE,
                self.tile_picker.y as i32 / TILE_SIZE,
            ),
            autotile: self.is_autotile,
            animation: if self.is_tile_animated {
                Some(self.tile_animation)
            } else {
                None
            },
        }
    }

    pub fn area_data(&self) -> &AreaData {
        &self.area_data
    }

    pub fn map_settings(&self) -> (MapId, &MapSettings) {
        (self.id, &self.settings)
    }
}

fn option_textedit(ui: &mut Ui, value: &mut Option<String>) -> InnerResponse<()> {
    ui.horizontal(|ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, "").changed() && enabled != value.is_some() {
            if enabled {
                *value = Some(String::new());
            } else {
                *value = None;
            }
        }

        ui.add_enabled_ui(enabled, |ui| match value.as_mut() {
            Some(text) => ui.text_edit_singleline(text),
            None => ui.label("disabled"),
        });
    })
}
