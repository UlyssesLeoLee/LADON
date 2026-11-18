//! Optional Slint GUI proof-of-concept, not the full planned editor/Agent UI.
//! Window renders immediately; directory and history reads happen on workers.
slint::slint! {
    import { Button, LineEdit, ScrollView } from "std-widgets.slint";

    export component ExplorerWindow inherits Window {
        title: "LADON Explorer — Native Preview";
        width: 860px;
        height: 580px;
        in-out property <string> directory: ".";
        in-out property <string> listing: "Press Refresh to list files.";
        in-out property <string> history_text: "Select a tracked file name to browse history in the CLI.";
        in-out property <string> status_text: "Ready — no AI needed to browse";
        callback refresh(string);
        callback create_ignore(string);
        VerticalLayout {
            spacing: 8px;
            padding: 12px;
            HorizontalLayout {
                spacing: 8px;
                Text { text: "Folder"; vertical-alignment: center; }
                path_input := LineEdit { text <=> root.directory; }
                Button { text: "Refresh"; clicked => { root.refresh(path_input.text); } }
                Button { text: ".LADONignore"; clicked => { root.create_ignore(path_input.text); } }
            }
            Text { text: "Files (background enumerated)"; font-weight: 600; }
            ScrollView {
                Text { text: root.listing; wrap: word-wrap; }
            }
            Text { text: "History & task functions are provided in the Rust CLI kernel; full editor is a design-phase feature."; wrap: word-wrap; }
            Text { text: root.status_text; color: #64748b; }
        }
    }
}

fn main()->Result<(),Box<dyn std::error::Error>>{
    let app=ExplorerWindow::new()?;
    let weak=app.as_weak();
    app.on_refresh(move |path|{
        let weak=weak.clone();
        std::thread::spawn(move||{
            let result=std::fs::read_dir(path.as_str()).map(|entries|{
                let mut names:Vec<String>=entries.filter_map(Result::ok)
                    .map(|x|x.file_name().to_string_lossy().to_string()).collect();
                names.sort();names.join("\n")
            }).unwrap_or_else(|e|format!("Cannot read folder: {e}"));
            let _=weak.upgrade_in_event_loop(move|window|{window.set_listing(result.into());});
        });
    });
    let weak=app.as_weak();
    app.on_create_ignore(move|path|{
        let weak=weak.clone();
        std::thread::spawn(move||{
            let target=std::path::Path::new(path.as_str()).join(".LADONignore");
            let result=if target.exists(){format!("Existing policy: {}",target.display())}
              else {match std::fs::write(&target,include_str!("../../config/examples/.LADONignore")){
                Ok(_)=>format!("Created: {}",target.display()),Err(e)=>format!("Write failed: {e}")}};
            let _=weak.upgrade_in_event_loop(move|w|{w.set_status_text(result.into());});
        });
    });
    app.run()?;
    Ok(())
}
