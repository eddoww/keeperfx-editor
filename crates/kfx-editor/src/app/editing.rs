//! The slab tools in slab coordinates (the viewport and the headless actions both drive them):
//! palette selection, brush and owner settings, strokes and shapes, the eyedropper, undo / redo,
//! selection, copy / cut / paste and the hovered slab.
use super::*;

impl EditorApp {
    pub(super) fn gd(&self) -> Option<&GameData> {
        self.doc.as_ref().map(|d| d.gd.as_ref())
    }

    /// Select a palette slab by slab name (`TREASURY_AREA`) or room name (`TREASURE`).
    pub fn select_palette(&mut self, name: &str) -> Result<(), String> {
        let gd = self.doc.as_ref().map(|d| d.gd.clone()).ok_or("no map")?;
        let gd = gd.as_ref();
        let kind = gd
            .slab_id(name)
            .or_else(|| {
                gd.room_id(name)
                    .map(|r| gd.rooms[r].assigned_slab as usize)
                    .filter(|&k| k > 0 || name.eq_ignore_ascii_case("HARD"))
            })
            .ok_or_else(|| format!("no slab or room named {name:?}"))?;
        if assets::is_door(gd, kind) {
            return Err(trf(
                "kfx_editor.name_is_a_door_doors_are_placed_with_things",
                "{name} is a door; doors are placed with things",
                &[("name", &name)],
            ));
        }
        self.brush = Some(kind as u16);
        self.custom_brush = None;
        if gd.slab_room(kind) > 0 {
            self.category = Category::Rooms;
        } else {
            self.category = Category::Terrain;
        }
        Ok(())
    }

    pub fn set_active_owner(&mut self, o: u8) {
        self.active_owner = o.min(8);
    }

    pub fn set_tool(&mut self, t: Tool) {
        self.cancel_tool();
        self.pasting = false;
        self.tool = t;
    }

    pub fn set_brush(&mut self, size: u32, round: bool) {
        self.brush_size = size.clamp(1, tools::MAX_BRUSH);
        self.brush_round = round;
    }

    pub fn set_shape_filled(&mut self, f: bool) {
        self.shape_filled = f;
    }

    pub fn set_owner_only(&mut self, f: bool) {
        self.owner_only = f;
    }

    pub fn set_editable_border(&mut self, f: bool) {
        self.editable_border = f;
    }

    pub fn set_view_flags(
        &mut self,
        ownership: Option<bool>,
        grid: Option<bool>,
        lighting: Option<bool>,
    ) {
        if let Some(v) = ownership {
            self.settings.layout.show_ownership = v;
        }
        if let Some(v) = grid {
            self.settings.layout.show_grid = v;
        }
        if let Some(v) = lighting {
            self.settings.layout.lighting = v;
        }
    }

    pub(super) fn editable(&self, x: i32, y: i32) -> bool {
        let Some(d) = &self.doc else { return false };
        let dm = d.dims();
        if !d.in_map(x, y) || d.doc.sidecar.locked_at(x, y).is_some() {
            return false;
        }
        self.editable_border || (x > 0 && y > 0 && x < dm.w as i32 - 1 && y < dm.h as i32 - 1)
    }

    /// Owner a placed slab gets: the active owner for ownable kinds, else neutral.
    pub(super) fn owner_for(&self, kind: u16) -> u8 {
        match self.gd() {
            Some(gd) if gd.slab(kind as usize).is_ownable != 0 || self.settings.ownable_natural => {
                self.active_owner
            }
            _ => PLAYER_NEUTRAL,
        }
    }

    /// Whether ownership painting may change slab (x, y): ownable kinds, or anything with
    /// "Ownable natural terrain" on.
    pub(super) fn owner_paintable(&self, x: i32, y: i32) -> bool {
        if self.settings.ownable_natural {
            return true;
        }
        match (self.doc.as_ref(), self.gd()) {
            (Some(d), Some(gd)) => d
                .state_at(x, y)
                .is_some_and(|s| gd.slab(s.kind as usize).is_ownable != 0),
            _ => true,
        }
    }

