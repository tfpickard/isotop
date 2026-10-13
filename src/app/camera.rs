#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

    #[test]
    fn camera_follows_growing_views_until_moved_by_hand() {
        let mut app = App::new(view("cells"), model::demo(1.0, 64), all_available());
        app.render(320, 180, 512);
        assert!(app.framed && app.scene.bounded());
        app.key(KeyCode::Left, KeyModifiers::NONE, 10.0);
        assert!(!app.framed, "panning takes over the camera");
        app.key(KeyCode::Char('8'), KeyModifiers::NONE, 10.0);
        assert!(
            app.framed && app.view == view("globe"),
            "switching view frames it again"
        );
    }

    #[test]
    fn a_turned_globe_holds_still_until_reset_and_then_spins_on_without_a_jump() {
        let mut app = globe_at(10.0);
        // Holding the earth freezes it where it is, however far the clock has run.
        let free = home_on_screen(&mut app);
        app.both(|c| c.hold(10.0));
        let frozen = home_on_screen(&mut app);
        assert!(
            (free[0] - frozen[0]).abs() < 0.01 && (free[1] - frozen[1]).abs() < 0.01,
            "holding jumped {free:?} to {frozen:?}"
        );
        app.both(|c| c.release(10.0));
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        scroll(&mut app, MouseEventKind::ScrollDown, 2);
        let held = home_on_screen(&mut app);
        for time in [40.0, 150.0] {
            app.animation = time;
            let later = home_on_screen(&mut app);
            assert!(
                (held[0] - later[0]).abs() < 0.01 && (held[1] - later[1]).abs() < 0.01,
                "the held earth moved by {held:?} to {later:?} at {time} s"
            );
        }
        // r renders straight after the key, before the camera has eased anywhere.
        app.animation = 150.0;
        app.key(KeyCode::Char('r'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.globe_held && !app.goal.globe_held);
        let straight_after = home_on_screen(&mut app);
        assert!(
            (held[0] - straight_after[0]).abs() < 0.01
                && (held[1] - straight_after[1]).abs() < 0.01,
            "r made the earth jump from {held:?} to {straight_after:?}"
        );
        // Then the camera eases back to the default while the earth turns on.
        let mut last = straight_after;
        for frame in 1..=120 {
            let elapsed = 1.0 / 20.0;
            app.animation += elapsed;
            let goal = app.goal.clone();
            app.camera.approach(&goal, 1.0 - (-elapsed / 0.2_f32).exp());
            let now = home_on_screen(&mut app);
            let jump = (now[0] - last[0]).hypot(now[1] - last[1]);
            assert!(jump < 100.0, "frame {frame} jumped {jump} px");
            last = now;
        }
        assert!(app.camera.globe_tilt.abs() < 1e-3);
        assert!(
            (app.camera.globe_turn.rem_euclid(TAU) - app.goal.globe_turn).abs() < 1e-3
                || (app.camera.globe_turn.rem_euclid(TAU) - TAU).abs() < 1e-3
        );
        // It is the globe a new session shows at that time: nothing doubled the spin.
        let mut fresh = globe_at(app.animation);
        let (a, b) = (home_on_screen(&mut app), home_on_screen(&mut fresh));
        assert!(
            (a[0] - b[0]).abs() < 2.0 && (a[1] - b[1]).abs() < 2.0,
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn holding_and_releasing_while_paused_or_rewound_does_not_jump() {
        let mut app = globe_at(0.0);
        app.history.push_back(model::demo(50.0, 64));
        app.history.push_back(model::demo(100.0, 64));
        app.animation = 500.0;
        // [ pauses and steps back to the sample at 50 s, whatever the animation clock says.
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert!(app.paused);
        assert_eq!(app.render_time(), 50.0);
        let free = home_on_screen(&mut app);
        let time = app.render_time();
        app.both(|c| c.hold(time));
        let held = home_on_screen(&mut app);
        assert!(
            (free[0] - held[0]).abs() < 0.01 && (free[1] - held[1]).abs() < 0.01,
            "holding jumped {free:?} to {held:?}"
        );
        scroll(&mut app, MouseEventKind::ScrollRight, 2);
        let turned = home_on_screen(&mut app);
        assert!(turned[0] < held[0], "the scroll turned it");
        // g releases it, and the earth stays where the hand left it.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        assert!(!app.camera.globe_held);
        let released = home_on_screen(&mut app);
        assert!(
            (turned[0] - released[0]).abs() < 0.01 && (turned[1] - released[1]).abs() < 0.01,
            "releasing jumped {turned:?} to {released:?}"
        );
        // Rewinding further carries the free earth with the sample time, not the clock.
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        app.key(KeyCode::Char('['), KeyModifiers::NONE, 10.0);
        assert_eq!(app.render_time(), 1.0);
    }
}
