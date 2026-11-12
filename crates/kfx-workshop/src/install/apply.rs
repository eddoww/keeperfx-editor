//! Applying an [`InstallPlan`] transactionally, and uninstalling from its [`Manifest`].
use super::*;

fn missing_dirs(install: &Path, rel_dir: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut base = install.to_path_buf();
    for comp in rel_dir.split('/').filter(|c| !c.is_empty()) {
        cur = if cur.is_empty() {
            comp.to_string()
        } else {
            format!("{cur}/{comp}")
        };
        base.push(comp);
        if !base.exists() {
            out.push(cur.clone());
        }
    }
    out
}

fn build_repack(
    entries: &[String],
    staged: &HashMap<String, PathBuf>,
    strip: &str,
) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for e in entries {
            let name = e.strip_prefix(strip).unwrap_or(e);
            let bytes = std::fs::read(
                staged
                    .get(e)
                    .ok_or_else(|| Error::Plan(format!("{e} was not extracted")))?,
            )
            .map_err(|x| Error::io(e, x))?;
            zw.start_file(name, opts)
                .map_err(|x| Error::Other(x.to_string()))?;
            zw.write_all(&bytes)
                .map_err(|x| Error::Other(x.to_string()))?;
        }
        zw.finish().map_err(|x| Error::Other(x.to_string()))?;
    }
    Ok(buf.into_inner())
}

/// Apply a plan. Either everything in the plan is installed (and a manifest is saved in `store`)
/// or the install folder is as it was.
pub fn apply(
    install: &Path,
    plan: &InstallPlan,
    store: &Store,
    limits: &Limits,
) -> Result<Manifest> {
    if plan.is_blocked() {
        return Err(Error::Plan(plan.blocked.join("; ")));
    }
    let id = store.fresh_id(&plan.name);
    let staging = store.staging_dir(&id);
    let backups = store.backup_dir(&id);
    let result = apply_inner(install, plan, store, limits, &id, &staging, &backups);
    let _ = std::fs::remove_dir_all(&staging);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&backups);
    }
    result
}

