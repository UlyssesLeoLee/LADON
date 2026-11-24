//! Feasibility trial of the Slint testing backend in this repository.
//!
//! The trial builds a small Slint window, clicks an element through the testing backend and
//! checks that the callback ran. It needs the `gui` feature, because Slint is enabled by it.
#![cfg(feature = "gui")]

use i_slint_backend_testing::ElementHandle;
use slint::platform::PointerEventButton;

#[test]
fn trial_click_reaches_callback() {
    i_slint_backend_testing::init_integration_test_with_system_time();

    slint::spawn_local(async move {
        slint::slint! {
            #[edition = "1.19"]
            export component TrialApp inherits Window {
                out property <int> clicks: 0;
                ta := TouchArea {
                    clicked => { root.clicks += 1; }
                }
            }
        }

        let app = TrialApp::new().unwrap();
        let mut found = ElementHandle::find_by_element_id(&app, "TrialApp::ta");
        let elem = found.next().expect("the touch area is found by its id");
        elem.single_click(PointerEventButton::Left).await;
        assert_eq!(app.get_clicks(), 1);

        slint::quit_event_loop().unwrap();
    })
    .unwrap();

    slint::run_event_loop().unwrap();
}