    pub(super) fn brush_name(&self) -> String {
        if let (Some(id), Some(d)) = (self.custom_brush, &self.doc) {
            return d
                .doc
                .sidecar
                .custom_slab(id)
                .map(|c| {
                    trf(
                        "kfx_editor.custom_name",
                        "custom {name}",
                        &[("name", &c.name)],
                    )
                })
                .unwrap_or_else(|| trf("kfx_editor.custom_id", "custom {id}", &[("id", &id)]));
        }
        match (self.brush, self.gd()) {
            (Some(k), Some(gd)) => {
                let r = gd.slab_room(k as usize);
                if r > 0 {
                    gd.room_name(r).unwrap_or("?").to_string()
                } else {
                    gd.slab_name(k as usize).unwrap_or("?").to_string()
                }
            }
            _ => trd("kfx_editor.none", "(none)").into(),
        }
    }

    pub(super) fn edits_for(&self, cells: &[(i32, i32)]) -> Vec<(i32, i32, Edit)> {
        let e = if let Some(v) = self.style_brush {
            Some(Edit::Style(v))
        } else if self.owner_only {
            Some(Edit::Owner(self.active_owner))
        } else if let Some(id) = self.custom_brush {
            let k = self
                .doc
                .as_ref()
                .and_then(|d| d.doc.sidecar.custom_slab(id))
                .map(|c| c.recognized_as)
                .unwrap_or(0);
            Some(Edit::Custom {
                id,
                owner: self.owner_for(k),
            })
        } else {
            self.brush.map(|k| Edit::Place {
                kind: k,
                owner: self.owner_for(k),
            })
        };
        let Some(e) = e else { return vec![] };
        let mut out = vec![];
        let mut seen = HashSet::new();
        for &(x, y) in cells {
            let imgs: Vec<(i32, i32, Edit)> = match (&self.symmetry, &self.doc) {
                (Some(s), Some(d)) => {
                    let owner = match e {
                        Edit::Place { owner, .. }
                        | Edit::Owner(owner)
                        | Edit::Custom { owner, .. } => owner,
                        Edit::Style(_) => 0,
                    };
                    s.slab_edits(d.dims(), x, y, 0, owner)
                        .into_iter()
                        .map(|(ix, iy, _, o)| {
                            let ie = match e {
                                Edit::Place { kind, owner } => Edit::Place {
                                    kind,
                                    owner: if owner == PLAYER_NEUTRAL { owner } else { o },
                                },
                                Edit::Custom { id, owner } => Edit::Custom {
                                    id,
                                    owner: if owner == PLAYER_NEUTRAL { owner } else { o },
                                },
                                Edit::Owner(_) => Edit::Owner(o),
                                Edit::Style(v) => Edit::Style(v),
                            };
                            (ix, iy, ie)
                        })
                        .collect()
                }
                _ => vec![(x, y, e)],
            };
            for (ix, iy, ie) in imgs {
                if matches!(ie, Edit::Owner(_)) && !self.owner_paintable(ix, iy) {
                    continue;
                }
                if self.editable(ix, iy) && seen.insert((ix, iy)) {
                    out.push((ix, iy, ie));
                }
            }
        }
        // Fortify: the earth around placed ground of a keeper becomes that keeper's walls
        if self.fortify {
            if let (Some(d), Some(gd)) = (&self.doc, self.gd()) {
                let seed = kfx_map::decorate::Decorator::seed_for_level(d.doc.level);
                for (x, y, e) in
                    crate::fortify::fortify_edits(&d.doc, gd, &out, self.settings.wall_art, seed)
                {
                    if self.editable(x, y) && seen.insert((x, y)) {
                        out.push((x, y, e));
                    }
                }
            }
        }
        out
    }

    pub(super) fn stroke_name(&self, tool: Tool) -> String {
        if let Some(v) = self.style_brush {
            trf(
                "kfx_editor.tool_style_v",
                "{tool} style {v}",
                &[("tool", &tool.label()), ("v", &v)],
            )
        } else if self.owner_only {
            trf(
                "kfx_editor.tool_owner_active_owner",
                "{tool} owner {active_owner}",
                &[
                    ("tool", &tool.label()),
                    ("active_owner", &PLAYERS[self.active_owner as usize].0),
                ],
            )
        } else {
            format!("{} {}", tool.label(), self.brush_name())
        }
    }

