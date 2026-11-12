//! Reversible document changes for an undo journal.
//!
//! A [`Patch`] is an ordered list of [`Change`]s. [`Patch::apply`] runs them forward,
//! [`Patch::revert`] runs the inverses backwards, so `revert(apply(doc)) == doc` for every patch
//! made against that document state. Everything a [`MapDoc`] holds is covered: slab layers (kind,
//! owner, `.wlb`, `.slx`), the subtile grid (columns, wibble), things, action points, lights, the
//! map properties, and (for size changes) the whole document.
//!
//! Patches are compact: grid changes are stored as the bounding rectangle of the changed cells,
//! list changes as per-item edits after trimming the common prefix and suffix.
//!
//! The easy way to get a patch for any edit is [`MapDoc::record`]: it snapshots the document,
//! runs the edit and diffs ([`Patch::diff`]). That clones the whole document per edit; the
//! cheaper [`MapDoc::record_tracked`] logs what the edit touches instead and builds the same
//! patch (see its docs for the rules). The thing / action point / light operations of
//! [`crate::things`] return their patches directly.
use crate::mapdoc::MapDoc;
use crate::sidecar::Sidecar;
use kfx_formats::level_bin::{Column, Flg, MapDims, Own};
use kfx_formats::level_fx::{FxActionPoint, FxLight, FxThing};
use std::collections::HashMap;

/// Inclusive rectangle of grid cells (slabs or subtiles, depending on the change).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl CellRect {
    pub fn w(&self) -> usize {
        self.x1 - self.x0 + 1
    }
    pub fn h(&self) -> usize {
        self.y1 - self.y0 + 1
    }
    pub fn cells(&self) -> usize {
        self.w() * self.h()
    }
    fn iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let r = *self;
        (r.y0..=r.y1).flat_map(move |y| (r.x0..=r.x1).map(move |x| (x, y)))
    }
}

/// The slab layers of a rectangle of slabs, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlabBlock {
    pub kinds: Vec<u16>,
    pub owners: Vec<u8>,
    pub wlb: Vec<u8>,
    pub slx: Vec<u8>,
    /// Custom slab id per slab (0 = none), see [`crate::sidecar`].
    pub custom: Vec<u16>,
}

/// Columns and wibble of a rectangle of subtiles, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StlBlock {
    pub columns: Vec<Column>,
    pub wibble: Vec<u8>,
}

/// Scalar map properties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Props {
    pub texture_id: u8,
    pub inf_trailing: Vec<u8>,
    pub wlb_from_file: bool,
    /// Editor sidecar data: custom slab definitions and editor settings.
    pub sidecar: Box<Sidecar>,
}

/// Every editable part of a document (used for size changes and whole-map transforms).
#[derive(Clone, Debug, PartialEq)]
pub struct DocState {
    pub dims: MapDims,
    pub kinds: Vec<u16>,
    pub owners: Vec<u8>,
    pub wlb: Vec<u8>,
    pub slx: Vec<u8>,
    pub custom: Vec<u16>,
    pub columns: Vec<Column>,
    pub wibble: Vec<u8>,
    pub things: Vec<FxThing>,
    pub action_points: Vec<FxActionPoint>,
    pub lights: Vec<FxLight>,
    pub own_loaded: Option<Own>,
    pub flg: Option<Flg>,
    pub props: Props,
}

impl DocState {
    pub fn of(doc: &MapDoc) -> DocState {
        DocState {
            dims: doc.dims,
            kinds: doc.kinds.clone(),
            owners: doc.owners.clone(),
            wlb: doc.wlb.clone(),
            slx: doc.slx.clone(),
            custom: doc.custom_cells.clone(),
            columns: doc.columns.clone(),
            wibble: doc.wibble.clone(),
            things: doc.things.items.clone(),
            action_points: doc.action_points.items.clone(),
            lights: doc.lights.items.clone(),
            own_loaded: doc.own_loaded.clone(),
            flg: doc.flg.clone(),
            props: props_of(doc),
        }
    }
    fn put(&self, doc: &mut MapDoc) {
        doc.dims = self.dims;
        doc.kinds = self.kinds.clone();
        doc.owners = self.owners.clone();
        doc.wlb = self.wlb.clone();
        doc.slx = self.slx.clone();
        doc.custom_cells = self.custom.clone();
        doc.columns = self.columns.clone();
        doc.wibble = self.wibble.clone();
        doc.things.items = self.things.clone();
        doc.action_points.items = self.action_points.clone();
        doc.lights.items = self.lights.clone();
        doc.own_loaded = self.own_loaded.clone();
        doc.flg = self.flg.clone();
        set_props(doc, &self.props);
    }
}

