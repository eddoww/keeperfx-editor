//! The command line started through the windowed executable on Windows.
//!
//! `kfx-editor.exe` is built for the GUI subsystem, so Windows opens no console window next to
//! the editor; the price is that it has no console to print to, even when started from a
//! terminal (which also does not wait for it). When it is given a command (`validate`,
//! `--help`, `update --check`, ...), it runs the same command through `kfx-editor-cli.exe`
//! beside it, hidden, writes the output to `cli-output.log` in the configuration folder, and
//! shows the output and the log's location in a small window. Use `kfx-editor-cli.exe` in
//! terminals and scripts.
//!
//! The logic is compiled and tested on every platform; only the hidden start is Windows-only.

use kfx_prefab::i18n::{trd, trf};
use std::path::{Path, PathBuf};

/// The arguments ask for the command line rather than the window: a command, an option, or
/// anything that is not a single map to open.
pub fn wants_console(args: &[String]) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    first.starts_with('-')
        || first == "update"
        || crate::cli::COMMANDS.contains(&first.as_str())
        || crate::window_map_arg(args).is_err()
}

/// `kfx-editor-cli[.exe]` beside `gui_exe`.
pub fn cli_exe_beside(gui_exe: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "kfx-editor-cli.exe"
    } else {
        "kfx-editor-cli"
    };
    gui_exe.with_file_name(name)
}

/// Run `program args` without a console window, with standard output and error captured.
/// Returns the exit code and the output (standard output, then standard error).
pub fn capture(program: &Path, args: &[String]) -> Result<(i32, String), String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a console program started without a console window
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((out.status.code().unwrap_or(1), text))
}

/// The log file of a run, and the text written to it.
pub fn log_text(args: &[String], code: i32, output: &str) -> String {
    format!(
        "kfx-editor {} {}\nexit code {code}\n\n{output}\n",
        crate::VERSION,
        args.join(" ")
    )
}

/// The command line through the GUI executable (see the module documentation). Returns the
/// exit code.
pub fn run_for_gui(args: &[String]) -> i32 {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kfx-editor.exe"));
    let cli = cli_exe_beside(&exe);
    let (code, output) = if cli.is_file() {
        capture(&cli, args).unwrap_or_else(|e| (1, e))
    } else {
        (
            2,
            trf(
                "kfx_editor.kfx_editor_cli_exe_is_missing_from_dir",
                "kfx-editor-cli.exe is missing from {dir}: the command line needs it. Reinstall \
                the editor or download the release archive again.",
                &[("dir", &exe.parent().unwrap_or(Path::new(".")).display())],
            ),
        )
    };
    let log = crate::settings::config_dir().map(|d| d.join("cli-output.log"));
    let written = log.as_ref().is_some_and(|p| {
        p.parent()
            .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
            && std::fs::write(p, log_text(args, code, &output)).is_ok()
    });
    let log_note = match (&log, written) {
        (Some(p), true) => trf(
            "kfx_editor.the_output_is_also_in_path",
            "The output is also in {path}.",
            &[("path", &p.display())],
        ),
        _ => String::new(),
    };
    show_window(args, code, &output, &log_note);
    code
}

/// A small window with the output (the GUI executable has nowhere else to show it).
fn show_window(args: &[String], code: i32, output: &str, log_note: &str) {
    let head = trf(
        "kfx_editor.kfx_editor_exe_is_the_windowed_program_and_cannot",
        "kfx-editor.exe is the windowed program and cannot print to a terminal. The command \
        \"{cmd}\" ran through kfx-editor-cli.exe (exit code {code}). Use kfx-editor-cli.exe for \
        the command line.",
        &[("cmd", &args.join(" ")), ("code", &code)],
    );
    let tail: Vec<&str> = output.lines().rev().take(400).collect();
    let body = kfx_workshop::ui::safe_text(&tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    let (head, log_note) = (head.to_string(), log_note.to_string());
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("KeeperFX Map Editor")
            .with_icon(crate::window_icon())
            .with_inner_size([720.0, 420.0]),
        ..Default::default()
    };
    let _ = eframe::run_ui_native("kfx-editor-cli-output", options, move |ui, _| {
        eframe::egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.label(&head);
            if !log_note.is_empty() {
                ui.label(&log_note);
            }
            ui.separator();
            eframe::egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 36.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.monospace(&body);
                });
            if ui.button(trd("kfx_editor.close", "Close")).clicked() {
                ui.ctx()
                    .send_viewport_cmd(eframe::egui::ViewportCommand::Close);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn which_arguments_need_the_console() {
        assert!(!wants_console(&[]));
        for c in [
            &["--help"][..],
            &["--version"],
            &["update", "--check"],
            &["validate", "x.slb"],
            &["--screenshot", "o.png"],
            &["no/such/map00001.slb"],
            &["valdate"],
        ] {
            assert!(wants_console(&a(c)), "{c:?}");
        }
        // an existing map or folder opens the window
        let d = std::env::temp_dir();
        assert!(!wants_console(&a(&[d.to_str().unwrap()])));
        assert!(cli_exe_beside(Path::new("/x/kfx-editor"))
            .to_string_lossy()
            .contains("kfx-editor-cli"));
    }

    #[cfg(unix)]
    #[test]
    fn output_and_exit_code_are_captured() {
        let (code, out) = capture(
            Path::new("sh"),
            &a(&["-c", "echo to-stdout; echo to-stderr >&2; exit 3"]),
        )
        .unwrap();
        assert_eq!(code, 3);
        assert!(
            out.contains("to-stdout\n") && out.contains("to-stderr"),
            "{out}"
        );
        let log = log_text(&a(&["validate", "m"]), 3, &out);
        assert!(log.contains("validate m") && log.contains("exit code 3"));
        assert!(capture(Path::new("/no/such/program"), &[]).is_err());
    }
}