    pub(super) fn apply_cells(&mut self, cells: &[(i32, i32)]) {
        let edits = self.edits_for(cells);
        if edits.is_empty() {
            return;
        }
        let r = self.doc.as_mut().and_then(|d| d.apply(&edits));
        if let Some(r) = r {
            self.after_change(r);
        }
    }

    /// Run a whole edit as one undo step.
    pub(super) fn one_step(&mut self, name: &str, edits: &[(i32, i32, Edit)]) {
        let Some(d) = self.doc.as_mut() else { return };
        d.begin_stroke(name);
        let r = d.apply(edits);
        d.end_stroke();
        if let Some(r) = r {
            self.after_change(r);
        }
    }

    pub(super) fn can_draw(&mut self) -> bool {
        if self.brush.is_none()
            && self.custom_brush.is_none()
            && !self.owner_only
            && self.style_brush.is_none()
        {
            self.status = trd(
                "kfx_editor.choose_a_slab_on_the_palette_first_or_turn_on_ow",
                "Choose a slab on the palette first (or turn on \"Ownership only\").",
            );
            return false;
        }
        true
    }

    /// Left button pressed on slab `p` (`alt`: eyedropper).
    pub fn tool_press(&mut self, p: (i32, i32), alt: bool) {
        if self.doc.is_none() {
            return;
        }
        if alt || self.tool == Tool::Eyedropper {
            self.eyedrop(p);
            return;
        }
        if self.pasting {
            self.stamp_paste(p);
            return;
        }
        let drag = Drag {
            tool: self.tool,
            start: p,
            last: p,
            cur: p,
        };
        match self.tool {
            Tool::Paint => {
                if !self.can_draw() {
                    return;
                }
                let name = self.stroke_name(Tool::Paint);
                if let Some(d) = self.doc.as_mut() {
                    d.begin_stroke(&name);
                }
                let cells = tools::stroke_cells(
                    p,
                    p,
                    &tools::brush_cells(self.brush_size, self.brush_round),
                );
                self.apply_cells(&cells);
                self.drag = Some(drag);
            }
            Tool::Line | Tool::Box | Tool::Circle => {
                if self.can_draw() {
                    self.drag = Some(drag);
                }
            }
            Tool::Select => {
                self.selection = None;
                self.drag = Some(drag);
            }
            Tool::Fill => {
                if !self.can_draw() {
                    return;
                }
                let ed = self.editable_border;
                let Some(doc) = &self.doc else { return };
                let dm = doc.dims();
                let cells = tools::flood_fill(doc, p, |x, y| {
                    ed || (x > 0 && y > 0 && x < dm.w as i32 - 1 && y < dm.h as i32 - 1)
                });
                let edits = self.edits_for(&cells);
                let name = trf(
                    "kfx_editor.stroke_name_edits_slabs",
                    "{stroke_name} ({edits} slabs)",
                    &[
                        ("stroke_name", &self.stroke_name(Tool::Fill)),
                        ("edits", &edits.len()),
                    ],
                );
                self.one_step(&name, &edits);
            }
            Tool::Eyedropper | Tool::Place | Tool::Things => {}
        }
    }

    /// Pointer moved to slab `p` with the left button held.
    pub fn tool_drag(&mut self, p: (i32, i32)) {
        let Some(mut d) = self.drag else { return };
        if d.tool == Tool::Paint && p != d.last {
            let cells = tools::stroke_cells(
                d.last,
                p,
                &tools::brush_cells(self.brush_size, self.brush_round),
            );
            self.apply_cells(&cells);
            d.last = p;
        }
        d.cur = p;
        self.drag = Some(d);
    }

    /// Left button released on slab `p`.
    pub fn tool_release(&mut self, p: (i32, i32)) {
        self.tool_drag(p);
        let Some(d) = self.drag.take() else { return };
        match d.tool {
            Tool::Paint => {
                if let Some(doc) = self.doc.as_mut() {
                    doc.end_stroke();
                }
            }
            Tool::Line | Tool::Box | Tool::Circle => {
                let cells = self.shape_cells(&d);
                let edits = self.edits_for(&cells);
                let name = self.stroke_name(d.tool);
                self.one_step(&name, &edits);
            }
            Tool::Select => {
                let (a, b) = ordered(d.start, d.cur);
                let dm = self
                    .doc
                    .as_ref()
                    .map(|x| x.dims())
                    .unwrap_or(MapDims::new(1, 1));
                let a = (a.0.max(0), a.1.max(0));
                let b = (b.0.min(dm.w as i32 - 1), b.1.min(dm.h as i32 - 1));
                self.selection = (a.0 <= b.0 && a.1 <= b.1).then_some((a, b));
            }
            _ => {}
        }
    }