fn props_of(doc: &MapDoc) -> Props {
    Props {
        texture_id: doc.texture_id,
        inf_trailing: doc.inf_trailing.clone(),
        wlb_from_file: doc.wlb_from_file,
        sidecar: Box::new(doc.sidecar.clone()),
    }
}

fn set_props(doc: &mut MapDoc, p: &Props) {
    doc.texture_id = p.texture_id;
    doc.inf_trailing = p.inf_trailing.clone();
    doc.wlb_from_file = p.wlb_from_file;
    doc.sidecar = (*p.sidecar).clone();
}

/// One reversible change.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Slab layers of `rect` (slab coordinates).
    Slabs {
        rect: CellRect,
        before: SlabBlock,
        after: SlabBlock,
    },
    /// Columns / wibble of `rect` (subtile coordinates, the extra row/column included).
    Subtiles {
        rect: CellRect,
        before: StlBlock,
        after: StlBlock,
    },
    /// `None -> Some` inserts at `index`, `Some -> None` removes at `index`, `Some -> Some`
    /// replaces.
    Thing {
        index: usize,
        before: Option<FxThing>,
        after: Option<FxThing>,
    },
    ActionPoint {
        index: usize,
        before: Option<FxActionPoint>,
        after: Option<FxActionPoint>,
    },
    Light {
        index: usize,
        before: Option<FxLight>,
        after: Option<FxLight>,
    },
    Props {
        before: Props,
        after: Props,
    },
    /// The whole document (size changes).
    Whole {
        before: Box<DocState>,
        after: Box<DocState>,
    },
}

fn list_set<T: Clone>(v: &mut Vec<T>, index: usize, from: &Option<T>, to: &Option<T>) {
    match (from, to) {
        (None, Some(t)) => v.insert(index.min(v.len()), t.clone()),
        (Some(_), None) => {
            if index < v.len() {
                v.remove(index);
            }
        }
        (Some(_), Some(t)) => {
            if let Some(slot) = v.get_mut(index) {
                *slot = t.clone();
            }
        }
        (None, None) => {}
    }
}

impl Change {
    fn put(&self, doc: &mut MapDoc, forward: bool) {
        match self {
            Change::Slabs {
                rect,
                before,
                after,
            } => {
                let b = if forward { after } else { before };
                for (k, (x, y)) in rect.iter().enumerate() {
                    let i = doc.dims.slab_index(x, y);
                    doc.kinds[i] = b.kinds[k];
                    doc.owners[i] = b.owners[k];
                    doc.wlb[i] = b.wlb[k];
                    doc.slx[i] = b.slx[k];
                    doc.custom_cells[i] = b.custom[k];
                }
            }
            Change::Subtiles {
                rect,
                before,
                after,
            } => {
                let b = if forward { after } else { before };
                for (k, (x, y)) in rect.iter().enumerate() {
                    let i = doc.dims.stl_index(x, y);
                    doc.columns[i] = b.columns[k];
                    doc.wibble[i] = b.wibble[k];
                }
            }
            Change::Thing {
                index,
                before,
                after,
            } => {
                let (f, t) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                list_set(&mut doc.things.items, *index, f, t);
            }
            Change::ActionPoint {
                index,
                before,
                after,
            } => {
                let (f, t) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                list_set(&mut doc.action_points.items, *index, f, t);
            }
            Change::Light {
                index,
                before,
                after,
            } => {
                let (f, t) = if forward {
                    (before, after)
                } else {
                    (after, before)
                };
                list_set(&mut doc.lights.items, *index, f, t);
            }
            Change::Props { before, after } => set_props(doc, if forward { after } else { before }),
            Change::Whole { before, after } => (if forward { after } else { before }).put(doc),
        }
    }

