# Editor languages

The editor's own texts (menus, toolbar, panels, dialogs, hover texts, status messages, the
shortcut table) can be shown in another language. This is about the editor itself; translating
the texts of a level for the game is the [Translations](translations.md) grid.

The editor ships in English only. Other languages come from catalogue files; a text that a
catalogue does not have stays English, so a partial catalogue is usable.

## Choosing the language

Configuration > Preferences… > Interface > **Language**. The list holds English and every
catalogue found. The choice applies at once and is kept in the settings (`ui.language`); it is
applied again at the next start.

Texts already made before the switch keep their language until they are made again: the undo
history names ("Paint Rock", "Resize map"), the last status message and the hover texts of the
palette tiles (rebuilt when a map is opened).

## Where catalogues are read

Files `<code>.toml` (for example `fr.toml`) in these folders, in this order; a later folder replaces
a language of an earlier one:

1. `data/i18n/` next to the editor's executable;
2. the source tree's `data/i18n/` (when the editor is run from a build of the repository);
3. `i18n/` in the editor's settings folder (the folder shown under Preferences > Files, for
   example `~/.config/kfx-editor/i18n/` on Linux).

## Writing a catalogue

`data/i18n/en.toml` is the template: every text of the editor with its key and where it is used.
A catalogue only needs a `[meta]` block and the keys it translates:

```toml
[meta]
language = "fr"
name = "Français"

[strings]
"kfx_editor.file" = "Fichier"
"kfx_editor.saved_path" = "Enregistré : {path}"
"kfx_editor.n_mods" = { one = "{n} mod", other = "{n} mods" }
"kfx_editor.action.file.open" = "Ouvrir une carte"
```

* `{name}` placeholders are filled in by the editor: keep the same names as the English text (you
  may move them, not rename or drop them).
* A key with `one` / `other` in `en.toml` is a plural: `one` is used for exactly 1, `other` for
  every other number.
* Keys `kfx_editor.action.<id>` and `kfx_editor.action_group.<group>` are the labels and groups of
  the keyboard shortcut table (Help > Controls, Preferences > Shortcuts).
* Entries with `draft = true` in `en.toml` are texts of other parts (the script, configuration,
  campaign, Events, prefab and Workshop panels) that are not translatable yet; translating them
  has no effect for now.
* Window positions are remembered per window title, so a window may open at its default place
  the first time in a new language.

## For developers

The English text stays in the code next to a stable key: `trd("kfx_editor.save", "Save")`,
`trf("kfx_editor.saved_path", "Saved {path}", &[("path", &p.display())])`,
`trn(key, n, "{n} file", "{n} files", &[])`, and `tx(key, "English")` in `const` tables. English
output is exactly the literal. After adding or changing a text, regenerate the template:

```sh
cargo run -p kfx-prefab --example extract_strings
```

A test of the editor fails when `en.toml` is out of date, a key has
two different texts, or the arguments of a call do not match its placeholders. When the meaning of
a text changes, give it a new key so that old translations are not shown for it. The runtime and
the scanner are documented in `kfx_prefab::i18n`.