    pub(super) fn shape_cells(&self, d: &Drag) -> Vec<(i32, i32)> {
        match d.tool {
            Tool::Line => tools::stroke_cells(
                d.start,
                d.cur,
                &tools::brush_cells(self.brush_size, self.brush_round),
            ),
            Tool::Box => tools::rect_cells(d.start, d.cur, self.shape_filled),
            Tool::Circle => tools::circle_cells(
                d.start,
                tools::circle_radius(d.start, d.cur),
                self.shape_filled,
            ),
            Tool::Select => tools::rect_cells(d.start, d.cur, true),
            Tool::Paint => tools::brush_cells(self.brush_size, self.brush_round)
                .into_iter()
                .map(|(x, y)| (x + d.cur.0, y + d.cur.1))
                .collect(),
            _ => vec![d.cur],
        }
    }

    /// Escape: cancel the shape / stroke in progress, else leave paste mode, else drop the
    /// selection.
    pub fn cancel_tool(&mut self) {
        if self.pick.take().is_some() {
            self.status = trd("kfx_editor.pick_cancelled", "Pick cancelled");
        } else if self.thing_drag.take().is_some() || self.rect_sel.take().is_some() {
            self.status = trd("kfx_editor.cancelled", "Cancelled");
        } else if let Some(d) = self.drag.take() {
            if d.tool == Tool::Paint {
                let r = self.doc.as_mut().and_then(|x| x.cancel_stroke());
                if let Some(r) = r {
                    self.after_change(r);
                }
            }
            self.status = trd("kfx_editor.cancelled", "Cancelled");
        } else if self.pasting {
            self.pasting = false;
        } else if !self.selected.is_empty() {
            self.selected.clear();
        } else {
            self.selection = None;
        }
    }

    pub(super) fn eyedrop(&mut self, p: (i32, i32)) {
        let Some(s) = self.doc.as_ref().and_then(|d| d.state_at(p.0, p.1)) else {
            return;
        };
        let gd = self.doc.as_ref().unwrap().gd.clone();
        let gd = gd.as_ref();
        let custom = self
            .doc
            .as_ref()
            .and_then(|d| d.doc.custom_slab_at(p.0, p.1))
            .map(|c| c.id);
        if let Some(id) = custom {
            self.select_custom_brush(id);
        } else if assets::is_door(gd, s.kind as usize) {
            self.status = trd(
                "kfx_editor.doors_cannot_be_picked_as_slabs",
                "Doors cannot be picked as slabs",
            );
        } else {
            self.custom_brush = None;
            self.brush = Some(s.kind);
            self.category = if gd.slab_room(s.kind as usize) > 0 {
                Category::Rooms
            } else {
                Category::Terrain
            };
        }
        if (s.owner as usize) < 9 {
            self.active_owner = s.owner;
        }
        self.status = trf(
            "kfx_editor.picked_brush_name_owned_by_p",
            "Picked {brush_name} owned by {p}",
            &[
                ("brush_name", &self.brush_name()),
                (
                    "p",
                    &PLAYERS
                        .get(s.owner as usize)
                        .map(|p| p.0.text())
                        .unwrap_or_else(|| "?".into()),
                ),
            ],
        );
    }

    pub fn undo(&mut self) {
        self.drag = None;
        let Some(d) = self.doc.as_mut() else { return };
        let name = d.undo_name().map(str::to_string);
        let r = d.undo();
        if let Some(r) = r {
            self.after_change(r);
        }
        if let Some(n) = name {
            self.status = trf("kfx_editor.undo_n", "Undo: {n}", &[("n", &n)]);
        }
    }

    pub fn redo(&mut self) {
        self.drag = None;
        let Some(d) = self.doc.as_mut() else { return };
        let name = d.redo_name().map(str::to_string);
        let r = d.redo();
        if let Some(r) = r {
            self.after_change(r);
        }
        if let Some(n) = name {
            self.status = trf("kfx_editor.redo_n", "Redo: {n}", &[("n", &n)]);
        }
    }