    /// Rough memory size, for journal budgeting.
    pub fn size_hint(&self) -> usize {
        match self {
            Change::Slabs { rect, .. } => rect.cells() * 12,
            Change::Subtiles { rect, .. } => rect.cells() * 2 * (std::mem::size_of::<Column>() + 1),
            Change::Whole { before, after } => {
                (before.columns.len() + after.columns.len()) * (std::mem::size_of::<Column>() + 1)
                    + (before.kinds.len() + after.kinds.len()) * 5
            }
            _ => 256,
        }
    }
}

/// An ordered, reversible list of changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    pub changes: Vec<Change>,
}

impl Patch {
    pub fn new() -> Patch {
        Patch::default()
    }
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    /// Append `other` (applied after `self`).
    pub fn extend(&mut self, other: Patch) {
        self.changes.extend(other.changes);
    }
    pub fn apply(&self, doc: &mut MapDoc) {
        for c in &self.changes {
            c.put(doc, true);
        }
    }
    pub fn revert(&self, doc: &mut MapDoc) {
        for c in self.changes.iter().rev() {
            c.put(doc, false);
        }
    }
    /// The patch that undoes this one.
    pub fn inverted(&self) -> Patch {
        let changes = self
            .changes
            .iter()
            .rev()
            .map(|c| match c.clone() {
                Change::Slabs {
                    rect,
                    before,
                    after,
                } => Change::Slabs {
                    rect,
                    before: after,
                    after: before,
                },
                Change::Subtiles {
                    rect,
                    before,
                    after,
                } => Change::Subtiles {
                    rect,
                    before: after,
                    after: before,
                },
                Change::Thing {
                    index,
                    before,
                    after,
                } => Change::Thing {
                    index,
                    before: after,
                    after: before,
                },
                Change::ActionPoint {
                    index,
                    before,
                    after,
                } => Change::ActionPoint {
                    index,
                    before: after,
                    after: before,
                },
                Change::Light {
                    index,
                    before,
                    after,
                } => Change::Light {
                    index,
                    before: after,
                    after: before,
                },
                Change::Props { before, after } => Change::Props {
                    before: after,
                    after: before,
                },
                Change::Whole { before, after } => Change::Whole {
                    before: after,
                    after: before,
                },
            })
            .collect();
        Patch { changes }
    }
    pub fn size_hint(&self) -> usize {
        self.changes.iter().map(Change::size_hint).sum()
    }

