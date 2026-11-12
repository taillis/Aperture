//! The Customize page of the settings app: edit a copy of an Aperture layout (move, resize,
//! hide readings, recolor) or build a free canvas from single readings; background picture
//! or GIF; presets to share.
use super::*;
use aperture::custom::{self, Widget};

/// Layouts whose panels can be edited (their geometry lives in `dashboard/geometry.rs`).
const EDITABLE: &[&str] = &["axis", "ledger", "duet", "orbit", "atelier", "panorama", "cinema"];

#[cfg(windows)]
pub(super) fn pick_file(name: &str, ext: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter(name, ext).pick_file()
}

#[cfg(not(windows))]
pub(super) fn pick_file(_name: &str, _ext: &[&str]) -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn save_file(default: &str, name: &str, ext: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new().set_file_name(default).add_filter(name, ext).save_file()
}

#[cfg(not(windows))]
fn save_file(_default: &str, _name: &str, _ext: &[&str]) -> Option<PathBuf> {
    None
}

/// A readable name for a sensor key.
fn sensor_name(key: &str, vi: bool) -> String {
    let en = match key.trim_start_matches("group_") {
        "cpu" => "CPU panel",
        "gpu" => "GPU panel",
        "mem" => "Memory panel",
        "time" => "Clock panel",
        "weather" => "Weather panel",
        "music" => "Music panel",
        other => return other.replace('_', " ").to_uppercase(),
    };
    if !vi {
        return en.into();
    }
    match en {
        "CPU panel" => "Ô CPU",
        "GPU panel" => "Ô GPU",
        "Memory panel" => "Ô bộ nhớ",
        "Clock panel" => "Ô đồng hồ",
        "Weather panel" => "Ô thời tiết",
        _ => "Ô nhạc",
    }
    .into()
}

impl App {
    /// Starts editing a copy of the layout in use (only the Aperture layouts have panels).
    pub(super) fn edit_current_theme(&mut self) {
        let key = self.get_or("pro_layout", "axis");
        let key = if EDITABLE.contains(&key.as_str()) { key } else { "axis".to_string() };
        let widgets = custom::theme_template(&key);
        if self.get_or("pro_edit_layout", "") != key {
            self.set("pro_edit_widgets", &custom::encode(&widgets));
            self.unset("pro_edit_hidden");
        }
        self.set("pro_edit_layout", &key);
        self.set("pro_layout", &key);
        self.set("pro_custom_enabled", "false");
    }