    pub fn undo_all(&mut self) {
        self.drag = None;
        let r = self.doc.as_mut().and_then(|d| d.jump_to(0));
        if let Some(r) = r {
            self.after_change(r);
        }
    }

    pub(super) fn jump_history(&mut self, pos: usize) {
        self.drag = None;
        let r = self.doc.as_mut().and_then(|d| d.jump_to(pos));
        if let Some(r) = r {
            self.after_change(r);
        }
    }

    pub fn select_rect(&mut self, a: (i32, i32), b: (i32, i32)) {
        self.selection = Some(ordered(a, b));
    }

    pub fn select_all(&mut self) {
        if let Some(d) = &self.doc {
            let dm = d.dims();
            self.selection = Some(((0, 0), (dm.w as i32 - 1, dm.h as i32 - 1)));
        }
    }

    pub fn copy(&mut self) -> bool {
        let things_first =
            matches!(self.tool, Tool::Things | Tool::Place) || self.selection.is_none();
        if things_first && !self.selected.is_empty() {
            if let Some(d) = &self.doc {
                let items: Vec<ItemRef> = self.selected.iter().copied().collect();
                self.clipboard = Clipboard::copy_items(d, &items);
                if let Some(c) = &self.clipboard {
                    self.status = trf(
                        "kfx_editor.copied_items_items",
                        "Copied {items} items",
                        &[("items", &c.items.len())],
                    );
                    self.ctx
                        .copy_text(format!("KeeperFX items {}", c.items.len()));
                }
                return self.clipboard.is_some();
            }
        }
        let (Some((a, b)), Some(d)) = (self.selection, &self.doc) else {
            self.status = trd("kfx_editor.nothing_selected", "Nothing selected");
            return false;
        };
        self.clipboard = Clipboard::copy(d, a, b);
        if let Some(c) = &self.clipboard {
            self.status = trf(
                "kfx_editor.copied_w_x_h_slabs_with_items_items",
                "Copied {w}x{h} slabs with {items} items",
                &[("w", &c.w), ("h", &c.h), ("items", &c.items.len())],
            );
            // so that Ctrl+V reaches the app as a paste event (egui only sends one when the
            // system clipboard holds text)
            self.ctx
                .copy_text(format!("KeeperFX slabs {}x{}", c.w, c.h));
        }
        self.clipboard.is_some()
    }

    pub fn cut(&mut self) {
        if self.copy() {
            if self.clipboard.as_ref().is_some_and(|c| c.w == 0) {
                self.delete_things();
            } else {
                self.delete_selection_named(&trd("kfx_editor.cut", "Cut"));
            }
        }
    }

    /// Delete the selected things, else fill the slab selection with the current palette slab.
    pub fn delete_selection(&mut self) {
        if !self.selected.is_empty()
            && (self.selection.is_none() || matches!(self.tool, Tool::Things | Tool::Place))
        {
            self.delete_things();
        } else {
            self.delete_selection_named(&trd("kfx_editor.delete_3aea", "Delete"));
        }
    }

    pub(super) fn delete_selection_named(&mut self, what: &str) {
        let Some((a, b)) = self.selection else { return };
        let kind = self.brush.unwrap_or(kfx_map::autotile::kinds::EARTH as u16);
        let owner = self.owner_for(kind);
        let cells = tools::rect_cells(a, b, true);
        let edits: Vec<(i32, i32, Edit)> = cells
            .into_iter()
            .filter(|&(x, y)| self.editable(x, y))
            .map(|(x, y)| (x, y, Edit::Place { kind, owner }))
            .collect();
        let name = trf(
            "kfx_editor.what_selection",
            "{what} selection",
            &[("what", &what)],
        );
        self.one_step(&name, &edits);
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_some() {
            self.pasting = true;
            self.drag = None;
            self.status = trd(
                "kfx_editor.paste_click_to_stamp_r_rotate_f_flip_esc_stop",
                "Paste: click to stamp · R rotate · F flip · Esc stops",
            );
        } else {
            self.status = trd(
                "kfx_editor.the_clipboard_is_empty",
                "The clipboard is empty",
            );
        }
    }