    /// The patch turning `before` into `after`.
    pub fn diff(before: &MapDoc, after: &MapDoc) -> Patch {
        let mut p = Patch::new();
        if before.dims != after.dims
            || before.own_loaded != after.own_loaded
            || before.flg != after.flg
        {
            p.changes.push(Change::Whole {
                before: Box::new(DocState::of(before)),
                after: Box::new(DocState::of(after)),
            });
            return p;
        }
        let d = before.dims;
        // slab layers
        let slab_diff = |x: usize, y: usize| {
            let i = d.slab_index(x, y);
            before.kinds[i] != after.kinds[i]
                || before.owners[i] != after.owners[i]
                || before.wlb[i] != after.wlb[i]
                || before.slx[i] != after.slx[i]
                || before.custom_cells[i] != after.custom_cells[i]
        };
        if let Some(rect) = bounding(d.w as usize, d.h as usize, slab_diff) {
            let block = |doc: &MapDoc| {
                let idx: Vec<usize> = rect.iter().map(|(x, y)| d.slab_index(x, y)).collect();
                SlabBlock {
                    kinds: idx.iter().map(|&i| doc.kinds[i]).collect(),
                    owners: idx.iter().map(|&i| doc.owners[i]).collect(),
                    wlb: idx.iter().map(|&i| doc.wlb[i]).collect(),
                    slx: idx.iter().map(|&i| doc.slx[i]).collect(),
                    custom: idx.iter().map(|&i| doc.custom_cells[i]).collect(),
                }
            };
            p.changes.push(Change::Slabs {
                rect,
                before: block(before),
                after: block(after),
            });
        }
        let stl_diff = |x: usize, y: usize| {
            let i = d.stl_index(x, y);
            before.columns[i] != after.columns[i] || before.wibble[i] != after.wibble[i]
        };
        if let Some(rect) = bounding(d.stl_w() + 1, d.stl_h() + 1, stl_diff) {
            let block = |doc: &MapDoc| {
                let idx: Vec<usize> = rect.iter().map(|(x, y)| d.stl_index(x, y)).collect();
                StlBlock {
                    columns: idx.iter().map(|&i| doc.columns[i]).collect(),
                    wibble: idx.iter().map(|&i| doc.wibble[i]).collect(),
                }
            };
            p.changes.push(Change::Subtiles {
                rect,
                before: block(before),
                after: block(after),
            });
        }
        list_diff(
            &before.things.items,
            &after.things.items,
            &mut p.changes,
            |index, before, after| Change::Thing {
                index,
                before,
                after,
            },
        );
        list_diff(
            &before.action_points.items,
            &after.action_points.items,
            &mut p.changes,
            |index, before, after| Change::ActionPoint {
                index,
                before,
                after,
            },
        );
        list_diff(
            &before.lights.items,
            &after.lights.items,
            &mut p.changes,
            |index, before, after| Change::Light {
                index,
                before,
                after,
            },
        );
        let (pb, pa) = (props_of(before), props_of(after));
        if pb != pa {
            p.changes.push(Change::Props {
                before: pb,
                after: pa,
            });
        }
        p
    }
}

/// Bounding rectangle of the cells where `differs` holds.
fn bounding(w: usize, h: usize, differs: impl Fn(usize, usize) -> bool) -> Option<CellRect> {
    let mut r: Option<CellRect> = None;
    for y in 0..h {
        for x in 0..w {
            if differs(x, y) {
                r = Some(match r {
                    None => CellRect {
                        x0: x,
                        y0: y,
                        x1: x,
                        y1: y,
                    },
                    Some(r) => CellRect {
                        x0: r.x0.min(x),
                        y0: r.y0.min(y),
                        x1: r.x1.max(x),
                        y1: r.y1.max(y),
                    },
                });
            }
        }
    }
    r
}

/// Edits turning list `a` into `b`: common prefix and suffix kept, the middle replaced
/// index by index where both have an item, extra old items removed, extra new items inserted.
fn list_diff<T: Clone + PartialEq>(
    a: &[T],
    b: &[T],
    out: &mut Vec<Change>,
    mk: impl Fn(usize, Option<T>, Option<T>) -> Change,
) {
    let pre = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let max_suf = a.len().min(b.len()) - pre;
    let suf = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suf)
        .take_while(|(x, y)| x == y)
        .count();
    let am = &a[pre..a.len() - suf];
    let bm = &b[pre..b.len() - suf];
    let common = am.len().min(bm.len());
    for k in 0..common {
        if am[k] != bm[k] {
            out.push(mk(pre + k, Some(am[k].clone()), Some(bm[k].clone())));
        }
    }
    // remove the old tail of the middle (always at the same index), then insert the new one
    for k in common..am.len() {
        out.push(mk(pre + common, Some(am[k].clone()), None));
    }
    for (k, item) in bm.iter().enumerate().skip(common) {
        out.push(mk(pre + k, None, Some(item.clone())));
    }
}

impl MapDoc {
    /// Run `edit` and return its result with the patch that undoes / redoes it. Snapshots the
    /// whole document (clone) and diffs; always correct, whatever `edit` touches. See
    /// [`MapDoc::record_tracked`] for the cheap variant.
    pub fn record<R>(&mut self, edit: impl FnOnce(&mut MapDoc) -> R) -> (R, Patch) {
        let before = self.clone();
        let r = edit(self);
        (r, Patch::diff(&before, self))
    }

