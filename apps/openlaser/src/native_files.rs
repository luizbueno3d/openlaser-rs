//! Native file dialogs that never block the UI thread (SPEC §3.19 `FileDialogs`, UI §7.2).
//!
//! `rfd::AsyncFileDialog` with `set_parent`: on macOS the dialog future is created on the UI thread, where rfd begins
//! a window-modal *sheet* (`beginSheetModalForWindow:`) and returns at once; the synchronous `FileDialog` would
//! `runModal` on the main thread and freeze the window, Esc and STOP included. A worker thread then waits on the
//! future with [`crate::block_on`] and reads the chosen file, and `poll` collects the result without waiting.
use openlaser_ui_egui::{DialogKind, DialogTicket, FileDialogs};
use std::collections::HashMap;
use std::future::Future;
use std::sync::mpsc;

type PickResult = Option<(String, Vec<u8>)>;

pub struct NativeFiles {
    dialog: rfd::AsyncFileDialog,
    ctx: egui::Context,
    next: u64,
    pending: HashMap<u64, mpsc::Receiver<PickResult>>,
}

impl NativeFiles {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self { dialog: rfd::AsyncFileDialog::new().set_parent(cc), ctx: cc.egui_ctx.clone(), next: 1, pending: HashMap::new() }
    }

    /// Waits for `work` on a worker thread; the UI thread only ever `try_recv`s the result.
    fn queue(&mut self, work: impl Future<Output = PickResult> + Send + 'static) -> DialogTicket {
        let ticket = self.next;
        self.next = self.next.saturating_add(1).max(1);
        let (tx, rx) = mpsc::channel();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new().name("openlaser-file-dialog".to_owned()).spawn(move || {
            let _ = tx.send(crate::block_on(work));
            ctx.request_repaint();
        });
        // If no worker could start, the dropped sender makes `poll` report a cancelled dialog.
        let _ = spawned;
        self.pending.insert(ticket, rx);
        DialogTicket(ticket)
    }
}

fn filtered(dialog: rfd::AsyncFileDialog, kind: DialogKind) -> rfd::AsyncFileDialog {
    match kind {
        DialogKind::OpenProject => dialog.add_filter("OpenLaser Project", &["olp"]),
        DialogKind::Import => dialog.add_filter("Vector and image files", &["svg", "dxf", "png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp", "gif", "json"]),
        DialogKind::SaveProject => dialog.add_filter("OpenLaser Project", &["olp"]),
        DialogKind::ExportGcode => dialog.add_filter("G-code", &["gcode", "nc"]),
        DialogKind::ExportJob => dialog.add_filter("Job review (JSON)", &["json"]),
        DialogKind::ExportSvg => dialog.add_filter("SVG", &["svg"]),
    }
}

impl FileDialogs for NativeFiles {
    fn open(&mut self, kind: DialogKind) -> DialogTicket {
        // Created here, on the UI thread: rfd starts the parented sheet and returns without blocking.
        let pick = filtered(self.dialog.clone(), kind).pick_file();
        self.queue(async move {
            let file = pick.await?;
            let bytes = file.read().await;
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

    fn save(&mut self, kind: DialogKind, bytes: Vec<u8>) -> DialogTicket {
        let save = filtered(self.dialog.clone(), kind).save_file();
        self.queue(async move {
            let file = save.await?;
            file.write(&bytes).await.ok()?;
            Some((file.path().to_string_lossy().into_owned(), Vec::new()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A dialog future that takes a while and records the thread that waits on it.
    struct SlowPick {
        ready_at: Instant,
        ui: std::thread::ThreadId,
    }
    impl Future for SlowPick {
        type Output = PickResult;
        fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<PickResult> {
            assert_ne!(std::thread::current().id(), self.ui, "the dialog future was awaited on the UI thread");
            if Instant::now() >= self.ready_at {
                std::task::Poll::Ready(Some(("selected.olp".into(), vec![1, 2, 3])))
            } else {
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }
    }

    #[test]
    fn dialog_waits_on_a_worker_and_the_ui_thread_only_polls() {
        let mut files = NativeFiles { dialog: rfd::AsyncFileDialog::new(), ctx: egui::Context::default(), next: 1, pending: HashMap::new() };
        let ui = std::thread::current().id();
        let started = Instant::now();
        let ticket = files.queue(SlowPick { ready_at: started + Duration::from_millis(150), ui });
        assert!(started.elapsed() < Duration::from_millis(100), "queueing a dialog blocked the UI thread");
        assert_eq!(files.poll(ticket), None, "a pending dialog answers None at once");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = files.poll(ticket) {
                assert_eq!(result, Some(("selected.olp".into(), vec![1, 2, 3])));
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(files.poll(ticket).is_none(), "a ticket is answered once");
        assert!(files.poll(DialogTicket(99)).is_none());
    }
}