fn apply_inner(
    install: &Path,
    plan: &InstallPlan,
    store: &Store,
    limits: &Limits,
    id: &str,
    staging: &Path,
    backups: &Path,
) -> Result<Manifest> {
    // 1. re-validate against the disk: a plan is a promise about the state it saw
    for i in plan.applied_items() {
        let (_, exists) = resolve_ci(install, &i.dest);
        match (i.action, exists) {
            (Action::Create, true) => {
                return Err(Error::Plan(format!(
                    "{} appeared since the plan was made; plan again",
                    i.dest
                )))
            }
            (Action::Overwrite, false) | (Action::Create, false) | (Action::Overwrite, true) => {}
            _ => {}
        }
    }
    // 2. extract what is needed to staging (bounded)
    std::fs::create_dir_all(staging).map_err(|e| Error::io(staging, e))?;
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for i in plan.applied_items() {
        match &i.source {
            Source::Entry(e) => {
                needed.insert(e.clone());
            }
            Source::Repack(v) => needed.extend(v.iter().cloned()),
            Source::LifRenumber { entry, .. } => {
                needed.insert(entry.clone());
            }
            _ => {}
        }
    }
    let mut staged: HashMap<String, PathBuf> = HashMap::new();
    if !needed.is_empty() {
        let mut n = 0usize;
        archive::read_entries(
            &plan.archive,
            &|p| needed.contains(p),
            limits,
            &mut |name, bytes| {
                n += 1;
                let f = staging.join(format!("{n}.bin"));
                std::fs::write(&f, bytes).map_err(|e| Error::io(&f, e))?;
                staged.insert(name.to_string(), f);
                Ok(())
            },
        )?;
    }
    // 3. group by destination folder and commit one transaction per folder
    let mut m = Manifest {
        version: 1,
        id: id.to_string(),
        name: plan.name.clone(),
        kind: plan.kind.label().to_string(),
        install_root: install.display().to_string(),
        item: plan.item.clone(),
        archive_sha256: plan.archive_sha256.clone(),
        installed_at: now_secs(),
        files: vec![],
        dirs: vec![],
        load_order: None,
        levels: vec![],
        campaigns: plan.campaigns.clone(),
    };
    let mut by_dir: BTreeMap<String, Vec<&PlanItem>> = BTreeMap::new();
    for i in plan.applied_items() {
        by_dir
            .entry(split_dir(&i.dest).0.to_string())
            .or_default()
            .push(i);
    }
    let mut backup_n = 0usize;
    for (dir, items) in by_dir {
        let abs = install.join(&dir);
        let mut tx = SaveTransaction::new(&abs);
        let mut recs: Vec<InstalledFile> = vec![];
        for it in items {
            let name = split_dir(&it.dest).1.to_string();
            let bytes: Vec<u8> = match &it.source {
                Source::Entry(e) => std::fs::read(
                    staged
                        .get(e)
                        .ok_or_else(|| Error::Plan(format!("{e} was not extracted")))?,
                )
                .map_err(|x| Error::io(e, x))?,
                Source::Raw => {
                    std::fs::read(&plan.archive).map_err(|x| Error::io(&plan.archive, x))?
                }
                Source::Generated(b) => b.clone(),
                Source::Repack(v) => build_repack(v, &staged, &plan.strip_prefix)?,
                Source::LifRenumber { entry, from, to } => {
                    let raw = std::fs::read(
                        staged
                            .get(entry)
                            .ok_or_else(|| Error::Plan(format!("{entry} was not extracted")))?,
                    )
                    .map_err(|x| Error::io(entry, x))?;
                    let mut lif = kfx_formats::levelinfo::Lif::parse(&raw);
                    let nm = lif
                        .entries()
                        .iter()
                        .find(|e| e.number == *from as i32)
                        .map(|e| e.name.clone());
                    if let Some(nm) = nm {
                        lif.remove_entry(*from as i32);
                        lif.set_entry(*to as i32, &nm);
                    }
                    lif.to_bytes()
                }
            };
            let mut backup = None;
            if it.action == Action::Overwrite {
                let old = abs.join(&name);
                if old.is_file() {
                    backup_n += 1;
                    let rel = format!("files/{backup_n}");
                    let bp = backups.join(&rel);
                    std::fs::create_dir_all(bp.parent().unwrap_or(backups))
                        .map_err(|e| Error::io(&bp, e))?;
                    std::fs::copy(&old, &bp).map_err(|e| Error::io(&old, e))?;
                    backup = Some(rel);
                }
            }
            recs.push(InstalledFile {
                path: it.dest.clone(),
                size: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
                backup,
            });
            tx.write(name, bytes);
        }
        let created = missing_dirs(install, &dir);
        match tx.commit() {
            Ok(_) => {
                for d in created {
                    if !m.dirs.contains(&d) {
                        m.dirs.push(d);
                    }
                }
                m.files.extend(recs);
            }
            Err(e) => {
                // commit() restored this folder; undo the folders committed before it
                let _ = uninstall_inner(install, &m, store, true);
                return Err(e.into());
            }
        }
    }
    // 4. load_order.cfg
    let to_add: Vec<&LoadOrderEntry> = plan
        .load_order
        .iter()
        .filter(|e| !e.already_listed)
        .collect();
    if !to_add.is_empty() {
        let (lo, exists) = resolve_ci(install, LOAD_ORDER);
        let abs = install.join(&lo);
        let old = if exists {
            Some(std::fs::read(&abs).map_err(|e| Error::io(&abs, e))?)
        } else {
            None
        };
        let mut text = old
            .as_ref()
            .map(|b| kfx_formats::cfg::latin1_decode(b))
            .unwrap_or_else(fresh_load_order);
        let mut added = vec![];
        for e in &to_add {
            text = load_order_add(&text, &e.stage, &e.name);
            added.push((e.stage.clone(), e.name.clone()));
        }
        let bytes = kfx_formats::cfg::latin1_encode(&text);
        let mut backup = None;
        if let Some(o) = &old {
            let bp = backups.join("load_order.cfg");
            std::fs::create_dir_all(backups).map_err(|e| Error::io(backups, e))?;
            std::fs::write(&bp, o).map_err(|e| Error::io(&bp, e))?;
            backup = Some("load_order.cfg".to_string());
        }
        let created = missing_dirs(install, "mods");
        let mut tx = SaveTransaction::new(&install.join("mods"));
        tx.write(split_dir(&lo).1.to_string(), bytes.clone());
        if let Err(e) = tx.commit() {
            let _ = uninstall_inner(install, &m, store, true);
            return Err(e.into());
        }
        for d in created {
            if !m.dirs.contains(&d) {
                m.dirs.push(d);
            }
        }
        m.load_order = Some(LoadOrderRecord {
            existed_before: old.is_some(),
            backup,
            post_sha256: sha256_hex(&bytes),
            added,
        });
    }
    for (dir, number, name) in &plan.levels {
        let renumbered_from = plan
            .renumbered
            .iter()
            .find(|r| r.to == *number && r.dir.eq_ignore_ascii_case(dir))
            .map(|r| r.from);
        m.levels.push(InstalledLevel {
            dir: dir.clone(),
            number: *number,
            name: name.clone(),
            renumbered_from,
        });
    }
    // 5. the manifest, last
    if let Err(e) = store.save(&m) {
        let _ = uninstall_inner(install, &m, store, true);
        return Err(e);
    }
    Ok(m)
}