    /// Like [`MapDoc::record`], but without cloning the document: the mutation methods of
    /// [`MapDoc`] (slab / owner / `.wlb` / `.slx` setters, `place_slab*`, `regenerate_*`,
    /// `decorate`, the thing / action point / light operations, `apply_slab_objects`, resize and
    /// transforms, custom slabs, `edit_sidecar`) log the cells and lists they touch into a journal,
    /// and the patch is built from it. The patch is identical to the one [`MapDoc::record`] returns
    /// (same changes, same order: the bounding rectangle of the cells that really differ,
    /// [`list_diff`] of the lists that were touched).
    ///
    /// Rules: `edit` must change the document only through those methods, or declare direct
    /// writes first with [`MapDoc::touch_slab`] / [`MapDoc::touch_subtile`] /
    /// [`MapDoc::touch_things`] / [`MapDoc::touch_action_points`] / [`MapDoc::touch_lights`] /
    /// [`MapDoc::touch_props`] / [`MapDoc::touch_whole`] (before the write). A direct write that is
    /// not declared is missing from the patch; [`MapDoc::record_checked`] detects that. Calls nest.
    /// Operations that change the size, `.own` or `.flg` (`touch_whole`) fall back to one full
    /// clone, taken lazily.
    pub fn record_tracked<R>(&mut self, edit: impl FnOnce(&mut MapDoc) -> R) -> (R, Patch) {
        let outer = self.journal.take();
        self.journal = Some(Box::default());
        let r = edit(self);
        let j = self.journal.take().unwrap_or_default();
        let patch = j.build(self);
        self.journal = outer;
        if let Some(o) = self.journal.as_mut() {
            // the outer journal must know what the inner edit touched; the inner "before" values
            // are the outer ones for everything the outer edit had not touched yet
            let o_snapshot_needed = j.full.is_some() && o.full.is_none();
            if o_snapshot_needed {
                let mut start = (**j.full.as_ref().unwrap()).clone();
                o.revert_into(&mut start);
                o.full = Some(Box::new(start));
            }
            for (i, c) in j.slabs.iter() {
                o.slabs.entry(*i).or_insert(*c);
            }
            for (i, c) in j.stls.iter() {
                o.stls.entry(*i).or_insert(*c);
            }
            if o.things.is_none() {
                o.things = j.things.clone();
            }
            if o.aps.is_none() {
                o.aps = j.aps.clone();
            }
            if o.lights.is_none() {
                o.lights = j.lights.clone();
            }
            if o.props.is_none() {
                o.props = j.props.clone();
            }
        }
        (r, patch)
    }

    /// [`MapDoc::record_tracked`] that also runs the snapshot approach on a clone and panics when
    /// the patches or the resulting documents differ. For tests and debug builds.
    pub fn record_checked<R>(&mut self, edit: impl Fn(&mut MapDoc) -> R) -> (R, Patch) {
        let mut reference = self.clone();
        let (_, expected) = reference.record(&edit);
        let (r, got) = self.record_tracked(&edit);
        assert_eq!(
            got, expected,
            "tracked patch differs from the snapshot patch"
        );
        assert_eq!(
            DocState::of(self),
            DocState::of(&reference),
            "documents differ after the edit"
        );
        (r, got)
    }

    // ---- journal declarations (no-ops outside `record_tracked`)

