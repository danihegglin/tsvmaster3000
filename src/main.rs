mod editor;
mod lineedit;
mod table;
mod vim;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};

use editor::Editor;
use table::Table;

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from);
    let (table, message) = match &path {
        Some(p) if p.exists() => match Table::load(p) {
            Ok((table, stats)) => {
                let msg = vim::load_message(p, &table, &stats);
                (table, Some(msg))
            }
            Err(e) => {
                eprintln!("tsv: {}: {e}", p.display());
                std::process::exit(1);
            }
        },
        Some(_) => (Table::default(), Some("[New]".to_owned())),
        None => (Table::default(), None),
    };

    Application::new().run(move |cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1280.0), px(820.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("tsv".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let editor = cx.new(|cx| Editor::new(table, path, message, cx));
                window.focus(&editor.read(cx).focus);
                editor
            },
        )
        .expect("failed to open window");
        cx.activate(true);
    });
}
