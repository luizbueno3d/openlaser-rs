//! Control method names and their read-only classification (SPEC §4.5).

pub const METHODS: &[&str] = &[
    "engine.execute",
    "engine.commands",
    "engine.schema",
    "jobs.list",
    "jobs.cancel",
    "app.open",
    "app.import",
    "app.save",
    "app.export",
    "doc.inspect",
    "doc.render",
    "doc.select",
    "doc.close",
    "session.list",
    "machine.list",
    "machine.status",
    "job.plan",
    "job.preview",
    "job.estimate",
    "job.simulate",
    "job.export",
    "gcode.export",
    "frame.request",
    "frame.requests",
    "job.submit",
    "job.status",
    "job.withdraw",
    "machine.stop",
    "ui.inspect",
    "ui.elements",
    "ui.screenshot",
    "ui.focus",
    "ui.resize",
    "ui.set",
    "ui.menu.list",
    "ui.menu.invoke",
    "ui.dialog.open",
    "ui.dialog.set",
    "ui.dialog.confirm",
    "ui.dialog.cancel",
    "ui.pointer",
    "ui.click",
    "ui.move",
    "ui.drag",
    "ui.key",
    "ui.type",
    "ui.dock.move",
    "ui.dock.float",
    "ui.dock.close",
    "ui.dock.open",
    "ui.dock.resize",
    "app.quit",
    "methods",
];

pub fn is_read_only(method: &str) -> bool {
    matches!(
        method,
        "engine.commands"
            | "engine.schema"
            | "jobs.list"
            | "doc.inspect"
            | "doc.render"
            | "session.list"
            | "machine.list"
            | "machine.status"
            | "job.plan"
            | "job.preview"
            | "job.estimate"
            | "job.simulate"
            | "frame.requests"
            | "job.status"
            | "ui.inspect"
            | "ui.elements"
            | "ui.screenshot"
            | "ui.menu.list"
            | "methods"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_methods_table_is_exact() {
        assert_eq!(
            METHODS,
            &[
                "engine.execute",
                "engine.commands",
                "engine.schema",
                "jobs.list",
                "jobs.cancel",
                "app.open",
                "app.import",
                "app.save",
                "app.export",
                "doc.inspect",
                "doc.render",
                "doc.select",
                "doc.close",
                "session.list",
                "machine.list",
                "machine.status",
                "job.plan",
                "job.preview",
                "job.estimate",
                "job.simulate",
                "job.export",
                "gcode.export",
                "frame.request",
                "frame.requests",
                "job.submit",
                "job.status",
                "job.withdraw",
                "machine.stop",
                "ui.inspect",
                "ui.elements",
                "ui.screenshot",
                "ui.focus",
                "ui.resize",
                "ui.set",
                "ui.menu.list",
                "ui.menu.invoke",
                "ui.dialog.open",
                "ui.dialog.set",
                "ui.dialog.confirm",
                "ui.dialog.cancel",
                "ui.pointer",
                "ui.click",
                "ui.move",
                "ui.drag",
                "ui.key",
                "ui.type",
                "ui.dock.move",
                "ui.dock.float",
                "ui.dock.close",
                "ui.dock.open",
                "ui.dock.resize",
                "app.quit",
                "methods",
            ]
        );
    }
}