    pub(super) fn page_canvas(&mut self, ui: &mut egui::Ui) {
        let vi = self.lang == Lang::Vi;
        let free = self.bool_of("pro_custom_enabled", false);
        let editing = !self.get_or("pro_edit_layout", "").is_empty() && !free;
        // ---- mode
        ui.add_space(6.0);
        let mut mode = if free { "free" } else if editing { "edit" } else { "off" }.to_string();
        let opts = [
            ("off", self.t("Off", "Tắt")),
            ("edit", self.t("Edit a layout", "Sửa một bố cục")),
            ("free", self.t("Free canvas", "Canvas tự do")),
        ];
        card(ui, |ui| {
            let lbl = self.t("Mode", "Chế độ").to_string();
            let help = self.t("Edit a copy of an Aperture layout, or place single readings anywhere", "Sửa bản sao của một bố cục Aperture, hoặc đặt từng thông số ở bất kỳ đâu").to_string();
            let mut pick = None;
            row(ui, &lbl, &help, |ui| pick = segmented(ui, &mode, &opts));
            if let Some(v) = pick {
                mode = v;
            }
        });
        match mode.as_str() {
            "off" if free || editing => {
                self.unset("pro_edit_layout");
                self.unset("pro_edit_widgets");
                self.unset("pro_edit_hidden");
                self.unset("pro_edit_text_scale");
                self.set("pro_custom_enabled", "false");
                return;
            }
            "edit" if !editing => {
                self.edit_current_theme();
                return;
            }
            "free" if !free => {
                self.set("pro_custom_enabled", "true");
                if self.get_or("pro_custom_widgets", "").is_empty() {
                    // a useful start instead of an empty page
                    let mut start = Vec::new();
                    for (i, key) in ["clock", "cpu_temp", "gpu_temp", "fps"].iter().enumerate() {
                        let mut w = Widget::new(key);
                        w.x = 24.0 + i as f32 * 472.0;
                        w.y = 24.0;
                        w.w = 448.0;
                        w.h = 200.0;
                        w.size = 96.0;
                        w.bound();
                        start.push(w);
                    }
                    self.set("pro_custom_widgets", &custom::encode(&start));
                }
                return;
            }
            "off" => {
                ui.add_space(8.0);
                ui.label(RichText::new(self.t("Turn on a mode above to start. Your normal layout stays untouched until then.", "Bật một chế độ ở trên để bắt đầu. Bố cục bình thường của bạn không bị thay đổi.")).color(MUTED));
                return;
            }
            _ => {}
        }
        let builtin = editing;
        if builtin {
            // which layout is being edited
            let current = self.get_or("pro_edit_layout", "axis");
            card(ui, |ui| {
                let lbl = self.t("Layout to edit", "Bố cục đang sửa").to_string();
                let help = self.t("Changing it starts again from that layout's original panels", "Đổi bố cục sẽ bắt đầu lại từ các ô gốc của bố cục đó").to_string();
                let mut pick = None;
                row(ui, &lbl, &help, |ui| {
                    let opts: Vec<(&str, &str)> = EDITABLE.iter().map(|k| (*k, LAYOUTS.iter().find(|l| l.key == *k).map_or(*k, |l| l.name))).collect();
                    pick = segmented(ui, &current, &opts);
                });
                if let Some(k) = pick {
                    self.set("pro_layout", &k);
                    self.set("pro_edit_layout", "");
                    self.edit_current_theme();
                }
                divider(ui);
                let lbl = self.t("Back to the original panels", "Trả lại các ô gốc").to_string();
                let mut reset = false;
                row(ui, &lbl, "", |ui| reset = ui.button(self.t("Restore", "Khôi phục")).clicked());
                if reset {
                    let key = self.get_or("pro_edit_layout", "axis");
                    self.set("pro_edit_widgets", &custom::encode(&custom::theme_template(&key)));
                    self.unset("pro_edit_hidden");
                    self.unset("pro_edit_text_scale");
                }
            });
        }

        let storage = if builtin { "pro_edit_widgets" } else { "pro_custom_widgets" };
        let mut widgets = match custom::parse(&self.get(storage).unwrap_or_default()) {
            Ok(w) => w,
            Err(e) => {
                ui.colored_label(BAD, e);
                if ui.button(self.t("Start again", "Làm lại từ đầu")).clicked() {
                    self.unset(storage);
                }
                return;
            }
        };
        let mut changed = false;

        // ---- the canvas: the live preview with the panels on top
        section_title(ui, self.t("Canvas", "Canvas"), self.t("Click a panel to select it, drag to move, drag the corner to resize.", "Bấm vào một ô để chọn, kéo để di chuyển, kéo góc để đổi kích thước."));
        let width = ui.available_width();
        let scale = width / 1920.0;
        let (canvas, _) = ui.allocate_exact_size(Vec2::new(width, 462.0 * scale), Sense::hover());
        ui.painter().rect_filled(canvas, Rounding::same(10.0), Color32::BLACK);
        if let Some(texture) = &self.preview {
            egui::Image::new((texture.id(), canvas.size())).rounding(Rounding::same(10.0)).paint_at(ui, canvas);
        }
        let snap = ui.ctx().data_mut(|d| d.get_temp::<bool>(egui::Id::new("canvas_snap"))).unwrap_or(true);
        if snap {
            for x in (0..1920).step_by(40) {
                ui.painter().line_segment([canvas.min + Vec2::new(x as f32 * scale, 0.0), canvas.min + Vec2::new(x as f32 * scale, 462.0 * scale)], Stroke::new(0.5_f32, Color32::from_white_alpha(14)));
            }
            for y in (0..462).step_by(40) {
                ui.painter().line_segment([canvas.min + Vec2::new(0.0, y as f32 * scale), canvas.min + Vec2::new(1920.0 * scale, y as f32 * scale)], Stroke::new(0.5_f32, Color32::from_white_alpha(14)));
            }
        }
        let mut selected = ui.ctx().data_mut(|d| d.get_temp::<usize>(egui::Id::new("canvas_selected"))).unwrap_or(0).min(widgets.len().saturating_sub(1));
        for (i, w) in widgets.iter_mut().enumerate() {
            let r = Rect::from_min_size(canvas.min + Vec2::new(w.x, w.y) * scale, Vec2::new(w.w, w.h) * scale);
            let handle = Rect::from_min_size(r.right_bottom() - Vec2::splat(16.0), Vec2::splat(16.0));
            let resize = ui.interact(handle, ui.id().with(("resize", i)), Sense::drag());
            if resize.dragged() {
                selected = i;
                let delta = ui.input(|input| input.pointer.delta()) / scale;
                w.w += delta.x;
                w.h += delta.y;
                w.bound();
                changed = true;
            }
            let response = ui.interact(r, ui.id().with(("widget", i)), Sense::click_and_drag());
            if response.clicked() || response.drag_started() {
                selected = i;
            }
            if response.dragged() && !resize.dragged() {
                let delta = ui.input(|input| input.pointer.delta()) / scale;
                w.x += delta.x;
                w.y += delta.y;
                w.bound();
                changed = true;
            }
            if (response.drag_stopped() || resize.drag_stopped()) && snap {
                w.x = (w.x / 8.0).round() * 8.0;
                w.y = (w.y / 8.0).round() * 8.0;
                w.w = (w.w / 8.0).round() * 8.0;
                w.h = (w.h / 8.0).round() * 8.0;
                w.bound();
                changed = true;
            }
            let on = i == selected;
            ui.painter().rect_stroke(r, Rounding::same(6.0), Stroke::new(if on { 2.0_f32 } else { 1.0 }, if on { ACCENT } else { Color32::from_white_alpha(70) }));
            ui.painter().rect_filled(handle.shrink(3.0), Rounding::same(3.0), if on { ACCENT } else { Color32::from_white_alpha(70) });
            if on || response.hovered() {
                let name = sensor_name(&w.sensor, vi);
                let g = ui.painter().layout_no_wrap(name, font(11.5, Weight::Semibold), Color32::WHITE);
                let tag = Rect::from_min_size(r.min + Vec2::new(6.0, 6.0), g.size() + Vec2::new(12.0, 6.0));
                ui.painter().rect_filled(tag, Rounding::same(5.0), if on { ACCENT } else { Color32::from_black_alpha(180) });
                ui.painter().galley(tag.min + Vec2::new(6.0, 3.0), g, Color32::WHITE);
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("canvas_selected"), selected));

        // ---- the selected panel
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, w) in widgets.iter().enumerate() {
                if chip(ui, &sensor_name(&w.sensor, vi), 0, i == selected) {
                    selected = i;
                }
            }
            if !builtin && widgets.len() < 48 && ui.button(self.t("+ Add reading", "+ Thêm thông số")).clicked() {
                let mut w = Widget::new("cpu_temp");
                w.x = 40.0 + (widgets.len() % 5) as f32 * 320.0;
                w.y = 40.0 + (widgets.len() / 5 % 2) as f32 * 180.0;
                w.bound();
                widgets.push(w);
                selected = widgets.len() - 1;
                changed = true;
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("canvas_selected"), selected));
        let mut delete = false;
        let mut duplicate = false;
        if let Some(w) = widgets.get_mut(selected) {
            section_title(ui, &sensor_name(&w.sensor, vi), "");
            card(ui, |ui| {
                if !builtin {
                    let lbl = self.t("Reading", "Thông số").to_string();
                    row(ui, &lbl, "", |ui| {
                        egui::ComboBox::from_id_salt("canvas_sensor").selected_text(sensor_name(&w.sensor, vi)).width(240.0).height(400.0).show_ui(ui, |ui| {
                            for &key in custom::SENSORS {
                                changed |= ui.selectable_value(&mut w.sensor, key.into(), sensor_name(key, vi)).changed();
                            }
                        });
                    });
                    divider(ui);
                    let lbl = self.t("Text size", "Cỡ chữ").to_string();
                    row(ui, &lbl, "", |ui| changed |= ui.add(egui::Slider::new(&mut w.size, 12.0..=160.0).suffix(" px")).changed());
                    divider(ui);
                }
                let lbl = self.t("Position and size", "Vị trí và kích thước").to_string();
                row(ui, &lbl, "X · Y · W · H", |ui| {
                    for v in [&mut w.h, &mut w.w, &mut w.y, &mut w.x] {
                        changed |= ui.add(egui::DragValue::new(v).speed(1.0).range(0.0..=1920.0)).changed();
                    }
                });
                divider(ui);
                let lbl = self.t("Text color", "Màu chữ").to_string();
                let panel_lbl = self.t("Panel background", "Nền ô").to_string();
                row(ui, &lbl, "", |ui| {
                    changed |= ui.checkbox(&mut w.panel, panel_lbl.as_str()).changed();
                    ui.add_space(12.0);
                    changed |= ui.color_edit_button_srgb(&mut w.color).changed();
                });
                if !builtin {
                    divider(ui);
                    ui.horizontal(|ui| {
                        duplicate = ui.button(self.t("Duplicate", "Nhân bản")).clicked();
                        delete = ui.button(RichText::new(self.t("Delete", "Xóa")).color(BAD)).clicked();
                    });
                }
            });
            if builtin && ["group_cpu", "group_gpu", "group_mem"].contains(&w.sensor.as_str()) {
                let group = w.sensor.trim_start_matches("group_").to_string();
                let names: &[&str] = match group.as_str() {
                    "cpu" => &["load", "power", "clock", "fan", "uptime", "network"],
                    "gpu" => &["load", "power", "fan", "clock", "vram", "fps"],
                    _ => &["used", "free", "ssd", "disk", "read", "write"],
                };
                let mut hidden = self.get_or("pro_edit_hidden", "").split(',').filter(|v| !v.is_empty()).map(str::to_string).collect::<Vec<_>>();
                let mut edited = false;
                card(ui, |ui| {
                    ui.label(RichText::new(self.t("Readings to show", "Thông số hiển thị")).font(font(14.5, Weight::Medium)));
                    ui.horizontal_wrapped(|ui| {
                        for &name in names {
                            let key = format!("{group}:{name}");
                            let mut show = !hidden.contains(&key);
                            if ui.checkbox(&mut show, name.to_uppercase()).changed() {
                                hidden.retain(|v| v != &key);
                                if !show {
                                    hidden.push(key);
                                }
                                edited = true;
                            }
                        }
                    });
                });
                if edited {
                    self.set("pro_edit_hidden", &hidden.join(","));
                }
                // a hardware panel needs room for its six readings
                if let Some(original) = custom::theme_template(&self.get_or("pro_edit_layout", "axis")).iter().find(|p| p.sensor == w.sensor) {
                    if w.w < original.w * 0.6 || w.h < original.h * 0.6 {
                        w.w = w.w.max(original.w * 0.6);
                        w.h = w.h.max(original.h * 0.6);
                        w.bound();
                        changed = true;
                    }
                }
            }
        }
        if duplicate && widgets.len() < 48 {
            let mut w = widgets[selected].clone();
            w.x += 24.0;
            w.y += 24.0;
            w.bound();
            widgets.push(w);
            changed = true;
        }
        if delete && selected < widgets.len() {
            widgets.remove(selected);
            changed = true;
        }
        if changed {
            self.set(storage, &custom::encode(&widgets));
        }

        // ---- canvas-wide options
        section_title(ui, self.t("Canvas options", "Tùy chọn canvas"), "");
        card(ui, |ui| {
            let mut snap_value = snap;
            let lbl = self.t("Snap to an 8 px grid", "Bám lưới 8 px").to_string();
            row(ui, &lbl, "", |ui| {
                if toggle(ui, &mut snap_value).changed() {
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("canvas_snap"), snap_value));
                }
            });
            if builtin {
                divider(ui);
                let mut size = self.get("pro_edit_text_scale").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
                let lbl = self.t("Reading text size", "Cỡ chữ thông số").to_string();
                let mut ch = false;
                row(ui, &lbl, "", |ui| ch = ui.add(egui::Slider::new(&mut size, 0.85..=1.15).custom_formatter(|n, _| format!("{:.0} %", n * 100.0))).changed());
                if ch {
                    self.set("pro_edit_text_scale", &format!("{size:.2}"));
                }
                divider(ui);
                let lbl = self.t("Logo instead of the music panel", "Logo thay cho ô nhạc").to_string();
                let has_logo = !self.get_or("pro_logo", "").is_empty();
                let mut choose = false;
                let mut clear = false;
                row(ui, &lbl, &self.get_or("pro_logo", ""), |ui| {
                    if has_logo {
                        clear = ui.button(self.t("Show music again", "Hiện lại nhạc")).clicked();
                    }
                    choose = ui.button(self.t("Choose image…", "Chọn ảnh…")).clicked();
                });
                if clear {
                    self.unset("pro_logo");
                }
                if choose {
                    match pick_file("Image", &["png", "jpg", "jpeg", "bmp"]).and_then(|p| p.to_str().map(str::to_string)) {
                        Some(p) if !p.contains(['#', '\n', '\r']) => self.set("pro_logo", &p),
                        Some(_) => self.save_error = Some(self.t("Choose an image whose path has no # sign.", "Hãy chọn ảnh có đường dẫn không chứa dấu #.").into()),
                        None => {}
                    }
                }
            }
            divider(ui);
            let lbl = self.t("Background picture or GIF", "Ảnh nền hoặc GIF").to_string();
            let help = self.t("GIFs play on the display; the preview shows one frame", "GIF chạy trên màn hình; bản xem trước chỉ hiện một khung").to_string();
            let has_bg = !self.get_or("background", "").is_empty();
            let mut choose = false;
            let mut clear = false;
            row(ui, &lbl, &help, |ui| {
                if has_bg {
                    clear = ui.button(self.t("Remove", "Bỏ ảnh nền")).clicked();
                }
                choose = ui.button(self.t("Choose…", "Chọn…")).clicked();
            });
            if clear {
                self.unset("background");
            }
            if choose {
                if let Some(path) = pick_file("Image / GIF", &["png", "jpg", "jpeg", "bmp", "gif"]) {
                    let result = (|| -> std::io::Result<String> {
                        let dir = self.dir.join("assets").join("backgrounds");
                        std::fs::create_dir_all(&dir)?;
                        let suffix = path.extension().and_then(|s| s.to_str()).unwrap_or("png").to_ascii_lowercase();
                        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
                        let name = format!("background-{stamp}.{suffix}");
                        std::fs::copy(&path, dir.join(&name))?;
                        Ok(format!("assets/backgrounds/{name}"))
                    })();
                    match result {
                        Ok(p) => {
                            self.set("background", &p);
                            self.set("background_effect", "none");
                        }
                        Err(e) => self.save_error = Some(e.to_string()),
                    }
                }
            }
            if has_bg {
                divider(ui);
                let mut dim = self.get("background_dim").and_then(|v| v.trim().parse::<u32>().ok()).unwrap_or(30).min(100);
                let lbl = self.t("Darken the background", "Làm tối ảnh nền").to_string();
                let mut ch = false;
                row(ui, &lbl, "", |ui| ch = ui.add(egui::Slider::new(&mut dim, 0..=100).suffix(" %")).changed());
                if ch {
                    self.set("background_dim", &dim.to_string());
                }
            }
        });

        // ---- presets
        section_title(ui, self.t("Presets", "Mẫu đã lưu"), self.t("Share your canvas. A preset never touches sensors, weather or orientation.", "Chia sẻ canvas của bạn. Mẫu không đụng tới cảm biến, thời tiết hay chiều màn hình."));
        card(ui, |ui| {
            const KEYS: [&str; 10] = ["pro_edit_layout", "pro_edit_widgets", "pro_edit_hidden", "pro_edit_text_scale", "pro_logo", "pro_custom_enabled", "pro_custom_widgets", "background", "background_dim", "bg_fit"];
            let mut export = false;
            let mut import = false;
            let lbl = self.t("Export or import", "Xuất hoặc nhập").to_string();
            row(ui, &lbl, "", |ui| {
                import = ui.button(self.t("Import…", "Nhập…")).clicked();
                export = ui.button(self.t("Export…", "Xuất…")).clicked();
            });
            if export {
                if let Some(path) = save_file("my-aperture.conf", "Aperture preset", &["conf"]) {
                    let text = KEYS.iter().filter_map(|k| self.get(k).map(|v| format!("{k} = {v}\n"))).collect::<String>();
                    if let Err(e) = std::fs::write(path, text) {
                        self.save_error = Some(e.to_string());
                    }
                }
            }
            if import {
                if let Some(path) = pick_file("Aperture preset", &["conf"]) {
                    let result = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|text| {
                        let doc = ConfDoc::parse(&text);
                        custom::parse(&doc.get("pro_custom_widgets").unwrap_or_default())?;
                        custom::parse(&doc.get("pro_edit_widgets").unwrap_or_default())?;
                        if let Some(key) = doc.get("pro_edit_layout").filter(|v| !v.is_empty()) {
                            if custom::theme_template(&key).is_empty() {
                                return Err("This preset uses an unsupported layout".into());
                            }
                        }
                        Ok(doc)
                    });
                    match result {
                        Ok(doc) => {
                            for key in KEYS {
                                if key == "background" {
                                    continue; // the picture itself is not in the preset
                                }
                                match doc.get(key) {
                                    Some(v) => self.set(key, &v),
                                    None => self.unset(key),
                                }
                            }
                            if let Some(key) = doc.get("pro_edit_layout").filter(|v| !v.is_empty()) {
                                self.set("pro_layout", &key);
                            }
                        }
                        Err(e) => self.save_error = Some(e),
                    }
                }
            }
        });
    }
}
