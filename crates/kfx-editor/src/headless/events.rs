//! `--actions` keys of the Events editor (see docs/manual/cli.md, "Events editor").
use crate::app::logic::{view_by_name, WaveSpec};
use crate::app::EditorApp;
use serde_json::{Map, Value};

/// A JSON value as script argument text (`1`, `"PLAYER0"` -> `PLAYER0`).
fn arg(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The spawn location: `{"hero_gate": n}`, `{"action_point": n}`, `{"player": "PLAYER0"}` or
/// the location text (`-1`, `3`, `PLAYER0`).
fn location(v: &Value) -> Result<String, String> {
    if let Some(o) = v.as_object() {
        if let Some(n) = o.get("hero_gate").and_then(Value::as_i64) {
            return Ok((-n.abs()).to_string());
        }
        if let Some(n) = o.get("action_point").and_then(Value::as_i64) {
            return Ok(n.abs().to_string());
        }
        if let Some(p) = o.get("player").and_then(Value::as_str) {
            return Ok(p.to_ascii_uppercase());
        }
        return Err(format!("spawn {v}: use hero_gate, action_point or player"));
    }
    Ok(arg(v))
}

/// Parse the `events_add_wave` object.
pub fn wave_spec(v: &Value) -> Result<WaveSpec, String> {
    let o = v.as_object().ok_or("events_add_wave needs an object")?;
    let mut w = WaveSpec::default();
    let num = |k: &str| {
        o.get(k)
            .and_then(Value::as_u64)
            .map(|n| n.min(u32::MAX as u64) as u32)
    };
    if let Some(p) = o.get("party").and_then(Value::as_str) {
        w.party = Some(p.to_string());
    }
    if let Some(list) = o.get("creatures") {
        let list = list.as_array().ok_or("creatures: a list")?;
        for c in list {
            // "KNIGHT", ["KNIGHT", 4], ["KNIGHT", 4, 300] or
            // {"creature": "KNIGHT", "level": 4, "gold": 300}
            let (name, level, gold) = match c {
                Value::String(s) => (s.clone(), 1, 0),
                Value::Array(a) => (
                    a.first()
                        .and_then(Value::as_str)
                        .ok_or("creatures: [name, level, gold]")?
                        .to_string(),
                    a.get(1).and_then(Value::as_u64).unwrap_or(1) as u32,
                    a.get(2).and_then(Value::as_u64).unwrap_or(0) as u32,
                ),
                Value::Object(m) => (
                    m.get("creature")
                        .and_then(Value::as_str)
                        .ok_or("creatures: {creature, level, gold}")?
                        .to_string(),
                    m.get("level").and_then(Value::as_u64).unwrap_or(1) as u32,
                    m.get("gold").and_then(Value::as_u64).unwrap_or(0) as u32,
                ),
                _ => return Err(format!("creatures: bad entry {c}")),
            };
            w.creatures.push((name, level, gold.min(100_000)));
        }
    }
    if let Some(s) = o.get("objective").and_then(Value::as_str) {
        w.objective = s.to_ascii_uppercase();
    }
    if let Some(s) = o.get("owner").and_then(Value::as_str) {
        w.owner = s.to_ascii_uppercase();
    }
    if let Some(v) = o.get("spawn") {
        w.spawn = location(v)?;
    }
    w.turn = num("turn");
    if let Some(c) = o.get("condition") {
        let c = c
            .as_object()
            .ok_or("condition: {player, variable, op, value}")?;
        let g = |k: &str, d: &str| c.get(k).map(arg).unwrap_or_else(|| d.to_string());
        w.condition = Some((
            g("player", "PLAYER0").to_ascii_uppercase(),
            g("variable", "FLAG0").to_ascii_uppercase(),
            g("op", "=="),
            g("value", "1"),
        ));
    }
    if w.turn.is_none() && w.condition.is_none() {
        return Err("events_add_wave needs a turn or a condition".into());
    }
    if let Some(s) = o.get("clock_player").and_then(Value::as_str) {
        w.clock_player = s.to_ascii_uppercase();
    }
    w.copies = num("copies").unwrap_or(1);
    w.count = num("count").unwrap_or(1);
    w.interval = num("interval").unwrap_or(3000);
    w.copies_step = num("copies_step").unwrap_or(0);
    if let Some(t) = o.get("tunneller") {
        let t = t.as_object().ok_or("tunneller: {head_for, target}")?;
        w.tunneller = Some((
            t.get("head_for")
                .map(arg)
                .unwrap_or_else(|| "DUNGEON_HEART".into())
                .to_ascii_uppercase(),
            t.get("target").map(arg).unwrap_or_else(|| "0".into()),
        ));
    }
    Ok(w)
}

/// The Events editor actions of one `--actions` entry.
pub fn run_logic(app: &mut EditorApp, obj: &Map<String, Value>) -> Result<(), String> {
    let get = |k: &str| obj.get(k);
    if let Some(v) = get("events").and_then(Value::as_str) {
        match v {
            "open" | "show" => app.open_events(),
            "text" => app.events_to_text(),
            "toggle" => app.toggle_events(),
            x => return Err(format!("events: unknown mode {x} (open, text, toggle)")),
        }
    }
    if let Some(v) = get("events_view").and_then(Value::as_str) {
        let view = view_by_name(v).ok_or_else(|| {
            format!(
                "events_view: unknown view {v} (cards, graph, timeline, parties, variables, lua)"
            )
        })?;
        app.events_set_view(view);
    }
    if let Some(v) = get("events_add_wave") {
        let w = wave_spec(v)?;
        let msg = app.events_add_wave(&w)?;
        eprintln!("events: {msg}");
    }
    if let Some(v) = get("events_select") {
        let summary = match v {
            Value::Number(n) => app.events_select(n.as_u64().map(|n| n as usize), None)?,
            Value::String(s) => app.events_select(None, Some(s))?,
            _ => return Err("events_select: an event number (1-based) or a text".into()),
        };
        eprintln!("events: selected {summary}");
    }
    if let Some(n) = get("events_undo").and_then(Value::as_u64) {
        app.events_edit(|ed| (0..n).for_each(|_| ed.undo()));
    }
    if let Some(n) = get("events_redo").and_then(Value::as_u64) {
        app.events_edit(|ed| (0..n).for_each(|_| ed.redo()));
    }
    if let Some(v) = get("events_party").and_then(Value::as_str) {
        let name = v.to_ascii_uppercase();
        if !app.events_editor().is_some_and(|ed| ed.show_party(&name)) {
            return Err(format!("events_party: no party {name}"));
        }
    }
    if get("events_print").is_some() {
        eprintln!("{}", app.events_report());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::wave_spec;

    #[test]
    fn wave_json() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"party": "raiders", "creatures": [["KNIGHT", 4, 300], "ARCHER", {"creature": "WIZARD", "level": 2}],
                "owner": "PLAYER_GOOD", "spawn": {"hero_gate": 2}, "turn": 1200, "copies": 2}"#,
        )
        .unwrap();
        let w = wave_spec(&v).unwrap();
        assert_eq!(w.party.as_deref(), Some("raiders"));
        assert_eq!(
            w.creatures,
            vec![
                ("KNIGHT".into(), 4, 300),
                ("ARCHER".into(), 1, 0),
                ("WIZARD".into(), 2, 0)
            ]
        );
        assert_eq!(
            (w.spawn.as_str(), w.turn, w.copies, w.count),
            ("-2", Some(1200), 2, 1)
        );
        let v: serde_json::Value = serde_json::from_str(r#"{"party": "P", "spawn": {"action_point": 3}, "condition": {"player": "player0", "variable": "flag2", "op": ">=", "value": 1}}"#).unwrap();
        let w = wave_spec(&v).unwrap();
        assert_eq!(w.spawn, "3");
        assert_eq!(
            w.condition,
            Some(("PLAYER0".into(), "FLAG2".into(), ">=".into(), "1".into()))
        );
        assert!(
            wave_spec(&serde_json::json!({"party": "P"})).is_err(),
            "a turn or a condition is required"
        );
    }
}
