//! Parented asynchronous native sheets, completed on dialog worker threads.
use openlaser_ui_egui::{DialogKind, DialogTicket, FileDialogs};
use std::collections::HashMap;
use std::sync::mpsc;

type PickResult = Option<(String, Vec<u8>)>;
type DialogReceiver = mpsc::Receiver<PickResult>;

pub struct NativeFiles {
    dialog: rfd::AsyncFileDialog,
    ctx: egui::Context,
    next: u64,
    pending: HashMap<u64, DialogReceiver>,
}
impl NativeFiles {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self { dialog: rfd::AsyncFileDialog::new().set_parent(cc), ctx: cc.egui_ctx.clone(), next: 1, pending: HashMap::new() }
    }
    fn queue(&mut self, task: impl FnOnce() -> Option<(String, Vec<u8>)> + Send + 'static) -> DialogTicket {
        let ticket = self.next;
        self.next = self.next.saturating_add(1).max(1);
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        let _ = std::thread::Builder::new().name("openlaser-file-dialog".to_owned()).spawn(move || {
            let _ = tx.send(task());
            ctx.request_repaint();
        });
        self.pending.insert(ticket, rx);
        DialogTicket(ticket)
    }
}
impl FileDialogs for NativeFiles {
    fn open(&mut self, kind: DialogKind) -> DialogTicket {
        let mut dialog = self.dialog.clone();
        self.queue(move || {
            match kind {
                DialogKind::OpenProject => {
                    dialog = dialog.add_filter("OpenLaser Project", &["olp"]);
                }
                DialogKind::Import => {
                    dialog = dialog.add_filter("Vector and image files", &["svg", "dxf", "png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp", "gif"]);
                }
                _ => {}
            }
            let file = crate::block_on(dialog.pick_file())?;
            let bytes = crate::block_on(file.read());
            Some((file.path().to_string_lossy().into_owned(), bytes))
        })
    }
    fn poll(&mut self, ticket: DialogTicket) -> Option<Option<(String, Vec<u8>)>> {
        let rx = self.pending.get(&ticket.0)?;
        match rx.try_recv() {
            Ok(value) => {
                self.pending.remove(&ticket.0);
                Some(value)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending.remove(&ticket.0);
                Some(None)
            }
        }
    }
    fn save(&mut self, _kind: DialogKind, bytes: Vec<u8>) -> DialogTicket {
        let dialog = self.dialog.clone().add_filter("OpenLaser Project", &["olp"]);
        self.queue(move || {
            let file = crate::block_on(dialog.save_file())?;
            crate::block_on(file.write(&bytes)).ok()?;
            Some((file.path().to_string_lossy().into_owned(), Vec::new()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn file_dialog_work_runs_off_the_ui_thread() {
        let mut files = NativeFiles { dialog: rfd::AsyncFileDialog::new(), ctx: egui::Context::default(), next: 1, pending: HashMap::new() };
        let ui = std::thread::current().id();
        let ticket = files.queue(move || {
            assert_ne!(std::thread::current().id(), ui);
            Some(("selected.olp".into(), vec![1, 2, 3]))
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = files.poll(ticket) {
                assert_eq!(result, Some(("selected.olp".into(), vec![1, 2, 3])));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(files.poll(ticket).is_none());
    }
}