    /// Declare a write to the slab layers (kind, owner, `.wlb`, `.slx`, custom id) of slab (x, y).
    pub fn touch_slab(&mut self, x: usize, y: usize) {
        if x < self.dims.w as usize && y < self.dims.h as usize {
            let i = self.dims.slab_index(x, y);
            self.j_slab(i);
        }
    }
    /// Declare a write to the column / wibble of a subtile of the `(3W+1) x (3H+1)` grid.
    pub fn touch_subtile(&mut self, x: usize, y: usize) {
        if x <= self.dims.stl_w() && y <= self.dims.stl_h() {
            let i = self.dims.stl_index(x, y);
            self.j_stl(i);
        }
    }
    /// Declare a write to `things.items`.
    pub fn touch_things(&mut self) {
        if let Some(j) = self.journal.as_mut() {
            if j.things.is_none() && j.full.is_none() {
                j.things = Some(self.things.items.clone());
            }
        }
    }
    /// Declare a write to `action_points.items`.
    pub fn touch_action_points(&mut self) {
        if let Some(j) = self.journal.as_mut() {
            if j.aps.is_none() && j.full.is_none() {
                j.aps = Some(self.action_points.items.clone());
            }
        }
    }
    /// Declare a write to `lights.items`.
    pub fn touch_lights(&mut self) {
        if let Some(j) = self.journal.as_mut() {
            if j.lights.is_none() && j.full.is_none() {
                j.lights = Some(self.lights.items.clone());
            }
        }
    }
    /// Declare a write to the map properties (`texture_id`, `inf_trailing`, `wlb_from_file`, the
    /// sidecar).
    pub fn touch_props(&mut self) {
        let need = self
            .journal
            .as_ref()
            .is_some_and(|j| j.props.is_none() && j.full.is_none());
        if need {
            let p = props_of(self);
            if let Some(j) = self.journal.as_mut() {
                j.props = Some(p);
            }
        }
    }
    /// Declare a change of the size, `.own` or `.flg`, or a rewrite of whole grids: from now on
    /// the patch is the diff against a snapshot of the state at the start of the edit.
    pub fn touch_whole(&mut self) {
        let Some(mut j) = self.journal.take() else {
            return;
        };
        if j.full.is_none() {
            let mut start = self.clone();
            j.revert_into(&mut start);
            j.full = Some(Box::new(start));
        }
        self.journal = Some(j);
    }

    pub(crate) fn j_slab(&mut self, i: usize) {
        if let Some(j) = self.journal.as_mut() {
            if j.full.is_none() && !j.slabs.contains_key(&i) {
                j.slabs.insert(
                    i,
                    (
                        self.kinds[i],
                        self.owners[i],
                        self.wlb[i],
                        self.slx[i],
                        self.custom_cells[i],
                    ),
                );
            }
        }
    }
    pub(crate) fn j_stl(&mut self, i: usize) {
        if let Some(j) = self.journal.as_mut() {
            if j.full.is_none() && !j.stls.contains_key(&i) {
                j.stls.insert(i, (self.columns[i], self.wibble[i]));
            }
        }
    }
}

/// Before-values of what an edit touched (see [`MapDoc::record_tracked`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Journal {
    /// slab index -> (kind, owner, wlb, slx, custom) before the edit
    slabs: HashMap<usize, (u16, u8, u8, u8, u16)>,
    /// subtile index -> (column, wibble) before the edit
    stls: HashMap<usize, (Column, u8)>,
    things: Option<Vec<FxThing>>,
    aps: Option<Vec<FxActionPoint>>,
    lights: Option<Vec<FxLight>>,
    props: Option<Props>,
    /// The whole document at the start of the edit, once a whole-document change was declared.
    full: Option<Box<MapDoc>>,
}

impl Journal {
    /// Put the logged before-values back into `doc` (same dims as at logging time).
    fn revert_into(&self, doc: &mut MapDoc) {
        for (&i, &(k, o, w, x, c)) in &self.slabs {
            doc.kinds[i] = k;
            doc.owners[i] = o;
            doc.wlb[i] = w;
            doc.slx[i] = x;
            doc.custom_cells[i] = c;
        }
        for (&i, &(c, w)) in &self.stls {
            doc.columns[i] = c;
            doc.wibble[i] = w;
        }
        if let Some(t) = &self.things {
            doc.things.items = t.clone();
        }
        if let Some(t) = &self.aps {
            doc.action_points.items = t.clone();
        }
        if let Some(t) = &self.lights {
            doc.lights.items = t.clone();
        }
        if let Some(p) = &self.props {
            set_props(doc, p);
        }
    }

