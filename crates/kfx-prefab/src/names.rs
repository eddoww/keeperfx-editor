//! Name <-> number tables of a [`GameData`] (what prefabs use instead of numbers).
use crate::model::ThingClass;
use kfx_map::gamedata::GameData;
use std::collections::HashMap;

/// Model names of every thing class and slab names, with case-insensitive lookup.
#[derive(Clone, Debug, Default)]
pub struct Names {
    by_class: HashMap<ThingClass, (Vec<(i64, String)>, HashMap<String, i64>)>,
    slabs: HashMap<String, u16>,
    slab_list: Vec<String>,
}

impl Names {
    pub fn new(gd: &GameData) -> Names {
        let mut n = Names::default();
        let mut put = |c: ThingClass, list: Vec<(i64, String)>| {
            let map = list
                .iter()
                .map(|(i, s)| (s.to_ascii_uppercase(), *i))
                .collect();
            n.by_class.insert(c, (list, map));
        };
        put(
            ThingClass::Object,
            gd.object_names()
                .into_iter()
                .map(|(i, s)| (i as i64, s))
                .collect(),
        );
        put(
            ThingClass::Creature,
            gd.creatures
                .iter()
                .enumerate()
                .map(|(i, s)| (i as i64 + 1, s.clone()))
                .collect(),
        );
        put(
            ThingClass::EffectGen,
            gd.effect_generator_names()
                .into_iter()
                .map(|(i, s)| (i as i64, s))
                .collect(),
        );
        put(
            ThingClass::Trap,
            gd.trap_names()
                .into_iter()
                .map(|(i, s)| (i as i64, s))
                .collect(),
        );
        put(
            ThingClass::Door,
            gd.door_names()
                .into_iter()
                .map(|(i, s)| (i as i64, s))
                .collect(),
        );
        for i in 0..gd.slab_count() {
            let name = gd.slab_name(i).unwrap_or("").to_string();
            n.slabs.entry(name.to_ascii_uppercase()).or_insert(i as u16);
            n.slab_list.push(name);
        }
        n
    }

    /// The name of a model, `#N` when it has none.
    pub fn model_name(&self, class: ThingClass, model: i64) -> String {
        self.by_class
            .get(&class)
            .and_then(|(l, _)| l.iter().find(|(i, n)| *i == model && !n.is_empty()))
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{model}"))
    }

    /// The model of a name; `#N` is taken as the number N.
    pub fn model_id(&self, class: ThingClass, name: &str) -> Option<i64> {
        if let Some(n) = name.strip_prefix('#') {
            return n.parse().ok();
        }
        self.by_class
            .get(&class)?
            .1
            .get(&name.to_ascii_uppercase())
            .copied()
    }

    /// Whether the configuration defines the model name.
    pub fn has_model(&self, class: ThingClass, name: &str) -> bool {
        match name.strip_prefix('#') {
            // a numbered model counts when the number is in the list
            Some(n) => n.parse::<i64>().is_ok_and(|n| {
                self.by_class
                    .get(&class)
                    .is_some_and(|(l, _)| l.iter().any(|(i, _)| *i == n))
            }),
            None => self.model_id(class, name).is_some(),
        }
    }

    pub fn slab_id(&self, name: &str) -> Option<u16> {
        if let Some(n) = name.strip_prefix('#') {
            return n.parse().ok();
        }
        self.slabs.get(&name.to_ascii_uppercase()).copied()
    }

    pub fn slab_name(&self, id: u16) -> String {
        match self.slab_list.get(id as usize) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => format!("#{id}"),
        }
    }

    pub fn slab_count(&self) -> usize {
        self.slab_list.len()
    }

    /// All model names of a class (for substitution pickers).
    pub fn model_names(&self, class: ThingClass) -> Vec<String> {
        self.by_class
            .get(&class)
            .map(|(l, _)| {
                l.iter()
                    .map(|(_, n)| n.clone())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn slab_names(&self) -> &[String] {
        &self.slab_list
    }
}
