use super::App;

impl App {
    pub(super) fn search(&mut self) {
        let query = self.search.as_deref().unwrap_or("").to_lowercase();
        self.matches = self
            .snapshot()
            .processes
            .iter()
            .filter(|p| {
                p.name.to_lowercase().contains(&query)
                    || p.command.to_lowercase().contains(&query)
                    || p.id.pid.to_string() == query
            })
            .map(|p| p.id)
            .collect();
        self.match_index = 0;
        if !query.is_empty() {
            self.select_match();
        }
    }

    pub(super) fn select_match(&mut self) {
        if let Some(&id) = self.matches.get(self.match_index) {
            self.selected = Some(id);
            self.thing = None;
            if let Some(&p) = self.scene.positions.get(&id) {
                self.goal.center = [p[0], p[1]];
                self.face(p);
            } else {
                self.focus = Some(id);
                self.fit = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

    #[test]
    fn searching_keeps_the_hold_but_stands_the_earth_upright_on_the_match() {
        let mut app = globe_at(30.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 6);
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        app.render(640, 360, 512);
        let target = *app.scene.positions.keys().next().unwrap();
        app.matches = vec![target];
        app.select_match();
        assert!(app.goal.globe_held, "the found process must not drift away");
        assert_eq!(app.goal.globe_tilt, 0.0);
        let goal = app.goal.clone();
        app.camera.approach(&goal, 1.0);
        app.camera = app.goal.clone();
        let frame = app.render(640, 360, 512);
        let at = frame.project(&app.camera, app.scene.positions[&target]);
        assert!((at[0] - 320.0).abs() < 1.5 && (at[1] - 360.0 * ORIGIN_Y).abs() < 1.5);
    }
}
