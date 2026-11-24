//! Settings of the window: the NAS folder file and the presence of the Google token.
//! The test sets process-wide environment variables, so it lives in its own test process.
use std::fs;

use ladon_ui_prototype::app::{self, Command, FailKind, Reply, State};

#[tokio::test]
async fn it_659_settings_roundtrip() {
    let settings = tempfile::tempdir().unwrap();
    let nas = tempfile::tempdir().unwrap();
    std::env::set_var("LADON_SETTINGS_DIR", settings.path());
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
    let mut state = State::new();

    match app::handle(&mut state, Command::LoadSettings).await {
        Reply::Settings { nas_folder, google_token_set } => {
            assert!(nas_folder.is_none());
            assert!(!google_token_set);
        }
        other => panic!("expected Settings, got {other:?}"),
    }

    match app::handle(&mut state, Command::SetNasFolder { path: "nas".into() }).await {
        Reply::Failed { kind, message, .. } => {
            assert_eq!(kind, FailKind::InvalidPath);
            assert_eq!(message, app::MSG_NAS_FOLDER_INVALID);
        }
        other => panic!("expected Failed, got {other:?}"),
    }

    let folder = nas.path().canonicalize().unwrap().to_string_lossy().into_owned();
    match app::handle(&mut state, Command::SetNasFolder { path: folder.clone() }).await {
        Reply::Settings { nas_folder, .. } => assert_eq!(nas_folder.as_deref(), Some(folder.as_str())),
        other => panic!("expected Settings, got {other:?}"),
    }
    let stored = fs::read_to_string(settings.path().join("nas-folder.txt")).unwrap();
    assert_eq!(stored.lines().next(), Some(folder.as_str()));

    match app::handle(&mut state, Command::LoadSettings).await {
        Reply::Settings { nas_folder, .. } => assert_eq!(nas_folder.as_deref(), Some(folder.as_str())),
        other => panic!("expected Settings, got {other:?}"),
    }

    std::env::set_var("LADON_GOOGLE_ACCESS_TOKEN", "token-value-for-test");
    let reply = app::handle(&mut state, Command::LoadSettings).await;
    assert!(matches!(reply, Reply::Settings { google_token_set: true, .. }));
    assert!(!format!("{reply:?}").contains("token-value-for-test"), "the token value must not appear in a reply");
    std::env::remove_var("LADON_GOOGLE_ACCESS_TOKEN");
}
