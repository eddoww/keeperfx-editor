//! Grid data overlay: the raw value of one of the map's grids written over every slab or
//! subtile of the view (View > Grid data). The values are the ones
//! the editor saves: slab kind (`.slb`), owner (`.own`), `.wlb`, `.slx` style, the `.dat` column
//! index (the index the column gets in the saved `.clm`), wibble (`.wib`), floor height and the
//! loaded `.flg` flags, and the custom slab id of the sidecar.
use kfx_map::column::ColumnPool;
use kfx_map::mapdoc::MapDoc;
use kfx_prefab::i18n::{tx, Tx};
use serde::{Deserialize, Serialize};

/// Which grid the overlay shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GridData {
    #[default]
    Off,
    /// `.slb` slab kind id.
    Kind,
    /// Slab owner (0-8).
    Owner,
    /// `.wlb` (water / lava bridge bits).
    Wlb,
    /// `.slx` texture style (0 = the map's).
    Style,
    /// `.dat` column index per subtile, as saved.
    Column,
    /// `.wib` wibble per subtile.
    Wibble,
    /// Floor height in cubes per subtile.
    Floor,
    /// `.flg` flags per subtile (hex; only when the map has a `.flg`).
    Flags,
    /// Custom slab id per slab (`-` for a normal slab).
    Custom,
}

impl GridData {
    pub const ALL: [GridData; 10] = [
        GridData::Off,
        GridData::Kind,
        GridData::Owner,
        GridData::Wlb,
        GridData::Style,
        GridData::Custom,
        GridData::Column,
        GridData::Wibble,
        GridData::Floor,
        GridData::Flags,
    ];

    pub fn label(self) -> Tx {
        match self {
            GridData::Off => tx("kfx_editor.off", "Off"),
            GridData::Kind => tx("kfx_editor.slab_kind_id_slb", "Slab kind ID (.slb)"),
            GridData::Owner => tx("kfx_editor.owner_own", "Owner (.own)"),
            GridData::Wlb => tx(
                "kfx_editor.bridge_liquid_bits_wlb",
                "Bridge/liquid bits (.wlb)",
            ),
            GridData::Style => tx("kfx_editor.texture_style_slx", "Texture style (.slx)"),
            GridData::Column => tx("kfx_editor.column_index_dat", "Column index (.dat)"),
            GridData::Wibble => tx("kfx_editor.wibble_wib", "Wibble (.wib)"),
            GridData::Floor => tx("kfx_editor.floor_height", "Floor height"),
            GridData::Flags => tx("kfx_editor.flags_flg", "Flags (.flg)"),
            GridData::Custom => tx("kfx_editor.custom_slab_id", "Custom slab ID"),
        }
    }

    pub fn parse(s: &str) -> Option<GridData> {
        Some(match s.to_ascii_lowercase().as_str() {
            "off" | "none" => GridData::Off,
            "kind" | "slb" => GridData::Kind,
            "owner" | "own" => GridData::Owner,
            "wlb" => GridData::Wlb,
            "style" | "slx" => GridData::Style,
            "column" | "dat" => GridData::Column,
            "wibble" | "wib" => GridData::Wibble,
            "floor" => GridData::Floor,
            "flags" | "flg" => GridData::Flags,
            "custom" | "fake" => GridData::Custom,
            _ => return None,
        })
    }

    /// Per subtile (else per slab).
    pub fn per_subtile(self) -> bool {
        matches!(
            self,
            GridData::Column | GridData::Wibble | GridData::Floor | GridData::Flags
        )
    }
}

/// `.dat` column indices of the current grid, as [`ColumnPool::build`] assigns them on save
/// (`None` when the pool overflows).
pub fn column_indices(doc: &MapDoc) -> Option<Vec<u16>> {
    let mut pool = ColumnPool::new();
    doc.columns
        .iter()
        .take(doc.dims.stl_grid())
        .map(|c| pool.intern(c).ok())
        .collect()
}

/// The text of cell (x, y) (slab or subtile coordinates); `cols` from [`column_indices`].
pub fn value(
    doc: &MapDoc,
    which: GridData,
    x: usize,
    y: usize,
    cols: Option<&[u16]>,
) -> Option<String> {
    let d = doc.dims;
    if which.per_subtile() {
        if x > d.stl_w() || y > d.stl_h() {
            return None;
        }
        let i = d.stl_index(x, y);
        return Some(match which {
            GridData::Column => cols?.get(i)?.to_string(),
            GridData::Wibble => doc.wibble.get(i)?.to_string(),
            GridData::Floor => doc.floor_height(x as i32, y as i32).to_string(),
            GridData::Flags => format!("{:x}", doc.flg.as_ref()?.raw.get(i)?),
            _ => return None,
        });
    }
    if x >= d.w as usize || y >= d.h as usize {
        return None;
    }
    let i = y * d.w as usize + x;
    Some(match which {
        GridData::Kind => doc.kinds.get(i)?.to_string(),
        GridData::Owner => doc.owners.get(i)?.to_string(),
        GridData::Wlb => doc.wlb.get(i)?.to_string(),
        GridData::Style => doc.slx.get(i)?.to_string(),
        GridData::Custom => match *doc.custom_cells.get(i)? {
            0 => "-".to_string(),
            id => id.to_string(),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_follow_the_grids() {
        let Some((_, d)) = crate::document::tests::load_shipped(1) else {
            return;
        };
        let doc = &d.doc;
        let cols = column_indices(doc).expect("pool");
        assert_eq!(cols.len(), doc.dims.stl_grid());
        let i = 10 * doc.dims.w as usize + 12;
        assert_eq!(
            value(doc, GridData::Kind, 12, 10, None),
            Some(doc.kinds[i].to_string())
        );
        assert_eq!(
            value(doc, GridData::Owner, 12, 10, None),
            Some(doc.owners[i].to_string())
        );
        assert_eq!(
            value(doc, GridData::Column, 4, 5, Some(&cols)),
            Some(cols[doc.dims.stl_index(4, 5)].to_string())
        );
        assert_eq!(
            value(doc, GridData::Kind, doc.dims.w as usize, 0, None),
            None
        );
        // equal columns share an index; index 0 is never handed out to a used column twice
        let a = doc.dims.stl_index(0, 0);
        let same = (0..doc.dims.stl_grid()).find(|&j| j != a && doc.columns[j] == doc.columns[a]);
        if let Some(j) = same {
            assert_eq!(cols[a], cols[j]);
        }
        assert!(GridData::Floor.per_subtile() && !GridData::Kind.per_subtile());
    }
}