    pub fn rotate_paste(&mut self) {
        if self.prefab_turn(false) {
            return;
        }
        if let Some(c) = &self.clipboard {
            self.clipboard = Some(c.rotate());
        }
    }

    pub fn flip_paste(&mut self) {
        if self.prefab_turn(true) {
            return;
        }
        if let Some(c) = &self.clipboard {
            self.clipboard = Some(c.flip());
        }
    }

    pub(super) fn stamp_paste(&mut self, p: (i32, i32)) {
        if self.prefab_stamp(p) {
            return;
        }
        let Some(c) = self.clipboard.clone() else {
            return;
        };
        let stl = self
            .hover
            .as_ref()
            .map(|h| (h.hit.stl_x as i32, h.hit.stl_y as i32))
            .unwrap_or((p.0 * 3 + 1, p.1 * 3 + 1));
        let edits: Vec<(i32, i32, Edit)> = c
            .stamp(p)
            .into_iter()
            .filter(|&(x, y, _, _)| self.editable(x, y))
            .map(|(x, y, k, o)| (x, y, Edit::Place { kind: k, owner: o }))
            .collect();
        let name = if c.w > 0 {
            trf(
                "kfx_editor.paste_w_x_h",
                "Paste {w}x{h}",
                &[("w", &c.w), ("h", &c.h)],
            )
        } else {
            trf(
                "kfx_editor.paste_items_items",
                "Paste {items} items",
                &[("items", &c.items.len())],
            )
        };
        let Some(d) = self.doc.as_mut() else { return };
        let dims = d.dims();
        let before = things::ItemsBefore::of(&d.doc);
        d.begin_stroke(&name);
        let mut rf = d.apply(&edits).unwrap_or_default();
        let inside = |x: Option<kfx_formats::level_fx::StlCoord>,
                      y: Option<kfx_formats::level_fx::StlCoord>| {
            x.zip(y).is_some_and(|(x, y)| {
                x.stl >= 0
                    && y.stl >= 0
                    && (x.stl as usize) < dims.stl_w()
                    && (y.stl as usize) < dims.stl_h()
            })
        };
        let items = c.stamp_items(p, stl);
        let (_, r2) = d.edit(&name, |m, g, _| {
            for it in items {
                match it {
                    tools::ClipItem::Thing(mut t) if inside(t.x, t.y) => {
                        let (x, y) = (t.x.unwrap(), t.y.unwrap());
                        let above = 0;
                        if t.z
                            .is_some_and(|z| z.stl < m.floor_height(x.stl, y.stl) as i32)
                            || t.z.is_none()
                        {
                            t.z = Some(kfx_formats::level_fx::StlCoord::new(
                                m.floor_height(x.stl, y.stl) as i32 + above,
                                0,
                            ));
                        }
                        let _ = m.add_thing(g, t);
                    }
                    tools::ClipItem::ActionPoint(a) if inside(a.x, a.y) => {
                        let _ = m.add_action_point(
                            a.x.unwrap(),
                            a.y.unwrap(),
                            a.range.unwrap_or_default(),
                            None,
                        );
                    }
                    tools::ClipItem::Light(l) if inside(l.x, l.y) => m.lights.items.push(l),
                    _ => {}
                }
            }
        });
        rf = rf.merge(r2);
        d.end_stroke();
        let new = before.changed(&self.doc.as_ref().unwrap().doc);
        if c.w == 0 {
            self.selected = new;
        }
        self.after_change(rf);
    }

    /// Move the pointer to slab `p` (headless): sets the hover for previews.
    pub fn hover_slab(&mut self, p: (i32, i32)) {
        if p.0 < 0 || p.1 < 0 {
            self.hover = None;
            return;
        }
        let (x, y) = (p.0 as usize * 3 + 1, p.1 as usize * 3 + 1);
        let h = self.terrain.top_height(x, y) as f32;
        self.hover = Some(HoverInfo {
            hit: Hit {
                stl_x: x,
                stl_y: y,
                point: Vec3::new(x as f32 + 0.5, h, y as f32 + 0.5),
                height: h,
            },
            slab: (p.0 as usize, p.1 as usize),
        });
    }
}