// ---------------------------------------------------------------- uninstall

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UninstallReport {
    pub removed: Vec<String>,
    pub restored: Vec<String>,
    /// Changed since the install, left in place.
    pub kept_modified: Vec<String>,
    pub removed_dirs: Vec<String>,
    pub load_order_restored: bool,
    pub load_order_lines_removed: Vec<(String, String)>,
}

/// Remove an install and restore what it replaced. Files the user modified after the install
/// are kept unless `force`.
pub fn uninstall(install: &Path, id: &str, store: &Store, force: bool) -> Result<UninstallReport> {
    let m = store.load(id)?;
    let rep = uninstall_inner(install, &m, store, force)?;
    if rep.kept_modified.is_empty() || force {
        store.remove(id);
    } else {
        // keep the record (minus what is gone) so the user can retry with force
        let mut m2 = m.clone();
        m2.files.retain(|f| rep.kept_modified.contains(&f.path));
        m2.dirs.clear();
        m2.load_order = None;
        store.save(&m2)?;
    }
    Ok(rep)
}

fn uninstall_inner(
    install: &Path,
    m: &Manifest,
    store: &Store,
    force: bool,
) -> Result<UninstallReport> {
    let mut rep = UninstallReport::default();
    let backups = store.backup_dir(&m.id);
    for f in m.files.iter().rev() {
        let path = install.join(&f.path);
        let cur = std::fs::read(&path).ok();
        match cur {
            None => {
                // already gone; restore the backup if there is one
                if let Some(b) = &f.backup {
                    if let Ok(old) = std::fs::read(backups.join(b)) {
                        atomic_write(&path, &old).map_err(|e| Error::io(&path, e))?;
                        rep.restored.push(f.path.clone());
                    }
                }
            }
            Some(bytes) => {
                if !force && sha256_hex(&bytes) != f.sha256 {
                    rep.kept_modified.push(f.path.clone());
                    continue;
                }
                match &f.backup {
                    Some(b) => {
                        let old = std::fs::read(backups.join(b))
                            .map_err(|e| Error::io(backups.join(b), e))?;
                        atomic_write(&path, &old).map_err(|e| Error::io(&path, e))?;
                        rep.restored.push(f.path.clone());
                    }
                    None => {
                        std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                        rep.removed.push(f.path.clone());
                    }
                }
            }
        }
    }
    if let Some(lo) = &m.load_order {
        let (rel, exists) = resolve_ci(install, LOAD_ORDER);
        let path = install.join(&rel);
        if exists {
            let cur = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            if sha256_hex(&cur) == lo.post_sha256 {
                // untouched since: restore byte for byte
                match &lo.backup {
                    Some(b) => {
                        let old = std::fs::read(backups.join(b))
                            .map_err(|e| Error::io(backups.join(b), e))?;
                        atomic_write(&path, &old).map_err(|e| Error::io(&path, e))?;
                    }
                    None => {
                        std::fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
                    }
                }
                rep.load_order_restored = true;
            } else {
                // edited since: only take our lines out
                let mut text = kfx_formats::cfg::latin1_decode(&cur);
                for (stage, name) in &lo.added {
                    text = load_order_remove(&text, stage, name);
                    rep.load_order_lines_removed
                        .push((stage.clone(), name.clone()));
                }
                atomic_write(&path, &kfx_formats::cfg::latin1_encode(&text))
                    .map_err(|e| Error::io(&path, e))?;
            }
        }
    }
    for d in m.dirs.iter().rev() {
        let p = install.join(d);
        if std::fs::remove_dir(&p).is_ok() {
            rep.removed_dirs.push(d.clone());
        }
    }
    Ok(rep)
}
