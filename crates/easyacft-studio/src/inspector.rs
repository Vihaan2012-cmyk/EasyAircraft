//! The part inspector: live values, why, settings with typed editors, bindings.

use crate::{edit, engine_view, unit, why, App, SETTABLE};
use easyacft_lang::model::Field;
use easyacft_lang::{Kind, Type};
use easyacft_parts::catalog;
use eframe::egui;
use std::path::PathBuf;

fn shown(f: &Field) -> String {
    match f {
        Field::Str(s) => s.clone(),
        Field::Num(n) => format!("{n}"),
        Field::Bool(b) => format!("{b}"),
        Field::List(l) => l.join(", "),
    }
}

fn matches_kind(kinds: &[&str], domain: &str, kind: &str) -> bool {
    kinds.iter().any(|k| match k.split_once('.') {
        Some((d, "*")) => d == domain,
        Some((d, x)) => d == domain && x == kind,
        None => *k == "*" || *k == domain,
    })
}

/// The name being typed at the end of `text`, for completion.
fn last_token(text: &str) -> &str {
    let start = text.rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).map_or(0, |i| i + 1);
    &text[start..]
}

impl App {
    pub fn inspector(&mut self, ui: &mut egui::Ui) {
        let Some(sel) = self.selected else {
            ui.label("Select a part on the diagram or in the list.");
            return;
        };
        let Some(a) = &self.aircraft else { return };
        let Some(part) = a.checked.parts.get(sel).cloned() else { return };
        let spec = catalog::find(part.domain, part.kind);
        ui.heading(&part.id);
        ui.label(format!("{}.{}", part.domain, part.kind));
        if let Some(s) = spec {
            ui.small(s.help);
        }
        let failed = self.get(&format!("{}.failed", part.id)).is_some_and(|v| v != 0.0);
        if self.get(&format!("{}.failed", part.id)).is_some() && ui.button(if failed { "Repair" } else { "Fail" }).clicked() {
            self.set(&format!("{}.failed", part.id), if failed { 0.0 } else { 1.0 });
        }
        ui.separator();
        let outs: Vec<(String, Type)> = spec.map(|s| s.outputs.iter().map(|(n, t)| (n.to_string(), *t)).collect()).unwrap_or_default();
        let mut sets = Vec::new();
        let mut plot = None;
        egui::Grid::new("outs").num_columns(3).show(ui, |ui| {
            for (n, t) in &outs {
                let full = format!("{}.{n}", part.id);
                let v = self.get(&full).unwrap_or(0.0);
                ui.label(n);
                if SETTABLE.contains(&n.as_str()) {
                    if *t == Type::Bool {
                        let mut b = v != 0.0;
                        if ui.checkbox(&mut b, unit(*t, v)).changed() {
                            sets.push((full.clone(), b as u8 as f64));
                        }
                    } else {
                        let mut x = v;
                        if ui.add(egui::DragValue::new(&mut x).speed(0.1)).changed() {
                            sets.push((full.clone(), x));
                        }
                    }
                } else {
                    ui.label(unit(*t, v));
                }
                if ui.small_button("plot").clicked() {
                    plot = Some(full);
                }
                ui.end_row();
            }
        });
        for (n, v) in sets {
            self.set(&n, v);
        }
        if let Some(full) = plot {
            if !self.plots.iter().any(|p| p.0 == full) {
                self.plots.push((full, Vec::new()));
            }
        }
        let mut jump = None;
        if part.domain == "electrical" && part.kind == "bus" {
            let a = self.aircraft.as_ref().unwrap();
            let mut rows: Vec<(usize, String)> = Vec::new();
            for (k, q) in a.checked.parts.iter().enumerate() {
                if q.domain != "electrical" {
                    continue;
                }
                let between = q.parts("between");
                if matches!(q.kind, "breaker" | "sspc") && between.first() == Some(&sel) {
                    let other = between.iter().copied().find(|&b| b != sel);
                    // A second feed reaches its load bus through a diode.
                    let through: Vec<usize> = other.into_iter().chain(a.checked.parts.iter().filter(|d| d.kind == "diode" && d.parts("between").first().copied() == other).filter_map(|d| d.parts("between").get(1).copied())).collect();
                    let users: Vec<String> = a.checked.parts.iter().filter(|u| u.kind == "load" && u.part("bus").is_some_and(|b| through.contains(&b))).map(|u| {
                        let w = self.get(&format!("{}.power", u.id)).unwrap_or(0.0);
                        format!("{} {}", u.id, unit(Type::Num(easyacft_core::Dim::POWER), w))
                    }).collect();
                    let st = if self.get(&format!("{}.tripped", q.id)) == Some(1.0) { "TRIPPED" } else if self.get(&format!("{}.closed", q.id)) == Some(1.0) { "in" } else { "out" };
                    let amps = self.get(&format!("{}.current", q.id)).unwrap_or(0.0);
                    let rating = format!("  {:.0} A", q.num("rating"));
                    rows.push((k, format!("{}{rating}  {st}  {:.1} A  {}", q.id, amps.abs(), users.join(", "))));
                } else if q.kind == "load" && q.part("bus") == Some(sel) {
                    let w = self.get(&format!("{}.power", q.id)).unwrap_or(0.0);
                    rows.push((k, format!("{}  {}", q.id, unit(Type::Num(easyacft_core::Dim::POWER), w))));
                }
            }
            if !rows.is_empty() {
                ui.separator();
                egui::CollapsingHeader::new(format!("Breakers and users ({})", rows.len())).default_open(true).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(320.0).id_salt("feeders").show(ui, |ui| {
                        for (k, t) in rows {
                            if ui.link(t).clicked() {
                                jump = Some(k);
                            }
                        }
                    });
                });
            }
        }
        {
            let a = self.aircraft.as_ref().unwrap();
            let get = |n: &str| self.get(n);
            let fmt = |n: &str, v: f64| {
                let t = a.checked.slots.index.get(n).map(|&i| a.checked.slots.names[i].1).unwrap_or(Type::Num(easyacft_core::Dim::NONE));
                unit(t, v)
            };
            let lines = why::explain(&a.checked, sel, &get, &fmt);
            if !lines.is_empty() {
                ui.separator();
                egui::CollapsingHeader::new("Why").default_open(true).show(ui, |ui| {
                    for l in lines {
                        ui.horizontal(|ui| {
                            ui.add_space(l.depth as f32 * 14.0);
                            match l.part {
                                Some(k) if k != sel => {
                                    if ui.link(&l.text).clicked() {
                                        jump = Some(k);
                                    }
                                }
                                _ => {
                                    ui.add(egui::Label::new(egui::RichText::new(&l.text).monospace()).wrap());
                                }
                            }
                        });
                    }
                });
            }
        }
        if jump.is_some() {
            self.selected = jump;
            self.edits.clear();
            return;
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Settings");
            ui.checkbox(&mut self.all_settings, "All settings");
        });
        let Some(model) = &self.model else { return };
        let decl = model.parts.iter().find(|p| p.id == part.id).cloned();
        let file = decl.as_ref().and_then(|d| model.sources.files.get(d.file)).map(|f| PathBuf::from(&f.0));
        let Some(s) = spec else { return };
        let parts: Vec<(String, &'static str, &'static str)> = self.aircraft.as_ref().unwrap().checked.parts.iter().map(|p| (p.id.clone(), p.domain, p.kind)).collect();
        let names: Vec<String> = self.aircraft.as_ref().unwrap().checked.slots.names.iter().map(|n| n.0.clone()).collect();
        for f in s.fields {
            let current = decl.as_ref().and_then(|d| d.get(f.key)).map(|x| shown(&x.value));
            let set = current.is_some();
            if !self.all_settings && !set && f.default.is_some() {
                continue;
            }
            let entry = self.edits.entry(f.key.to_owned()).or_insert_with(|| current.clone().unwrap_or_default());
            ui.horizontal(|ui| {
                ui.label(f.key).on_hover_text(f.help);
                let hint = f.default.map(|d| format!("default {d}")).unwrap_or_else(|| "required".into());
                match f.kind {
                    Kind::Bool => {
                        let mut b = if entry.is_empty() { f.default == Some("true") } else { entry == "true" };
                        if ui.checkbox(&mut b, "").changed() {
                            *entry = b.to_string();
                        }
                    }
                    Kind::Ref(kinds) => {
                        let label = if entry.is_empty() { "(none)".to_owned() } else { entry.clone() };
                        egui::ComboBox::from_id_salt(f.key).selected_text(label).show_ui(ui, |ui| {
                            ui.selectable_value(entry, String::new(), "(none)");
                            for (id, d, k) in parts.iter().filter(|(_, d, k)| matches_kind(kinds, d, k)) {
                                ui.selectable_value(entry, id.clone(), format!("{id}  {d}.{k}"));
                            }
                        });
                    }
                    Kind::Refs(kinds) => {
                        ui.menu_button(if entry.is_empty() { "(none)".to_owned() } else { entry.clone() }, |ui| {
                            let mut list: Vec<String> = entry.split(',').map(|x| x.trim().to_owned()).filter(|x| !x.is_empty()).collect();
                            for (id, _, _) in parts.iter().filter(|(_, d, k)| matches_kind(kinds, d, k)) {
                                let mut on = list.contains(id);
                                if ui.checkbox(&mut on, id).changed() {
                                    if on {
                                        list.push(id.clone());
                                    } else {
                                        list.retain(|x| x != id);
                                    }
                                }
                            }
                            *entry = list.join(", ");
                        });
                    }
                    _ => {
                        ui.add(egui::TextEdit::singleline(entry).hint_text(hint).desired_width(230.0));
                    }
                }
            });
            match f.kind {
                Kind::Expr(want) if !entry.trim().is_empty() => {
                    let a = self.aircraft.as_ref().unwrap();
                    match easyacft_lang::expr::compile(entry.trim(), &a.checked.slots) {
                        Ok(p) if p.ty != want && !(matches!(want, Type::Num(_)) && matches!(p.ty, Type::Num(d) if d == easyacft_core::Dim::NONE)) => {
                            ui.small(format!("gives {}, needs {}", p.ty.name(), want.name()));
                        }
                        Ok(_) => {}
                        Err(e) => {
                            ui.small(format!("{}{}", e.message, e.help.map(|h| format!(" ({h})")).unwrap_or_default()));
                        }
                    }
                    let tok = last_token(entry).to_owned();
                    if tok.len() >= 2 && !names.contains(&tok) {
                        let hits: Vec<&String> = names.iter().filter(|n| n.starts_with(&tok)).take(6).collect();
                        let hits = if hits.is_empty() { names.iter().filter(|n| n.contains(&tok)).take(6).collect() } else { hits };
                        ui.horizontal_wrapped(|ui| {
                            for h in hits {
                                if ui.small_button(h.as_str()).clicked() {
                                    let keep = entry.len() - tok.len();
                                    entry.truncate(keep);
                                    entry.push_str(h);
                                }
                            }
                        });
                    }
                }
                Kind::Quantity(dim) if !entry.trim().is_empty() => match easyacft_core::parse_quantity(entry.trim()) {
                    Ok(q) if q.dim != dim => {
                        ui.small(format!("this is {}, it needs {}", q.dim.name(), dim.name()));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        ui.small(format!("{e}"));
                    }
                },
                _ => {}
            }
        }
        if part.kind == "gas_turbine" {
            let base = PathBuf::from(&self.dir);
            let g = easyacft_parts::engine::GasPath::new(engine_view::with_edits(easyacft_parts::engine::Params::from_part(&part, &base), &self.edits));
            ui.separator();
            ui.label("Design point, as typed");
            egui::Grid::new("design").show(ui, |ui| {
                for (k, v) in engine_view::design_rows(&g) {
                    ui.small(k);
                    ui.small(v);
                    ui.end_row();
                }
            });
        }
        let mut reload = false;
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                if let Some(f) = &file {
                    self.history.touch(f);
                    let mut msg = Vec::new();
                    for fs in s.fields {
                        let Some(v) = self.edits.get(fs.key) else { continue };
                        let had = decl.as_ref().and_then(|d| d.get(fs.key)).map(|x| shown(&x.value));
                        if had.as_deref() == Some(v.as_str()) || (v.is_empty() && had.is_none()) {
                            continue;
                        }
                        let r = match fs.kind {
                            Kind::Number | Kind::Bool => edit::set(f, &part.id, fs.key, v),
                            Kind::Refs(_) => {
                                let list: Vec<String> = v.split(',').map(|x| format!("\"{}\"", x.trim())).filter(|x| x != "\"\"").collect();
                                edit::set(f, &part.id, fs.key, &if v.trim().is_empty() { String::new() } else { format!("[{}]", list.join(", ")) })
                            }
                            _ => edit::set_as(f, &part.id, fs.key, v, true),
                        };
                        if let Err(e) = r {
                            msg.push(e);
                        }
                    }
                    self.history.commit();
                    if !msg.is_empty() {
                        self.status = msg.join("; ");
                    }
                    reload = true;
                }
            }
            if ui.button("Remove part").clicked() {
                if let Some(f) = &file {
                    self.history.touch(f);
                    let _ = edit::remove(f, &part.id);
                    self.history.commit();
                    self.selected = None;
                    reload = true;
                }
            }
        });
        ui.separator();
        ui.label("Bind to X-Plane");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("bind_out").selected_text(self.bind_out.clone()).show_ui(ui, |ui| {
                for (n, _) in &outs {
                    ui.selectable_value(&mut self.bind_out, n.clone(), n);
                }
            });
            ui.add(egui::TextEdit::singleline(&mut self.bind_ref).hint_text("dataref").desired_width(200.0));
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.bind_write, "X-Plane sets it");
            ui.add(egui::TextEdit::singleline(&mut self.bind_unit).hint_text("unit, e.g. psi").desired_width(80.0));
            if ui.button("Bind").clicked() && !self.bind_ref.trim().is_empty() && !self.bind_out.is_empty() {
                let main = PathBuf::from(&self.dir).join("aircraft.toml");
                self.history.touch(&main);
                match edit::bind(&main, self.bind_ref.trim(), &format!("{}.{}", part.id, self.bind_out), self.bind_write, self.bind_unit.trim()) {
                    Ok(()) => {
                        self.status = format!("bound {} to {}.{}", self.bind_ref.trim(), part.id, self.bind_out);
                        reload = true;
                    }
                    Err(e) => self.status = e,
                }
                self.history.commit();
            }
        });
        if reload {
            self.reload();
        }
    }
}
