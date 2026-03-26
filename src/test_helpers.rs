use zellij_tile::prelude::*;

pub fn make_tab(position: usize, name: &str) -> TabInfo {
    TabInfo {
        position,
        name: name.to_string(),
        active: position == 0,
        ..Default::default()
    }
}

pub fn make_tab_active(position: usize, name: &str, active: bool) -> TabInfo {
    TabInfo {
        position,
        name: name.to_string(),
        active,
        ..Default::default()
    }
}

pub fn make_pane(id: u32, is_plugin: bool) -> PaneInfo {
    PaneInfo {
        id,
        is_plugin,
        title: format!("pane-{id}"),
        ..Default::default()
    }
}
