use std::collections::HashMap;
use zellij_tile::prelude::*;

/// Build a mapping from terminal pane_id -> (tab_index, tab_name).
/// Uses PaneManifest (keyed by tab_index) cross-referenced with TabInfo list.
pub fn build_pane_to_tab_map(
    tabs: &[TabInfo],
    manifest: &PaneManifest,
) -> HashMap<u32, (usize, String)> {
    let tab_name_by_position: HashMap<usize, String> = tabs
        .iter()
        .map(|t| (t.position, t.name.clone()))
        .collect();

    let mut map = HashMap::new();
    for (&tab_index, panes) in &manifest.panes {
        let tab_name = tab_name_by_position
            .get(&tab_index)
            .cloned()
            .unwrap_or_default();
        for pane in panes {
            if !pane.is_plugin {
                map.insert(pane.id, (tab_index, tab_name.clone()));
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{make_pane, make_tab};
    use std::collections::HashMap;

    #[test]
    fn empty_tabs_and_manifest() {
        let tabs = vec![];
        let manifest = PaneManifest {
            panes: HashMap::new(),
        };
        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert!(map.is_empty());
    }

    #[test]
    fn single_tab_single_pane() {
        let tabs = vec![make_tab(0, "main")];
        let mut panes = HashMap::new();
        panes.insert(0, vec![make_pane(10, false)]);
        let manifest = PaneManifest { panes };

        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&10], (0, "main".to_string()));
    }

    #[test]
    fn plugin_panes_filtered_out() {
        let tabs = vec![make_tab(0, "main")];
        let mut panes = HashMap::new();
        panes.insert(
            0,
            vec![
                make_pane(10, false),
                make_pane(11, true), // plugin pane
            ],
        );
        let manifest = PaneManifest { panes };

        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert_eq!(map.len(), 1);
        assert!(map.contains_key(&10));
        assert!(!map.contains_key(&11));
    }

    #[test]
    fn multiple_tabs_multiple_panes() {
        let tabs = vec![make_tab(0, "dev"), make_tab(1, "test"), make_tab(2, "docs")];
        let mut panes = HashMap::new();
        panes.insert(0, vec![make_pane(10, false), make_pane(11, false)]);
        panes.insert(1, vec![make_pane(20, false)]);
        panes.insert(2, vec![make_pane(30, false), make_pane(31, true)]);
        let manifest = PaneManifest { panes };

        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert_eq!(map.len(), 4); // 2 + 1 + 1 (plugin filtered)
        assert_eq!(map[&10], (0, "dev".to_string()));
        assert_eq!(map[&11], (0, "dev".to_string()));
        assert_eq!(map[&20], (1, "test".to_string()));
        assert_eq!(map[&30], (2, "docs".to_string()));
    }

    #[test]
    fn tab_with_no_panes_in_manifest() {
        let tabs = vec![make_tab(0, "empty"), make_tab(1, "full")];
        let mut panes = HashMap::new();
        // Tab 0 has no panes in manifest
        panes.insert(1, vec![make_pane(20, false)]);
        let manifest = PaneManifest { panes };

        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert_eq!(map.len(), 1);
        assert_eq!(map[&20], (1, "full".to_string()));
    }

    #[test]
    fn manifest_tab_without_matching_tab_info() {
        let tabs = vec![make_tab(0, "main")];
        let mut panes = HashMap::new();
        panes.insert(0, vec![make_pane(10, false)]);
        panes.insert(5, vec![make_pane(50, false)]); // tab 5 not in tabs
        let manifest = PaneManifest { panes };

        let map = build_pane_to_tab_map(&tabs, &manifest);
        assert_eq!(map.len(), 2);
        assert_eq!(map[&10], (0, "main".to_string()));
        // Tab 5 not in tabs → empty string name
        assert_eq!(map[&50], (5, String::new()));
    }
}