    /// The patch [`Patch::diff`] would give between the start of the edit and `after`.
    fn build(&self, after: &MapDoc) -> Patch {
        if let Some(full) = &self.full {
            return Patch::diff(full, after);
        }
        let d = after.dims;
        let mut p = Patch::new();
        // slab layers: bounding rectangle of the cells that really changed
        let mut rect: Option<CellRect> = None;
        for (&i, &(k, o, w, x, c)) in &self.slabs {
            if (k, o, w, x, c)
                != (
                    after.kinds[i],
                    after.owners[i],
                    after.wlb[i],
                    after.slx[i],
                    after.custom_cells[i],
                )
            {
                rect = Some(grow(rect, i % d.w as usize, i / d.w as usize));
            }
        }
        if let Some(rect) = rect {
            let idx: Vec<usize> = rect.iter().map(|(x, y)| d.slab_index(x, y)).collect();
            let b = |i: usize| {
                self.slabs.get(&i).copied().unwrap_or((
                    after.kinds[i],
                    after.owners[i],
                    after.wlb[i],
                    after.slx[i],
                    after.custom_cells[i],
                ))
            };
            let before = SlabBlock {
                kinds: idx.iter().map(|&i| b(i).0).collect(),
                owners: idx.iter().map(|&i| b(i).1).collect(),
                wlb: idx.iter().map(|&i| b(i).2).collect(),
                slx: idx.iter().map(|&i| b(i).3).collect(),
                custom: idx.iter().map(|&i| b(i).4).collect(),
            };
            let aft = SlabBlock {
                kinds: idx.iter().map(|&i| after.kinds[i]).collect(),
                owners: idx.iter().map(|&i| after.owners[i]).collect(),
                wlb: idx.iter().map(|&i| after.wlb[i]).collect(),
                slx: idx.iter().map(|&i| after.slx[i]).collect(),
                custom: idx.iter().map(|&i| after.custom_cells[i]).collect(),
            };
            p.changes.push(Change::Slabs {
                rect,
                before,
                after: aft,
            });
        }
        let sw = d.stl_w() + 1;
        let mut rect: Option<CellRect> = None;
        for (&i, &(c, w)) in &self.stls {
            if c != after.columns[i] || w != after.wibble[i] {
                rect = Some(grow(rect, i % sw, i / sw));
            }
        }
        if let Some(rect) = rect {
            let idx: Vec<usize> = rect.iter().map(|(x, y)| d.stl_index(x, y)).collect();
            let b = |i: usize| {
                self.stls
                    .get(&i)
                    .copied()
                    .unwrap_or((after.columns[i], after.wibble[i]))
            };
            let before = StlBlock {
                columns: idx.iter().map(|&i| b(i).0).collect(),
                wibble: idx.iter().map(|&i| b(i).1).collect(),
            };
            let aft = StlBlock {
                columns: idx.iter().map(|&i| after.columns[i]).collect(),
                wibble: idx.iter().map(|&i| after.wibble[i]).collect(),
            };
            p.changes.push(Change::Subtiles {
                rect,
                before,
                after: aft,
            });
        }
        if let Some(b) = &self.things {
            list_diff(
                b,
                &after.things.items,
                &mut p.changes,
                |index, before, after| Change::Thing {
                    index,
                    before,
                    after,
                },
            );
        }
        if let Some(b) = &self.aps {
            list_diff(
                b,
                &after.action_points.items,
                &mut p.changes,
                |index, before, after| Change::ActionPoint {
                    index,
                    before,
                    after,
                },
            );
        }
        if let Some(b) = &self.lights {
            list_diff(
                b,
                &after.lights.items,
                &mut p.changes,
                |index, before, after| Change::Light {
                    index,
                    before,
                    after,
                },
            );
        }
        if let Some(pb) = &self.props {
            let pa = props_of(after);
            if *pb != pa {
                p.changes.push(Change::Props {
                    before: pb.clone(),
                    after: pa,
                });
            }
        }
        p
    }
}

fn grow(r: Option<CellRect>, x: usize, y: usize) -> CellRect {
    match r {
        None => CellRect {
            x0: x,
            y0: y,
            x1: x,
            y1: y,
        },
        Some(r) => CellRect {
            x0: r.x0.min(x),
            y0: r.y0.min(y),
            x1: r.x1.max(x),
            y1: r.y1.max(y),
        },
    }
}
