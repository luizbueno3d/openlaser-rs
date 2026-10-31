//! Native catalogue adapter. Quit is routed through craft-quit's close guard.
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::sync::mpsc;

pub fn install(ctx: &egui::Context, fallback: bool) -> Result<(Menu, mpsc::Receiver<String>), Box<dyn std::error::Error + Send + Sync>> {
    let menu = Menu::new();
    let app = Submenu::new("OpenLaser", true);
    let quit = MenuItem::with_id(
        craft_quit::QUIT_MENU_ID,
        openlaser_ui_egui::i18n::t("Quit OpenLaser", "Sair do OpenLaser"),
        true,
        Some(muda::accelerator::Accelerator::new(muda::accelerator::Modifiers::META, muda::accelerator::Code::KeyQ)),
    );
    app.append(&quit)?;
    menu.append(&app)?;
    if !fallback {
        for name in openlaser_ui_egui::menu_catalog::MENUS {
            let sub = Submenu::new(openlaser_ui_egui::i18n::menu(name), true);
            append_rows(&sub, &[*name])?;
            menu.append(&sub)?;
        }
    }
    let ctx = ctx.clone();
    let (tx, rx) = mpsc::channel();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        route_event(&ctx, &tx, event.id.as_ref());
    }));
    menu.init_for_nsapp();
    Ok((menu, rx))
}

fn append_rows(menu: &Submenu, path: &[&str]) -> muda::Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for row in openlaser_ui_egui::menu_catalog::CATALOG.iter().filter(|r| r.path.starts_with(path)) {
        if let Some(next) = row.path.get(path.len()) {
            if seen.insert(*next) {
                let sub = Submenu::new(openlaser_ui_egui::i18n::menu(next), true);
                let mut nested = path.to_vec();
                nested.push(next);
                append_rows(&sub, &nested)?;
                menu.append(&sub)?;
            }
        } else if openlaser_ui_egui::menu_catalog::is_separator(row) {
            menu.append(&PredefinedMenuItem::separator())?;
        } else {
            menu.append(&MenuItem::with_id(
                openlaser_ui_egui::menu_catalog::native_id(row),
                openlaser_ui_egui::i18n::menu(row.label),
                openlaser_ui_egui::menu_catalog::is_live(row),
                None,
            ))?;
        }
    }
    Ok(())
}

fn route_event(ctx: &egui::Context, tx: &mpsc::Sender<String>, id: &str) {
    if !craft_quit::on_menu_event(ctx, id) {
        let _ = tx.send(id.into());
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_quit_requests_close_instead_of_terminating() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        route_event(&ctx, &tx, craft_quit::QUIT_MENU_ID);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::Close));
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn native_catalog_actions_reach_the_gui_dispatcher() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        route_event(&ctx, &tx, "edit.copy");
        assert_eq!(rx.try_recv().unwrap(), "edit.copy");
    }
}
